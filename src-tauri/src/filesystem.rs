//! filesystem.rs — Scanner and watcher.
//!
//! Responsibilities:
//!   - Extension + directory filtering helpers
//!   - AppWatcherManager: wraps notify, routes events to IndexSender
//!
//! The watcher does NOT write to SQLite directly.
//! It sends IndexCommand messages via IndexSender.

use crate::indexer::{find_owning_folder, IndexCommand, IndexSender};
use notify::event::ModifyKind;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc::channel, Arc, RwLock};
use std::thread;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub processed: u64,
    pub total: u64,
    pub current_file: String,
    pub is_scanning: bool,
}

pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "pdf", "docx", "doc", "txt", "md", "markdown", "rtf", "odt", "csv", "xlsx", "xls", "pptx",
    "ppt", "epub", "json", "yaml", "yml", "xml", "html", "htm", "log",
];

const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".vscode",
    ".idea",
    "node_modules",
    "target",
    "build",
    "dist",
    "AppData",
    "$RECYCLE.BIN",
    "System Volume Information",
    "tmp",
    "temp",
];

pub fn is_supported_doc(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with('.') {
            return false;
        }
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return false;
        }
    }
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return SUPPORTED_EXTENSIONS.contains(&ext.to_lowercase().as_str());
    }
    false
}

pub fn is_dir_excluded(dir_name: &str) -> bool {
    if dir_name.starts_with('.') {
        return true;
    }
    EXCLUDED_DIRS
        .iter()
        .any(|&ex| dir_name.eq_ignore_ascii_case(ex))
}

// ─── Watcher manager ──────────────────────────────────────────────────────────

pub struct AppWatcherManager {
    watcher: Option<RecommendedWatcher>,
    /// Shared live state with the event loop thread so watch/unwatch mutations are immediately visible.
    monitored_paths: Arc<RwLock<HashSet<PathBuf>>>,
    index_tx: IndexSender,
}

impl AppWatcherManager {
    pub fn new(index_tx: IndexSender) -> Self {
        Self {
            watcher: None,
            monitored_paths: Arc::new(RwLock::new(HashSet::new())),
            index_tx,
        }
    }

    /// Expose read-only reference to monitored_paths for testing live visibility.
    pub fn monitored_paths(&self) -> Arc<RwLock<HashSet<PathBuf>>> {
        Arc::clone(&self.monitored_paths)
    }

    /// Start watching `folder_path`. Idempotent.
    pub fn watch(&mut self, folder_path: &str) -> Result<(), String> {
        let path = PathBuf::from(folder_path);

        {
            let paths = self
                .monitored_paths
                .read()
                .map_err(|e| format!("[watcher] read lock error: {e}"))?;
            if paths.contains(&path) {
                return Ok(());
            }
        }

        if !path.exists() {
            return Err(format!("[watcher] path does not exist: {folder_path}"));
        }

        if self.watcher.is_none() {
            let (tx, rx) = channel();
            let watcher_res = RecommendedWatcher::new(
                move |res| {
                    if let Err(e) = tx.send(res) {
                        eprintln!("[watcher] callback channel send failed: {e}");
                    }
                },
                Config::default(),
            );

            match watcher_res {
                Ok(mut watcher) => {
                    let index_tx = self.index_tx.clone();
                    // Share the live monitored_paths Arc with the event thread
                    let monitored = Arc::clone(&self.monitored_paths);

                    thread::Builder::new()
                        .name("root-watcher-events".to_string())
                        .spawn(move || run_event_loop(rx, index_tx, monitored))
                        .map_err(|e| format!("[watcher] thread spawn failed: {e}"))?;

                    watcher
                        .watch(&path, RecursiveMode::Recursive)
                        .map_err(|e| format!("[watcher] watch failed for {folder_path}: {e}"))?;

                    self.watcher = Some(watcher);

                    let mut paths = self
                        .monitored_paths
                        .write()
                        .map_err(|e| format!("[watcher] write lock error: {e}"))?;
                    paths.insert(path);
                }
                Err(e) => return Err(format!("[watcher] failed to create watcher: {e}")),
            }
        } else if let Some(ref mut watcher) = self.watcher {
            watcher
                .watch(&path, RecursiveMode::Recursive)
                .map_err(|e| format!("[watcher] watch failed for {folder_path}: {e}"))?;
            let mut paths = self
                .monitored_paths
                .write()
                .map_err(|e| format!("[watcher] write lock error: {e}"))?;
            paths.insert(path);
        }

        eprintln!("[watcher] watching: {folder_path}");
        Ok(())
    }

    /// Delegate watch_folder with AppHandle for Tauri IPC compatibility.
    pub fn watch_folder(
        &mut self,
        folder_path: &str,
        _app_handle: AppHandle,
    ) -> Result<(), String> {
        self.watch(folder_path)
    }

    pub fn unwatch_folder(&mut self, folder_path: &str) -> Result<(), String> {
        let path = PathBuf::from(folder_path);

        {
            let paths = self
                .monitored_paths
                .read()
                .map_err(|e| format!("[watcher] read lock error: {e}"))?;
            if !paths.contains(&path) {
                return Ok(());
            }
        }

        if let Some(ref mut watcher) = self.watcher {
            watcher
                .unwatch(&path)
                .map_err(|e| format!("[watcher] unwatch failed for {folder_path}: {e}"))?;
        }

        let mut paths = self
            .monitored_paths
            .write()
            .map_err(|e| format!("[watcher] write lock error: {e}"))?;
        paths.remove(&path);
        eprintln!("[watcher] stopped watching: {folder_path}");
        Ok(())
    }
}

// ─── Event loop ───────────────────────────────────────────────────────────────

fn run_event_loop(
    rx: std::sync::mpsc::Receiver<notify::Result<Event>>,
    index_tx: IndexSender,
    monitored_paths: Arc<RwLock<HashSet<PathBuf>>>,
) {
    while let Ok(event_res) = rx.recv() {
        match event_res {
            Ok(event) => {
                let monitored_guard = match monitored_paths.read() {
                    Ok(g) => g,
                    Err(e) => {
                        eprintln!("[watcher] monitored_paths read lock poisoned: {e}");
                        continue;
                    }
                };
                handle_fs_event(event, &index_tx, &*monitored_guard);
            }
            Err(e) => eprintln!("[watcher] notify error: {e}"),
        }
    }
    eprintln!("[watcher] event loop exited");
}

pub fn handle_fs_event(event: Event, index_tx: &IndexSender, monitored: &HashSet<PathBuf>) {
    use notify::event::RenameMode;

    let send_cmd = |cmd: IndexCommand| {
        if let Err(e) = index_tx.send(cmd) {
            eprintln!("[watcher] failed to deliver command to IndexWorker: {e}");
        }
    };

    match &event.kind {
        EventKind::Create(_) => {
            for path in &event.paths {
                if find_owning_folder(path, monitored).is_some() && is_supported_doc(path) {
                    eprintln!("[watcher] CREATE: {}", path.display());
                    send_cmd(IndexCommand::UpsertFile {
                        file_path: path.clone(),
                    });
                }
            }
        }

        EventKind::Modify(ModifyKind::Data(_)) | EventKind::Modify(ModifyKind::Metadata(_)) => {
            for path in &event.paths {
                if find_owning_folder(path, monitored).is_some() && is_supported_doc(path) {
                    eprintln!("[watcher] MODIFY: {}", path.display());
                    send_cmd(IndexCommand::UpsertFile {
                        file_path: path.clone(),
                    });
                }
            }
        }

        EventKind::Remove(_) => {
            for path in &event.paths {
                eprintln!("[watcher] REMOVE: {}", path.display());
                send_cmd(IndexCommand::MarkMissing {
                    file_path: path.clone(),
                });
            }
        }

        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if event.paths.len() >= 2 {
                let old_path = &event.paths[0];
                let new_path = &event.paths[1];
                eprintln!(
                    "[watcher] RENAME (Both): {} -> {}",
                    old_path.display(),
                    new_path.display()
                );
                send_cmd(IndexCommand::MarkMissing {
                    file_path: old_path.clone(),
                });
                if find_owning_folder(new_path, monitored).is_some() && is_supported_doc(new_path) {
                    send_cmd(IndexCommand::UpsertFile {
                        file_path: new_path.clone(),
                    });
                }
            }
        }

        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            for path in &event.paths {
                eprintln!("[watcher] RENAME-FROM: {}", path.display());
                send_cmd(IndexCommand::MarkMissing {
                    file_path: path.clone(),
                });
            }
        }

        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            for path in &event.paths {
                if find_owning_folder(path, monitored).is_some() && is_supported_doc(path) {
                    eprintln!("[watcher] RENAME-TO: {}", path.display());
                    send_cmd(IndexCommand::UpsertFile {
                        file_path: path.clone(),
                    });
                }
            }
        }

        EventKind::Modify(ModifyKind::Name(RenameMode::Any)) => {
            if event.paths.len() >= 2 {
                let old_path = &event.paths[0];
                let new_path = &event.paths[1];
                send_cmd(IndexCommand::MarkMissing {
                    file_path: old_path.clone(),
                });
                if find_owning_folder(new_path, monitored).is_some() && is_supported_doc(new_path) {
                    send_cmd(IndexCommand::UpsertFile {
                        file_path: new_path.clone(),
                    });
                }
            } else if let Some(path) = event.paths.first() {
                if path.exists() {
                    if find_owning_folder(path, monitored).is_some() && is_supported_doc(path) {
                        send_cmd(IndexCommand::UpsertFile {
                            file_path: path.clone(),
                        });
                    }
                } else {
                    send_cmd(IndexCommand::MarkMissing {
                        file_path: path.clone(),
                    });
                }
            }
        }

        EventKind::Modify(ModifyKind::Any) | EventKind::Any => {
            for path in &event.paths {
                if find_owning_folder(path, monitored).is_some() && is_supported_doc(path) {
                    eprintln!("[watcher] MODIFY (Generic): {}", path.display());
                    send_cmd(IndexCommand::UpsertFile {
                        file_path: path.clone(),
                    });
                }
            }
        }

        EventKind::Access(_) | EventKind::Other => {}

        _ => {}
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_supported_extensions_all_accepted() {
        let dir = tempdir().unwrap();
        for ext in SUPPORTED_EXTENSIONS {
            let path = dir.path().join(format!("file.{ext}"));
            fs::write(&path, b"data").unwrap();
            assert!(
                is_supported_doc(&path),
                "Extension .{ext} should be supported"
            );
        }
    }

    #[test]
    fn test_unsupported_extensions_rejected() {
        let dir = tempdir().unwrap();
        for ext in &["exe", "dll", "zip", "tar", "mp3", "png", "jpg", "rs", "py"] {
            let path = dir.path().join(format!("file.{ext}"));
            fs::write(&path, b"data").unwrap();
            assert!(
                !is_supported_doc(&path),
                "Extension .{ext} should be rejected"
            );
        }
    }

    #[test]
    fn test_extension_check_is_case_insensitive() {
        let dir = tempdir().unwrap();
        for name in &["FILE.PDF", "doc.DOCX", "sheet.XlSx", "note.MD"] {
            let path = dir.path().join(name);
            fs::write(&path, b"data").unwrap();
            assert!(
                is_supported_doc(&path),
                "{name} should be accepted (case-insensitive)"
            );
        }
    }

    #[test]
    fn test_hidden_files_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".hidden.pdf");
        fs::write(&path, b"data").unwrap();
        assert!(!is_supported_doc(&path));
    }

    #[test]
    fn test_no_extension_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("noextension");
        fs::write(&path, b"data").unwrap();
        assert!(!is_supported_doc(&path));
    }

    #[test]
    fn test_excluded_dirs_all_rejected() {
        for name in &[
            ".git",
            ".vscode",
            "node_modules",
            "target",
            "build",
            "dist",
            "tmp",
            "temp",
        ] {
            assert!(is_dir_excluded(name), "{name} should be excluded");
        }
    }

    #[test]
    fn test_excluded_dirs_case_insensitive() {
        assert!(is_dir_excluded("NODE_MODULES"));
        assert!(is_dir_excluded("Target"));
        assert!(is_dir_excluded("DIST"));
    }

    #[test]
    fn test_hidden_dirs_excluded() {
        assert!(is_dir_excluded(".hidden_dir"));
        assert!(is_dir_excluded(".git"));
    }

    #[test]
    fn test_normal_dirs_not_excluded() {
        assert!(!is_dir_excluded("Documents"));
        assert!(!is_dir_excluded("Projects"));
        assert!(!is_dir_excluded("my_folder"));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlinks_rejected() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real.pdf");
        fs::write(&real, b"data").unwrap();
        let link = dir.path().join("link.pdf");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(!is_supported_doc(&link), "symlink should be rejected");
    }

    #[test]
    fn test_unicode_filenames_accepted() {
        let dir = tempdir().unwrap();
        let names = ["документ.pdf", "文件.docx", "résumé.pdf"];
        for name in &names {
            let path = dir.path().join(name);
            fs::write(&path, b"data").unwrap();
            assert!(is_supported_doc(&path), "{name} should be accepted");
        }
    }

    #[test]
    fn test_special_character_filenames_accepted() {
        let dir = tempdir().unwrap();
        // These should index fine — special chars in name, not path separator
        for name in &[
            "file with spaces.pdf",
            "report(final).docx",
            "data_2024.xlsx",
        ] {
            let path = dir.path().join(name);
            fs::write(&path, b"data").unwrap();
            assert!(is_supported_doc(&path), "{name} should be accepted");
        }
    }

    #[test]
    fn test_dynamic_watch_unwatch_visibility() {
        let monitored = Arc::new(RwLock::new(HashSet::new()));

        // 1. Initial empty
        assert!(monitored.read().unwrap().is_empty());

        // 2. Add first folder
        let folder1 = PathBuf::from("/data/docs");
        monitored.write().unwrap().insert(folder1.clone());
        assert!(monitored.read().unwrap().contains(&folder1));

        // 3. Add second folder after startup
        let folder2 = PathBuf::from("/data/projects");
        monitored.write().unwrap().insert(folder2.clone());
        assert!(monitored.read().unwrap().contains(&folder1));
        assert!(monitored.read().unwrap().contains(&folder2));

        // 4. Remove first folder
        monitored.write().unwrap().remove(&folder1);
        assert!(!monitored.read().unwrap().contains(&folder1));
        assert!(monitored.read().unwrap().contains(&folder2));
    }

    #[test]
    fn test_similar_prefix_folder_isolation() {
        let mut monitored = HashSet::new();
        let folder = PathBuf::from("/data/docs");
        monitored.insert(folder.clone());

        let target_file = PathBuf::from("/data/docs/report.pdf");
        let similar_file = PathBuf::from("/data/docs-old/report.pdf");
        let sibling_file = PathBuf::from("/data/docs_archive/report.pdf");

        assert_eq!(find_owning_folder(&target_file, &monitored), Some(&folder));
        assert_eq!(find_owning_folder(&similar_file, &monitored), None);
        assert_eq!(find_owning_folder(&sibling_file, &monitored), None);
    }

    #[test]
    fn test_watcher_first_second_and_removed_folder_events() {
        let (tx, rx) = std::sync::mpsc::sync_channel(20);
        let index_sender = IndexSender(tx);
        let monitored = Arc::new(RwLock::new(HashSet::new()));

        let dir = tempdir().unwrap();
        let folder1 = dir.path().join("f1");
        let folder2 = dir.path().join("f2");
        fs::create_dir_all(&folder1).unwrap();
        fs::create_dir_all(&folder2).unwrap();

        // 1. Add first folder
        monitored.write().unwrap().insert(folder1.clone());

        let file1 = folder1.join("doc1.pdf");
        fs::write(&file1, b"doc1").unwrap();
        let event1 = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![file1.clone()],
            attrs: Default::default(),
        };
        handle_fs_event(event1, &index_sender, &*monitored.read().unwrap());

        // First folder receives event
        match rx.try_recv().unwrap() {
            IndexCommand::UpsertFile { file_path } => assert_eq!(file_path, file1),
            other => panic!("Expected UpsertFile, got {other:?}"),
        }

        // 2. Add second folder dynamically
        monitored.write().unwrap().insert(folder2.clone());

        let file2 = folder2.join("doc2.pdf");
        fs::write(&file2, b"doc2").unwrap();
        let event2 = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![file2.clone()],
            attrs: Default::default(),
        };
        handle_fs_event(event2, &index_sender, &*monitored.read().unwrap());

        // Second folder receives event
        match rx.try_recv().unwrap() {
            IndexCommand::UpsertFile { file_path } => assert_eq!(file_path, file2),
            other => panic!("Expected UpsertFile, got {other:?}"),
        }

        // 3. Remove first folder dynamically
        monitored.write().unwrap().remove(&folder1);

        let file3 = folder1.join("doc3.pdf");
        fs::write(&file3, b"doc3").unwrap();
        let event3 = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![file3],
            attrs: Default::default(),
        };
        handle_fs_event(event3, &index_sender, &*monitored.read().unwrap());

        // Removed folder produces NO event
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_rename_modes_and_cross_folder_handling() {
        let (tx, rx) = std::sync::mpsc::sync_channel(20);
        let index_sender = IndexSender(tx);

        let dir = tempdir().unwrap();
        let folder_a = dir.path().join("folder_a");
        let folder_b = dir.path().join("folder_b");
        let outside = dir.path().join("outside");
        fs::create_dir_all(&folder_a).unwrap();
        fs::create_dir_all(&folder_b).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let mut monitored = HashSet::new();
        monitored.insert(folder_a.clone());
        monitored.insert(folder_b.clone());

        // Rename within same watched folder
        let old_file = folder_a.join("old.pdf");
        let new_file = folder_a.join("new.pdf");
        fs::write(&new_file, b"data").unwrap();

        let both_event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            paths: vec![old_file.clone(), new_file.clone()],
            attrs: Default::default(),
        };
        handle_fs_event(both_event, &index_sender, &monitored);

        match rx.try_recv().unwrap() {
            IndexCommand::MarkMissing { file_path } => assert_eq!(file_path, old_file),
            other => panic!("Expected MarkMissing, got {other:?}"),
        }
        match rx.try_recv().unwrap() {
            IndexCommand::UpsertFile { file_path } => assert_eq!(file_path, new_file),
            other => panic!("Expected UpsertFile, got {other:?}"),
        }

        // Rename from monitored to outside folder
        let moved_outside = outside.join("moved.pdf");
        fs::write(&moved_outside, b"data").unwrap();
        let out_event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            paths: vec![new_file.clone(), moved_outside],
            attrs: Default::default(),
        };
        handle_fs_event(out_event, &index_sender, &monitored);

        match rx.try_recv().unwrap() {
            IndexCommand::MarkMissing { file_path } => assert_eq!(file_path, new_file),
            other => panic!("Expected MarkMissing, got {other:?}"),
        }
        // Outside file must NOT be upserted
        assert!(rx.try_recv().is_err());

        // Rename between watched folders (folder_a -> folder_b)
        let file_in_a = folder_a.join("move_to_b.pdf");
        let file_in_b = folder_b.join("moved_in_b.pdf");
        fs::write(&file_in_b, b"data").unwrap();

        let cross_event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Name(
                notify::event::RenameMode::Both,
            )),
            paths: vec![file_in_a.clone(), file_in_b.clone()],
            attrs: Default::default(),
        };
        handle_fs_event(cross_event, &index_sender, &monitored);

        match rx.try_recv().unwrap() {
            IndexCommand::MarkMissing { file_path } => assert_eq!(file_path, file_in_a),
            other => panic!("Expected MarkMissing, got {other:?}"),
        }
        match rx.try_recv().unwrap() {
            IndexCommand::UpsertFile { file_path } => assert_eq!(file_path, file_in_b),
            other => panic!("Expected UpsertFile, got {other:?}"),
        }
    }

    #[test]
    fn test_concurrent_folder_add_remove() {
        let monitored = Arc::new(RwLock::new(HashSet::new()));
        let mut handles = Vec::new();

        for i in 0..8 {
            let m = Arc::clone(&monitored);
            let handle = std::thread::spawn(move || {
                let path = PathBuf::from(format!("/folder_{i}"));
                for _ in 0..50 {
                    m.write().unwrap().insert(path.clone());
                    let _ = m.read().unwrap().contains(&path);
                    m.write().unwrap().remove(&path);
                }
            });
            handles.push(handle);
        }

        for h in handles {
            h.join().unwrap();
        }
    }
}
