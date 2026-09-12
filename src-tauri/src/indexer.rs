//! indexer.rs — The sole serialised write path to the documents table.
//!
//! Architecture:
//!   notify watcher ──┐
//!                    │  IndexCommand channel
//!   startup scan  ───┼────────────────────► IndexWorker thread ──► SQLite
//!                    │                              │
//!   rescan cmd ──────┘               app.emit("index-update")
//!
//! Only IndexWorker writes to the `documents` table.
//! All other code sends IndexCommand messages via IndexSender.

use crate::database::{self, current_timestamp, get_active_paths_for_folder, DocumentRecord};
use crate::filesystem::{is_dir_excluded, is_supported_doc, ScanProgress};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

// ─── Command enum ────────────────────────────────────────────────────────────

/// Every mutation to the documents table must go through this enum.
#[derive(Debug)]
pub enum IndexCommand {
    /// Upsert a single file discovered via watcher event.
    /// IndexWorker resolves the owning watched folder dynamically from the database.
    UpsertFile { file_path: PathBuf },
    /// Upsert a single file with a verified, genuinely known folder_id (used by scan).
    ScanFile { folder_id: i64, file_path: PathBuf },
    /// Mark a file MISSING (watcher DELETE / RENAME-FROM event).
    MarkMissing { file_path: PathBuf },
    /// Remove a document by doc_id (sole write path for document deletions).
    RemoveDocument {
        doc_id: i64,
        reply: Option<mpsc::SyncSender<Result<bool, String>>>,
    },
    /// Full reconciliation of a watched folder against the live filesystem.
    ReconcileFolder { folder_id: i64, folder_path: String },
    /// Graceful shutdown — worker thread exits cleanly.
    Shutdown,
}

// ─── Sender handle ───────────────────────────────────────────────────────────

/// Clone-able sender handle stored in AppState.
#[derive(Clone)]
pub struct IndexSender(pub mpsc::SyncSender<IndexCommand>);

impl IndexSender {
    /// Blocking send with backpressure. Never silently drops commands under queue pressure.
    /// Returns Err if the IndexWorker thread has disconnected / terminated.
    pub fn send(&self, cmd: IndexCommand) -> Result<(), String> {
        self.0.send(cmd).map_err(|e| {
            eprintln!("[indexer] failed to send command to worker (disconnected): {e}");
            format!("IndexWorker disconnected: {e}")
        })
    }
}

// ─── Worker ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ReconcileStats {
    pub inserted: u32,
    pub updated: u32,
    pub marked_missing: u32,
}

pub struct IndexWorker {
    db: Arc<Mutex<Connection>>,
    app_handle: Option<AppHandle>,
    rx: mpsc::Receiver<IndexCommand>,
}

impl IndexWorker {
    pub fn new(
        db: Arc<Mutex<Connection>>,
        app_handle: AppHandle,
        rx: mpsc::Receiver<IndexCommand>,
    ) -> Self {
        Self {
            db,
            app_handle: Some(app_handle),
            rx,
        }
    }

    pub fn new_for_test(db: Arc<Mutex<Connection>>, rx: mpsc::Receiver<IndexCommand>) -> Self {
        Self {
            db,
            app_handle: None,
            rx,
        }
    }

    pub fn run(self) {
        while let Ok(cmd) = self.rx.recv() {
            match cmd {
                IndexCommand::UpsertFile { file_path } => {
                    self.handle_upsert(&file_path);
                }
                IndexCommand::ScanFile {
                    folder_id,
                    file_path,
                } => {
                    self.handle_scan_file(folder_id, &file_path);
                }
                IndexCommand::MarkMissing { file_path } => {
                    self.handle_missing(&file_path);
                }
                IndexCommand::RemoveDocument { doc_id, reply } => {
                    self.handle_remove_document(doc_id, reply);
                }
                IndexCommand::ReconcileFolder {
                    folder_id,
                    folder_path,
                } => {
                    self.handle_reconcile(folder_id, &folder_path);
                }
                IndexCommand::Shutdown => {
                    eprintln!("[indexer] shutdown received");
                    break;
                }
            }
        }
    }

    /// Handles file discovered via watcher (folder_id dynamically resolved from DB).
    fn handle_upsert(&self, file_path: &Path) {
        if !file_path.exists() {
            eprintln!("[indexer] upsert skip — gone: {}", file_path.display());
            return;
        }
        if !is_supported_doc(file_path) {
            return;
        }

        let conn = match self.db.lock() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[indexer] DB lock poisoned: {e}");
                return;
            }
        };

        let resolved_id = match resolve_folder_id(&conn, file_path) {
            Some(id) => id,
            None => {
                eprintln!("[indexer] no owning folder for: {}", file_path.display());
                return;
            }
        };

        let doc = match build_document_record(resolved_id, file_path) {
            Some(d) => d,
            None => return,
        };

        match database::save_document(&conn, &doc) {
            Ok(_) => {
                eprintln!("[indexer] upserted: {}", file_path.display());
                drop(conn);
                self.emit_index_update();
            }
            Err(e) => eprintln!("[indexer] upsert error for {}: {e}", file_path.display()),
        }
    }

    /// Handles file discovered with genuinely known folder_id.
    fn handle_scan_file(&self, folder_id: i64, file_path: &Path) {
        if !file_path.exists() || !is_supported_doc(file_path) {
            return;
        }

        let conn = match self.db.lock() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[indexer] DB lock poisoned: {e}");
                return;
            }
        };

        let doc = match build_document_record(folder_id, file_path) {
            Some(d) => d,
            None => return,
        };

        match database::save_document(&conn, &doc) {
            Ok(_) => {
                eprintln!("[indexer] scanned: {}", file_path.display());
                drop(conn);
                self.emit_index_update();
            }
            Err(e) => eprintln!("[indexer] scan save error for {}: {e}", file_path.display()),
        }
    }

    fn handle_missing(&self, file_path: &Path) {
        let path_str = file_path.to_string_lossy();
        match self.db.lock() {
            Ok(conn) => match database::mark_document_status(&conn, &path_str, "MISSING") {
                Ok(_) => {
                    eprintln!("[indexer] marked MISSING: {path_str}");
                    drop(conn);
                    self.emit_index_update();
                }
                Err(e) => eprintln!("[indexer] mark-missing error for {path_str}: {e}"),
            },
            Err(e) => eprintln!("[indexer] DB lock poisoned: {e}"),
        }
    }

    fn handle_remove_document(
        &self,
        doc_id: i64,
        reply: Option<mpsc::SyncSender<Result<bool, String>>>,
    ) {
        let res = match self.db.lock() {
            Ok(conn) => match database::remove_document(&conn, doc_id) {
                Ok(_) => {
                    eprintln!("[indexer] removed document id={doc_id}");
                    self.emit_index_update();
                    Ok(true)
                }
                Err(e) => {
                    eprintln!("[indexer] remove document error id={doc_id}: {e}");
                    Err(format!("Database error removing document: {e}"))
                }
            },
            Err(e) => {
                eprintln!("[indexer] DB lock poisoned: {e}");
                Err(format!("Database lock poisoned: {e}"))
            }
        };

        if let Some(r) = reply {
            if let Err(e) = r.send(res) {
                eprintln!("[indexer] failed to send reply for remove_document: {e}");
            }
        }
    }

    fn handle_reconcile(&self, folder_id: i64, folder_path: &str) {
        eprintln!("[indexer] reconcile start: folder_id={folder_id} path={folder_path}");
        self.emit_scan_progress(0, &format!("Scanning {folder_path}…"), true);

        let root = Path::new(folder_path);
        if !root.exists() || !root.is_dir() {
            eprintln!("[indexer] reconcile skip — not accessible: {folder_path}");
            self.emit_scan_progress(0, "Folder not accessible", false);
            return;
        }

        let mut fs_docs: Vec<DocumentRecord> = Vec::new();
        let mut scanned: u64 = 0;

        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                if e.file_type().is_dir() {
                    if let Some(name) = e.file_name().to_str() {
                        return !is_dir_excluded(name);
                    }
                }
                true
            });

        for entry in walker.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() && is_supported_doc(path) {
                if let Some(doc) = build_document_record(folder_id, path) {
                    scanned += 1;
                    fs_docs.push(doc);
                    if scanned % 50 == 0 {
                        self.emit_scan_progress(scanned, &path.to_string_lossy(), true);
                    }
                }
            }
        }

        eprintln!("[indexer] reconcile scanned {scanned} files on disk");

        let fs_path_set: HashSet<String> = fs_docs.iter().map(|d| d.path.clone()).collect();

        let conn = match self.db.lock() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[indexer] reconcile DB lock error: {e}");
                self.emit_scan_progress(0, "Database error during reconcile", false);
                return;
            }
        };

        match reconcile_in_transaction(&conn, folder_id, &fs_docs, &fs_path_set) {
            Ok(stats) => {
                eprintln!(
                    "[indexer] reconcile done: inserted={} updated={} marked_missing={}",
                    stats.inserted, stats.updated, stats.marked_missing
                );
            }
            Err(e) => {
                eprintln!("[indexer] reconcile transaction failed: {e}");
                self.emit_scan_progress(0, "Database write error during reconcile", false);
                return;
            }
        }

        drop(conn);
        self.emit_scan_progress(scanned, "Done", false);
        self.emit_index_update();
    }

    fn emit_index_update(&self) {
        if let Some(ref handle) = self.app_handle {
            if let Err(e) = handle.emit("index-update", ()) {
                eprintln!("[indexer] emit index-update error: {e}");
            }
        }
    }

    fn emit_scan_progress(&self, processed: u64, current_file: &str, is_scanning: bool) {
        if let Some(ref handle) = self.app_handle {
            if let Err(e) = handle.emit(
                "scan-progress",
                ScanProgress {
                    processed,
                    total: 0, // indeterminate
                    current_file: current_file.to_string(),
                    is_scanning,
                },
            ) {
                eprintln!("[indexer] emit scan-progress error: {e}");
            }
        }
    }
}

// ─── Folder-ID resolution (for watcher events) ───────────────────────────────

/// Look up which watched folder owns `path` by querying the DB.
/// Finds the longest-prefix match among all watched_folders.
fn resolve_folder_id(conn: &Connection, path: &Path) -> Option<i64> {
    let mut stmt = conn.prepare("SELECT id, path FROM watched_folders").ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;

    let mut best: Option<(i64, usize)> = None;
    for row in rows.flatten() {
        let (id, folder_path) = row;
        let folder_pb = PathBuf::from(&folder_path);
        if is_path_under_folder(path, &folder_pb) {
            let depth = folder_pb.components().count();
            if best.map_or(true, |(_, d)| depth > d) {
                best = Some((id, depth));
            }
        }
    }
    best.map(|(id, _)| id)
}

// ─── Reconciliation transaction ───────────────────────────────────────────────

pub fn reconcile_in_transaction(
    conn: &Connection,
    folder_id: i64,
    fs_docs: &[DocumentRecord],
    fs_path_set: &HashSet<String>,
) -> rusqlite::Result<ReconcileStats> {
    let mut stats = ReconcileStats::default();

    // Check if the folder still exists in watched_folders (guards against race with folder removal)
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM watched_folders WHERE id = ?1)",
        rusqlite::params![folder_id],
        |row| row.get(0),
    )?;
    if !exists {
        eprintln!("[indexer] reconcile aborted: folder {folder_id} no longer exists in DB");
        return Ok(stats);
    }

    let db_paths = get_active_paths_for_folder(conn, folder_id)?;

    conn.execute_batch("BEGIN IMMEDIATE")?;

    for doc in fs_docs {
        database::save_document(conn, doc)?;
        if db_paths.contains(&doc.path) {
            stats.updated += 1;
        } else {
            stats.inserted += 1;
        }
    }

    for db_path in &db_paths {
        if !fs_path_set.contains(db_path) {
            database::mark_document_status(conn, db_path, "MISSING")?;
            stats.marked_missing += 1;
        }
    }

    conn.execute_batch("COMMIT")?;
    Ok(stats)
}

// ─── Document record builder ──────────────────────────────────────────────────

pub fn build_document_record(folder_id: i64, path: &Path) -> Option<DocumentRecord> {
    let metadata = fs::metadata(path).ok()?;
    let filename = path.file_name()?.to_str()?.to_string();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let size = metadata.len();

    let created_at = metadata
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_else(current_timestamp);

    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or_else(current_timestamp);

    Some(DocumentRecord {
        id: 0,
        watched_folder_id: folder_id,
        path: path.to_string_lossy().to_string(),
        filename,
        extension,
        size,
        created_at,
        modified_at,
        indexed_at: current_timestamp(),
        status: "ACTIVE".to_string(),
    })
}

// ─── Path containment helpers ─────────────────────────────────────────────────

/// Returns true if `path` is strictly inside `folder` (component-aware, not string prefix).
pub fn is_path_under_folder(path: &Path, folder: &Path) -> bool {
    path.starts_with(folder) && path != folder
}

/// Find the most-specific watched folder that owns `path`.
pub fn find_owning_folder<'a>(path: &Path, monitored: &'a HashSet<PathBuf>) -> Option<&'a PathBuf> {
    monitored
        .iter()
        .filter(|f| is_path_under_folder(path, f))
        .max_by_key(|f| f.components().count())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{get_active_paths_for_folder, save_document};
    use std::fs;
    use tempfile::tempdir;

    fn open_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory DB");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE watched_folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE documents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                watched_folder_id INTEGER NOT NULL,
                path TEXT NOT NULL UNIQUE,
                filename TEXT NOT NULL,
                extension TEXT NOT NULL,
                size INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                modified_at INTEGER NOT NULL,
                indexed_at INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'ACTIVE',
                FOREIGN KEY(watched_folder_id) REFERENCES watched_folders(id) ON DELETE CASCADE
            );",
        )
        .unwrap();
        conn
    }

    fn insert_folder(conn: &Connection, path: &str) -> i64 {
        conn.execute(
            "INSERT INTO watched_folders (path, created_at, updated_at) VALUES (?1, 0, 0)",
            [path],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn make_doc(folder_id: i64, path: &str, filename: &str, ext: &str) -> DocumentRecord {
        DocumentRecord {
            id: 0,
            watched_folder_id: folder_id,
            path: path.to_string(),
            filename: filename.to_string(),
            extension: ext.to_string(),
            size: 1024,
            created_at: 0,
            modified_at: 0,
            indexed_at: 0,
            status: "ACTIVE".to_string(),
        }
    }

    // ── Path containment ─────────────────────────────────────────────────────

    #[test]
    fn test_path_containment_correct() {
        let folder = PathBuf::from("/home/user/Documents");
        let file = PathBuf::from("/home/user/Documents/report.pdf");
        assert!(is_path_under_folder(&file, &folder));
    }

    #[test]
    fn test_path_containment_no_false_prefix_match() {
        let folder = PathBuf::from("/home/user/Documents");
        let sibling = PathBuf::from("/home/user/DocumentsBackup/report.pdf");
        assert!(!is_path_under_folder(&sibling, &folder));
    }

    #[test]
    fn test_path_containment_exact_folder_excluded() {
        let folder = PathBuf::from("/home/user/Documents");
        assert!(!is_path_under_folder(&folder, &folder));
    }

    #[test]
    fn test_find_owning_folder_picks_most_specific() {
        let mut monitored = HashSet::new();
        monitored.insert(PathBuf::from("/home/user"));
        monitored.insert(PathBuf::from("/home/user/Documents"));
        let file = PathBuf::from("/home/user/Documents/report.pdf");
        let owner = find_owning_folder(&file, &monitored).unwrap();
        assert_eq!(owner, &PathBuf::from("/home/user/Documents"));
    }

    // ── Reconciliation ───────────────────────────────────────────────────────

    #[test]
    fn test_reconcile_inserts_new_file() {
        let conn = open_test_db();
        let fid = insert_folder(&conn, "/tmp/test");
        let fs_docs = vec![make_doc(fid, "/tmp/test/a.pdf", "a.pdf", "pdf")];
        let fs_paths: HashSet<String> = fs_docs.iter().map(|d| d.path.clone()).collect();
        let stats = reconcile_in_transaction(&conn, fid, &fs_docs, &fs_paths).unwrap();
        assert_eq!(stats.inserted, 1);
        assert_eq!(stats.marked_missing, 0);
        let active = get_active_paths_for_folder(&conn, fid).unwrap();
        assert!(active.contains("/tmp/test/a.pdf"));
    }

    #[test]
    fn test_reconcile_marks_missing_deleted_file() {
        let conn = open_test_db();
        let fid = insert_folder(&conn, "/tmp/test");
        save_document(
            &conn,
            &make_doc(fid, "/tmp/test/gone.pdf", "gone.pdf", "pdf"),
        )
        .unwrap();
        let stats = reconcile_in_transaction(&conn, fid, &[], &HashSet::new()).unwrap();
        assert_eq!(stats.marked_missing, 1);
        let active = get_active_paths_for_folder(&conn, fid).unwrap();
        assert!(!active.contains("/tmp/test/gone.pdf"));
    }

    #[test]
    fn test_reconcile_no_duplicates_on_repeated_run() {
        let conn = open_test_db();
        let fid = insert_folder(&conn, "/tmp/test");
        let fs_docs = vec![make_doc(fid, "/tmp/test/a.pdf", "a.pdf", "pdf")];
        let fs_paths: HashSet<String> = fs_docs.iter().map(|d| d.path.clone()).collect();
        reconcile_in_transaction(&conn, fid, &fs_docs, &fs_paths).unwrap();
        let stats2 = reconcile_in_transaction(&conn, fid, &fs_docs, &fs_paths).unwrap();
        assert_eq!(stats2.inserted, 0);
        assert_eq!(stats2.updated, 1);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE watched_folder_id=?1",
                [fid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_reconcile_rename_old_missing_new_active() {
        let conn = open_test_db();
        let fid = insert_folder(&conn, "/tmp/test");
        save_document(&conn, &make_doc(fid, "/tmp/test/old.pdf", "old.pdf", "pdf")).unwrap();
        let fs_docs = vec![make_doc(fid, "/tmp/test/new.pdf", "new.pdf", "pdf")];
        let fs_paths: HashSet<String> = fs_docs.iter().map(|d| d.path.clone()).collect();
        let stats = reconcile_in_transaction(&conn, fid, &fs_docs, &fs_paths).unwrap();
        assert_eq!(stats.inserted, 1);
        assert_eq!(stats.marked_missing, 1);
        let active = get_active_paths_for_folder(&conn, fid).unwrap();
        assert!(active.contains("/tmp/test/new.pdf"));
        assert!(!active.contains("/tmp/test/old.pdf"));
    }

    // ── build_document_record ────────────────────────────────────────────────

    #[test]
    fn test_build_document_record_real_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("report.pdf");
        fs::write(&path, b"PDF").unwrap();
        let doc = build_document_record(1, &path).unwrap();
        assert_eq!(doc.extension, "pdf");
        assert_eq!(doc.filename, "report.pdf");
        assert_eq!(doc.status, "ACTIVE");
        assert!(doc.size > 0);
    }

    #[test]
    fn test_build_document_record_missing_file_returns_none() {
        let path = PathBuf::from("/does/not/exist/file.pdf");
        let result = build_document_record(1, &path);
        assert!(result.is_none());
    }

    // ── Mark-missing ─────────────────────────────────────────────────────────

    #[test]
    fn test_mark_missing_command_updates_status() {
        let conn = open_test_db();
        let fid = insert_folder(&conn, "/tmp/t");
        save_document(&conn, &make_doc(fid, "/tmp/t/file.pdf", "file.pdf", "pdf")).unwrap();
        crate::database::mark_document_status(&conn, "/tmp/t/file.pdf", "MISSING").unwrap();
        let active = get_active_paths_for_folder(&conn, fid).unwrap();
        assert!(!active.contains("/tmp/t/file.pdf"));
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE path='/tmp/t/file.pdf' AND status='MISSING'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    // ── Queue pressure & guaranteed delivery ──────────────────────────────────

    #[test]
    fn test_queue_pressure_without_silent_command_loss() {
        // Channel with capacity of only 5 items
        let (tx, rx) = mpsc::sync_channel::<IndexCommand>(5);
        let sender = IndexSender(tx);

        let total_commands = 100;
        let sender_clone = sender.clone();

        // Producer thread sending under queue pressure
        let producer = std::thread::spawn(move || {
            for i in 0..total_commands {
                sender_clone
                    .send(IndexCommand::MarkMissing {
                        file_path: PathBuf::from(format!("/tmp/file_{i}.pdf")),
                    })
                    .expect("Blocking send should not fail while worker is connected");
            }
        });

        // Consumer thread draining the queue
        let mut received = 0;
        while received < total_commands {
            if let Ok(IndexCommand::MarkMissing { .. }) = rx.recv() {
                received += 1;
            }
        }

        producer.join().unwrap();
        assert_eq!(
            received, total_commands,
            "Zero commands dropped under queue pressure"
        );
    }

    #[test]
    fn test_worker_disconnect_handling() {
        let (tx, rx) = mpsc::sync_channel::<IndexCommand>(5);
        let sender = IndexSender(tx);

        // Drop the receiver to simulate dead/crashed worker
        drop(rx);

        let result = sender.send(IndexCommand::Shutdown);
        assert!(
            result.is_err(),
            "Must report error when worker is disconnected"
        );
        assert!(result.unwrap_err().contains("IndexWorker disconnected"));
    }

    // ── Folder removal while indexing ─────────────────────────────────────────

    #[test]
    fn test_folder_removal_while_indexing_aborts_cleanly() {
        let conn = open_test_db();
        // folder id 9999 does not exist in DB (e.g. was just removed)
        let non_existent_fid = 9999;
        let fs_docs = vec![make_doc(non_existent_fid, "/tmp/f/a.pdf", "a.pdf", "pdf")];
        let fs_paths: HashSet<String> = fs_docs.iter().map(|d| d.path.clone()).collect();

        let stats = reconcile_in_transaction(&conn, non_existent_fid, &fs_docs, &fs_paths).unwrap();
        assert_eq!(stats.inserted, 0);
        assert_eq!(stats.updated, 0);
        assert_eq!(stats.marked_missing, 0);

        // Verify no documents were inserted for the non-existent folder
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    // ── Missed watcher event reconciliation ───────────────────────────────────

    #[test]
    fn test_reconciliation_after_missed_watcher_event() {
        let dir = tempdir().unwrap();
        let folder_path = dir.path().to_string_lossy().to_string();

        let conn = open_test_db();
        let fid = insert_folder(&conn, &folder_path);

        // Create file directly on disk (simulating missed watcher event)
        let file_path = dir.path().join("unwatched.pdf");
        fs::write(&file_path, b"content").unwrap();

        let doc = build_document_record(fid, &file_path).unwrap();
        let fs_docs = vec![doc];
        let fs_paths: HashSet<String> = fs_docs.iter().map(|d| d.path.clone()).collect();

        let stats = reconcile_in_transaction(&conn, fid, &fs_docs, &fs_paths).unwrap();
        assert_eq!(stats.inserted, 1);

        let active = get_active_paths_for_folder(&conn, fid).unwrap();
        assert!(active.contains(&file_path.to_string_lossy().to_string()));
    }

    // ── Watcher + reconciliation race ─────────────────────────────────────────

    #[test]
    fn test_watcher_reconciliation_race() {
        let conn = open_test_db();
        let fid = insert_folder(&conn, "/tmp/race");
        let doc = make_doc(fid, "/tmp/race/doc.pdf", "doc.pdf", "pdf");

        // Simulate watcher event upserting the file
        save_document(&conn, &doc).unwrap();

        // Simulate concurrent reconcile running with the same file
        let fs_docs = vec![doc.clone()];
        let fs_paths: HashSet<String> = fs_docs.iter().map(|d| d.path.clone()).collect();
        let stats = reconcile_in_transaction(&conn, fid, &fs_docs, &fs_paths).unwrap();

        assert_eq!(stats.inserted, 0);
        assert_eq!(stats.updated, 1);

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE watched_folder_id=?1",
                [fid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Exactly one document record exists");
    }

    // ── Deterministic document removal semantics ──────────────────────────────

    #[test]
    fn test_deterministic_document_removal_via_worker() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("remove_me.pdf");
        fs::write(&file_path, b"PDF content").unwrap();

        let conn = Arc::new(Mutex::new(open_test_db()));
        let fid = {
            let c = conn.lock().unwrap();
            insert_folder(&c, &dir.path().to_string_lossy())
        };

        let (tx, rx) = mpsc::sync_channel::<IndexCommand>(10);
        let sender = IndexSender(tx);

        // Spawn IndexWorker in background
        let worker_conn = Arc::clone(&conn);
        let worker = IndexWorker::new_for_test(worker_conn, rx);
        let worker_thread = std::thread::spawn(move || worker.run());

        // 1. Index the document initially
        sender
            .send(IndexCommand::ScanFile {
                folder_id: fid,
                file_path: file_path.clone(),
            })
            .unwrap();

        // Query document id
        let doc_id = {
            // Give worker a moment to process
            std::thread::sleep(std::time::Duration::from_millis(50));
            let c = conn.lock().unwrap();
            let id: i64 = c
                .query_row(
                    "SELECT id FROM documents WHERE watched_folder_id=?1",
                    [fid],
                    |r| r.get(0),
                )
                .unwrap();
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

        // 3. Verify removed from DB
        {
            let c = conn.lock().unwrap();
            let count: i64 = c
                .query_row(
                    "SELECT COUNT(*) FROM documents WHERE id=?1",
                    [doc_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "Document must be removed from SQLite");
        }

        // 4. Verify physical file is untouched
        assert!(
            file_path.exists(),
            "Physical file on disk must NOT be deleted"
        );

        // Shutdown worker
        sender.send(IndexCommand::Shutdown).unwrap();
        worker_thread.join().unwrap();
    }

    // ── Final filesystem / index convergence ──────────────────────────────────

    #[test]
    fn test_final_filesystem_index_convergence_after_mixed_operations() {
        let dir = tempdir().unwrap();
        let folder_path = dir.path().to_string_lossy().to_string();

        let conn = open_test_db();
        let fid = insert_folder(&conn, &folder_path);

        // 1. Create 3 files
        let f1 = dir.path().join("doc1.pdf");
        let f2 = dir.path().join("doc2.docx");
        let f3 = dir.path().join("doc3.txt");
        fs::write(&f1, b"v1").unwrap();
        fs::write(&f2, b"v1").unwrap();
        fs::write(&f3, b"v1").unwrap();

        // Initial reconcile
        let d1 = build_document_record(fid, &f1).unwrap();
        let d2 = build_document_record(fid, &f2).unwrap();
        let d3 = build_document_record(fid, &f3).unwrap();
        let fs_docs = vec![d1, d2, d3];
        let fs_paths: HashSet<String> = fs_docs.iter().map(|d| d.path.clone()).collect();
        reconcile_in_transaction(&conn, fid, &fs_docs, &fs_paths).unwrap();

        // 2. Modify f1, delete f2, rename f3 -> f4
        fs::write(&f1, b"v2-updated").unwrap();
        fs::remove_file(&f2).unwrap();
        let f4 = dir.path().join("doc4.txt");
        fs::rename(&f3, &f4).unwrap();

        // Re-scan and reconcile against live disk
        let mut live_docs = Vec::new();
        for entry in fs::read_dir(dir.path()).unwrap() {
            let entry = entry.unwrap();
            let p = entry.path();
            if is_supported_doc(&p) {
                live_docs.push(build_document_record(fid, &p).unwrap());
            }
        }
        let live_paths: HashSet<String> = live_docs.iter().map(|d| d.path.clone()).collect();
        let stats = reconcile_in_transaction(&conn, fid, &live_docs, &live_paths).unwrap();

        assert_eq!(stats.inserted, 1, "doc4.txt was inserted");
        assert_eq!(stats.updated, 1, "doc1.pdf was updated");
        assert_eq!(
            stats.marked_missing, 2,
            "doc2.docx and doc3.txt were marked MISSING"
        );

        let active = get_active_paths_for_folder(&conn, fid).unwrap();
        assert_eq!(active.len(), 2);
        assert!(active.contains(&f1.to_string_lossy().to_string()));
        assert!(active.contains(&f4.to_string_lossy().to_string()));
        assert!(!active.contains(&f2.to_string_lossy().to_string()));
        assert!(!active.contains(&f3.to_string_lossy().to_string()));
    }
}
