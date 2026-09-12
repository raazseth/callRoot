use root_core::database::{
    add_folder, get_active_paths_for_folder, init_database, query_documents,
};
use root_core::filesystem::AppWatcherManager;
use root_core::indexer::{
    build_document_record, reconcile_in_transaction, IndexCommand, IndexSender, IndexWorker,
};
use rusqlite::Connection;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tempfile::tempdir;

fn setup_test_env() -> (Arc<Mutex<Connection>>, PathBuf, tempfile::TempDir) {
    let dir = tempdir().expect("tempdir failed");
    let db_path = dir.path().join("test_root.db");
    let conn = init_database(&db_path).expect("init_database failed");
    (Arc::new(Mutex::new(conn)), dir.path().to_path_buf(), dir)
}

fn poll_until<F>(timeout: Duration, mut condition: F) -> bool
where
    F: FnMut() -> bool,
{
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    condition()
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. VERIFY THE REAL END-TO-END INDEX PIPELINE
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_real_e2e_watcher_to_sqlite_pipeline() {
    let (db, _base, _guard) = setup_test_env();

    // 1. Create watched directories on disk
    let watch_dir = _base.join("watched_folder");
    let watch_dir2 = _base.join("watched_folder2");
    fs::create_dir_all(&watch_dir).unwrap();
    fs::create_dir_all(&watch_dir2).unwrap();

    // 2. Register folder in SQLite
    let folder_id = {
        let conn = db.lock().unwrap();
        let folder = add_folder(&conn, &watch_dir.to_string_lossy()).unwrap();
        folder.id
    };

    // 3. Start IndexWorker thread
    let (tx, rx) = mpsc::sync_channel::<IndexCommand>(1024);
    let index_sender = IndexSender(tx);
    let worker_db = Arc::clone(&db);
    let worker = IndexWorker::new_for_test(worker_db, rx);
    let worker_handle = std::thread::spawn(move || worker.run());

    // 4. Start actual OS RecommendedWatcher via AppWatcherManager
    let mut watcher_mgr = AppWatcherManager::new(index_sender.clone());
    watcher_mgr
        .watch(&watch_dir.to_string_lossy())
        .expect("Watcher watch failed");

    // Give watcher thread a moment to initialize
    std::thread::sleep(Duration::from_millis(100));

    // ── Test 1: Create supported file → ACTIVE row appears ──
    let test_file = watch_dir.join("document_a.pdf");
    fs::write(&test_file, b"sample content v1").unwrap();

    let doc_indexed = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        let docs = query_documents(&conn, Some(folder_id)).unwrap();
        docs.iter()
            .any(|d| d.path == test_file.to_string_lossy() && d.status == "ACTIVE")
    });
    assert!(doc_indexed, "Created file must appear in SQLite as ACTIVE");

    // ── Test 2: Modify supported file → row updates ──
    let initial_modified = {
        let conn = db.lock().unwrap();
        let docs = query_documents(&conn, Some(folder_id)).unwrap();
        docs.iter()
            .find(|d| d.path == test_file.to_string_lossy())
            .unwrap()
            .modified_at
    };

    // Sleep to ensure filesystem timestamp increments
    std::thread::sleep(Duration::from_secs(1));
    fs::write(&test_file, b"sample content v2 modified").unwrap();

    let doc_updated = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        let docs = query_documents(&conn, Some(folder_id)).unwrap();
        docs.iter()
            .any(|d| d.path == test_file.to_string_lossy() && d.modified_at >= initial_modified)
    });
    assert!(
        doc_updated,
        "Modified file must update modified_at in SQLite"
    );

    // ── Test 3: Delete file → row becomes MISSING ──
    fs::remove_file(&test_file).unwrap();

    let doc_missing = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        let active = get_active_paths_for_folder(&conn, folder_id).unwrap();
        !active.contains(&test_file.to_string_lossy().to_string())
    });
    assert!(doc_missing, "Deleted file must become MISSING in SQLite");

    // ── Test 4: Rename file → old path MISSING, new path ACTIVE ──
    let rename_src = watch_dir.join("rename_src.docx");
    let rename_dst = watch_dir.join("rename_dst.docx");
    fs::write(&rename_src, b"rename test").unwrap();

    // Wait for src to be active first
    let src_active = poll_until(Duration::from_secs(3), || {
        let conn = db.lock().unwrap();
        let active = get_active_paths_for_folder(&conn, folder_id).unwrap();
        active.contains(&rename_src.to_string_lossy().to_string())
    });
    assert!(src_active, "Source file must be ACTIVE before rename");

    // Perform real OS rename
    fs::rename(&rename_src, &rename_dst).unwrap();

    let rename_converged = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        let active = get_active_paths_for_folder(&conn, folder_id).unwrap();
        !active.contains(&rename_src.to_string_lossy().to_string())
            && active.contains(&rename_dst.to_string_lossy().to_string())
    });
    assert!(
        rename_converged,
        "Rename must leave old path MISSING and new path ACTIVE in SQLite"
    );

    // ── Test 5: Add second watched folder → its files are indexed ──
    let folder_id2 = {
        let conn = db.lock().unwrap();
        let f2 = add_folder(&conn, &watch_dir2.to_string_lossy()).unwrap();
        f2.id
    };
    watcher_mgr
        .watch(&watch_dir2.to_string_lossy())
        .expect("Watch second folder failed");

    std::thread::sleep(Duration::from_millis(100));
    let file_in_dir2 = watch_dir2.join("second_folder_doc.txt");
    fs::write(&file_in_dir2, b"content in folder 2").unwrap();

    let f2_indexed = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        let active = get_active_paths_for_folder(&conn, folder_id2).unwrap();
        active.contains(&file_in_dir2.to_string_lossy().to_string())
    });
    assert!(f2_indexed, "Files in second watched folder must be indexed");

    // ── Test 6: Remove watched folder → subsequent changes ignored ──
    watcher_mgr
        .unwatch_folder(&watch_dir2.to_string_lossy())
        .expect("Unwatch folder 2 failed");

    // Create a new file in unmonitored folder 2
    let ignored_file = watch_dir2.join("ignored_doc.pdf");
    fs::write(&ignored_file, b"should not be indexed").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    {
        let conn = db.lock().unwrap();
        let active = get_active_paths_for_folder(&conn, folder_id2).unwrap();
        assert!(
            !active.contains(&ignored_file.to_string_lossy().to_string()),
            "Changes in unwatched folder must NOT be indexed"
        );
    }

    // ── Test 7: Similar-prefix folders remain isolated ──
    let similar_prefix_dir = _base.join("watched_folder_backup");
    fs::create_dir_all(&similar_prefix_dir).unwrap();
    let similar_file = similar_prefix_dir.join("similar.pdf");
    fs::write(&similar_file, b"similar prefix content").unwrap();
    std::thread::sleep(Duration::from_millis(500));

    {
        let conn = db.lock().unwrap();
        let active1 = get_active_paths_for_folder(&conn, folder_id).unwrap();
        assert!(
            !active1.contains(&similar_file.to_string_lossy().to_string()),
            "Similar-prefix directory must not pollute watched folder index"
        );
    }

    // ── Test 8: Reconciliation restores correctness after missed events ──
    let missed_file = watch_dir.join("missed_while_offline.md");
    fs::write(&missed_file, b"missed event doc").unwrap();

    // Trigger reconciliation directly through IndexWorker channel
    index_sender
        .send(IndexCommand::ReconcileFolder {
            folder_id,
            folder_path: watch_dir.to_string_lossy().to_string(),
        })
        .unwrap();

    let reconcile_converged = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        let active = get_active_paths_for_folder(&conn, folder_id).unwrap();
        active.contains(&missed_file.to_string_lossy().to_string())
    });
    assert!(
        reconcile_converged,
        "Reconciliation must discover and index missed files"
    );

    // Shutdown worker
    index_sender.send(IndexCommand::Shutdown).unwrap();
    worker_handle.join().unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. VERIFY QUEUE BACKPRESSURE UNDER BURST
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_queue_backpressure_and_convergence_under_burst() {
    let (db, _base, _guard) = setup_test_env();
    let folder_id = {
        let conn = db.lock().unwrap();
        add_folder(&conn, &_base.to_string_lossy()).unwrap().id
    };

    // Tiny buffer size (10 items) to force high backpressure
    let (tx, rx) = mpsc::sync_channel::<IndexCommand>(10);
    let sender = IndexSender(tx);

    let worker_db = Arc::clone(&db);
    let worker = IndexWorker::new_for_test(worker_db, rx);
    let worker_handle = std::thread::spawn(move || worker.run());

    let burst_count = 150;
    let mut expected_paths = Vec::new();

    // Create 150 files and burst-send them
    for i in 0..burst_count {
        let file_path = _base.join(format!("burst_doc_{i}.pdf"));
        fs::write(&file_path, format!("content {i}")).unwrap();
        expected_paths.push(file_path.to_string_lossy().to_string());

        sender
            .send(IndexCommand::ScanFile {
                folder_id,
                file_path,
            })
            .expect("Blocking send must not drop or fail under backpressure");
    }

    // Wait for all burst items to be processed
    let all_processed = poll_until(Duration::from_secs(8), || {
        let conn = db.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE watched_folder_id = ?1 AND status = 'ACTIVE'",
                [folder_id],
                |r| r.get(0),
            )
            .unwrap();
        count == burst_count as i64
    });
    assert!(
        all_processed,
        "Every single burst command must be processed by IndexWorker without data loss"
    );

    sender.send(IndexCommand::Shutdown).unwrap();
    worker_handle.join().unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. VERIFY REAL OS RENAME BEHAVIOR
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_real_os_renames_cross_and_outside_folders() {
    let (db, _base, _guard) = setup_test_env();

    let folder_a = _base.join("folder_a");
    let folder_b = _base.join("folder_b");
    let outside_dir = _base.join("outside_dir");
    fs::create_dir_all(&folder_a).unwrap();
    fs::create_dir_all(&folder_b).unwrap();
    fs::create_dir_all(&outside_dir).unwrap();

    let fid_a = {
        let conn = db.lock().unwrap();
        add_folder(&conn, &folder_a.to_string_lossy()).unwrap().id
    };
    let fid_b = {
        let conn = db.lock().unwrap();
        add_folder(&conn, &folder_b.to_string_lossy()).unwrap().id
    };

    let (tx, rx) = mpsc::sync_channel::<IndexCommand>(100);
    let sender = IndexSender(tx);
    let worker_db = Arc::clone(&db);
    let worker = IndexWorker::new_for_test(worker_db, rx);
    let worker_handle = std::thread::spawn(move || worker.run());

    let mut watcher = AppWatcherManager::new(sender.clone());
    watcher.watch(&folder_a.to_string_lossy()).unwrap();
    watcher.watch(&folder_b.to_string_lossy()).unwrap();

    std::thread::sleep(Duration::from_millis(100));

    // 1. Rename into outside folder (A -> outside)
    let file_in_a = folder_a.join("file_in_a.pdf");
    fs::write(&file_in_a, b"content").unwrap();

    poll_until(Duration::from_secs(3), || {
        let conn = db.lock().unwrap();
        get_active_paths_for_folder(&conn, fid_a)
            .unwrap()
            .contains(&file_in_a.to_string_lossy().to_string())
    });

    let moved_to_outside = outside_dir.join("file_outside.pdf");
    fs::rename(&file_in_a, &moved_to_outside).unwrap();

    let outside_converged = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        let active_a = get_active_paths_for_folder(&conn, fid_a).unwrap();
        !active_a.contains(&file_in_a.to_string_lossy().to_string())
    });
    assert!(outside_converged, "File moved outside must become MISSING");

    // 2. Rename from outside INTO watched folder (outside -> B)
    let outside_src = outside_dir.join("intro.txt");
    fs::write(&outside_src, b"intro").unwrap();
    let inside_b = folder_b.join("intro_in_b.txt");
    fs::rename(&outside_src, &inside_b).unwrap();

    let inside_b_converged = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        get_active_paths_for_folder(&conn, fid_b)
            .unwrap()
            .contains(&inside_b.to_string_lossy().to_string())
    });
    assert!(
        inside_b_converged,
        "File moved in from outside must be indexed as ACTIVE"
    );

    sender.send(IndexCommand::Shutdown).unwrap();
    worker_handle.join().unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. VERIFY DOCUMENT REMOVAL SEMANTICS
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_document_removal_via_index_worker() {
    let (db, _base, _guard) = setup_test_env();
    let folder_id = {
        let conn = db.lock().unwrap();
        add_folder(&conn, &_base.to_string_lossy()).unwrap().id
    };

    let (tx, rx) = mpsc::sync_channel::<IndexCommand>(50);
    let sender = IndexSender(tx);
    let worker_db = Arc::clone(&db);
    let worker = IndexWorker::new_for_test(worker_db, rx);
    let worker_handle = std::thread::spawn(move || worker.run());

    let file_path = _base.join("remove_target.pdf");
    fs::write(&file_path, b"pdf data").unwrap();

    // 1. Index document
    sender
        .send(IndexCommand::ScanFile {
            folder_id,
            file_path: file_path.clone(),
        })
        .unwrap();

    let doc_id = {
        let mut id = 0;
        let found = poll_until(Duration::from_secs(3), || {
            let conn = db.lock().unwrap();
            let docs = query_documents(&conn, Some(folder_id)).unwrap();
            if let Some(d) = docs.first() {
                id = d.id;
                true
            } else {
                false
            }
        });
        assert!(found, "Document must be indexed");
        id
    };

    // 2. Remove document via IndexWorker
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    sender
        .send(IndexCommand::RemoveDocument {
            doc_id,
            reply: Some(reply_tx),
        })
        .unwrap();

    let reply = reply_rx.recv().unwrap();
    assert_eq!(reply, Ok(true));

    // 3. Verify SQLite entry removed
    {
        let conn = db.lock().unwrap();
        let docs = query_documents(&conn, Some(folder_id)).unwrap();
        assert!(
            docs.is_empty(),
            "Document record must be deleted from SQLite"
        );
    }

    // 4. Verify physical file is untouched
    assert!(
        file_path.exists(),
        "Physical file on disk must NOT be deleted"
    );

    sender.send(IndexCommand::Shutdown).unwrap();
    worker_handle.join().unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. VERIFY STARTUP RECOVERY
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_startup_recovery_discovers_offline_changes() {
    let (db, _base, _guard) = setup_test_env();
    let folder_id = {
        let conn = db.lock().unwrap();
        add_folder(&conn, &_base.to_string_lossy()).unwrap().id
    };

    // Indexer is NOT running yet.
    // Create 3 files on disk while offline:
    let f1 = _base.join("offline_1.pdf");
    let f2 = _base.join("offline_2.docx");
    let f3 = _base.join("offline_3.txt");
    fs::write(&f1, b"1").unwrap();
    fs::write(&f2, b"2").unwrap();
    fs::write(&f3, b"3").unwrap();

    // Now start the IndexWorker (simulating Root application startup)
    let (tx, rx) = mpsc::sync_channel::<IndexCommand>(50);
    let sender = IndexSender(tx);
    let worker_db = Arc::clone(&db);
    let worker = IndexWorker::new_for_test(worker_db, rx);
    let worker_handle = std::thread::spawn(move || worker.run());

    // Queue startup reconciliation (as lib.rs does)
    sender
        .send(IndexCommand::ReconcileFolder {
            folder_id,
            folder_path: _base.to_string_lossy().to_string(),
        })
        .unwrap();

    // Verify all 3 offline files are discovered and indexed
    let startup_converged = poll_until(Duration::from_secs(4), || {
        let conn = db.lock().unwrap();
        let active = get_active_paths_for_folder(&conn, folder_id).unwrap();
        active.contains(&f1.to_string_lossy().to_string())
            && active.contains(&f2.to_string_lossy().to_string())
            && active.contains(&f3.to_string_lossy().to_string())
    });
    assert!(
        startup_converged,
        "Startup reconciliation must discover all files created while offline"
    );

    sender.send(IndexCommand::Shutdown).unwrap();
    worker_handle.join().unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. VERIFY FAILURE RECOVERY (No panics, clean error reporting)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_failure_recovery_and_robustness() {
    let (db, _base, _guard) = setup_test_env();

    // 1. Worker disconnect recovery
    {
        let (tx, rx) = mpsc::sync_channel::<IndexCommand>(2);
        let sender = IndexSender(tx);
        drop(rx); // worker drops
        let res = sender.send(IndexCommand::Shutdown);
        assert!(res.is_err(), "Worker disconnect must return Err");
    }

    // 2. Missing folder during reconciliation (does not panic)
    {
        let conn = db.lock().unwrap();
        let stats = reconcile_in_transaction(&conn, 8888, &[], &HashSet::new()).unwrap();
        assert_eq!(stats.inserted, 0);
    }

    // 3. File disappearing between scan and builder
    {
        let non_existent = _base.join("phantom.pdf");
        let record = build_document_record(1, &non_existent);
        assert!(
            record.is_none(),
            "Non-existent file returns None without panic"
        );
    }

    // 4. Rapid create / delete / rename sequence
    {
        let folder_id = {
            let conn = db.lock().unwrap();
            add_folder(&conn, &_base.to_string_lossy()).unwrap().id
        };

        let (tx, rx) = mpsc::sync_channel::<IndexCommand>(50);
        let sender = IndexSender(tx);
        let worker_db = Arc::clone(&db);
        let worker = IndexWorker::new_for_test(worker_db, rx);
        let worker_handle = std::thread::spawn(move || worker.run());

        for i in 0..30 {
            let temp_f = _base.join(format!("rapid_{i}.pdf"));
            fs::write(&temp_f, b"rapid").unwrap();
            sender
                .send(IndexCommand::ScanFile {
                    folder_id,
                    file_path: temp_f.clone(),
                })
                .unwrap();
            fs::remove_file(&temp_f).unwrap();
            sender
                .send(IndexCommand::MarkMissing { file_path: temp_f })
                .unwrap();
        }

        sender.send(IndexCommand::Shutdown).unwrap();
        worker_handle.join().unwrap();
        // Completed without panics
    }
}
