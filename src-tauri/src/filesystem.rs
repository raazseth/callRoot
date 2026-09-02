use crate::database::{current_timestamp, DocumentRecord};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    pub processed: u64,
    pub total: u64,
    pub current_file: String,
    pub is_scanning: bool,
}

// Supported document extensions
const SUPPORTED_EXTENSIONS: &[&str] = &[
    "pdf", "docx", "doc", "txt", "md", "markdown", "rtf", "odt", "csv", "xlsx", "xls", "pptx", "ppt", "epub", "json", "yaml", "yml", "xml", "html", "htm", "log",
];

// Excluded system / hidden directories
const EXCLUDED_DIRS: &[&str] = &[
    ".git", ".vscode", ".idea", "node_modules", "target", "build", "dist", "AppData", "$RECYCLE.BIN", "System Volume Information", "tmp", "temp",
];

pub fn is_supported_doc(path: &Path) -> bool {
    // Skip hidden files or folders starting with '.'
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if name.starts_with('.') {
            return false;
        }
    }

    // Ignore symlinks or junctions
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return false;
        }
    }

    // Check extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let clean_ext = ext.to_lowercase();
        return SUPPORTED_EXTENSIONS.contains(&clean_ext.as_str());
    }

    false
}

pub fn is_dir_excluded(dir_name: &str) -> bool {
    if dir_name.starts_with('.') {
        return true;
    }
    EXCLUDED_DIRS.iter().any(|&ex| dir_name.eq_ignore_ascii_case(ex))
}

pub fn scan_folder_files(
    watched_folder_id: i64,
    folder_path: &str,
    app_handle: Option<&AppHandle>,
) -> Vec<DocumentRecord> {
    let mut documents = Vec::new();
    let root_path = Path::new(folder_path);

    if !root_path.exists() || !root_path.is_dir() {
        return documents;
    }

    let mut scanned_count = 0u64;

    let walker = WalkDir::new(root_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.file_type().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    return !is_dir_excluded(name);
                }
            }
            true
        });

    for entry in walker.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() && is_supported_doc(path) {
            scanned_count += 1;

            if let Ok(metadata) = fs::metadata(path) {
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
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

                let doc = DocumentRecord {
                    id: 0,
                    watched_folder_id,
                    path: path.to_string_lossy().to_string(),
                    filename,
                    extension,
                    size,
                    created_at,
                    modified_at,
                    indexed_at: current_timestamp(),
                    status: "ACTIVE".to_string(),
                };

                documents.push(doc);

                // Emit progress every 20 files
                if scanned_count % 20 == 0 {
                    if let Some(app) = app_handle {
                        let _ = app.emit(
                            "scan-progress",
                            ScanProgress {
                                processed: scanned_count,
                                total: scanned_count + 100, // estimated
                                current_file: path.to_string_lossy().to_string(),
                                is_scanning: true,
                            },
                        );
                    }
                }
            }
        }
    }

    if let Some(app) = app_handle {
        let _ = app.emit(
            "scan-progress",
            ScanProgress {
                processed: scanned_count,
                total: scanned_count,
                current_file: "Finished".to_string(),
                is_scanning: false,
            },
        );
    }

    documents
}

// Background File Watcher Manager
pub struct AppWatcherManager {
    watcher: Option<RecommendedWatcher>,
    monitored_paths: HashSet<String>,
}

impl AppWatcherManager {
    pub fn new() -> Self {
        Self {
            watcher: None,
            monitored_paths: HashSet::new(),
        }
    }

    pub fn watch_folder(&mut self, folder_path: &str, app_handle: AppHandle) {
        if self.monitored_paths.contains(folder_path) {
            return;
        }

        let path = PathBuf::from(folder_path);
        if !path.exists() {
            return;
        }

        if self.watcher.is_none() {
            let (tx, rx) = channel();

            let watcher_res = RecommendedWatcher::new(
                move |res| {
                    let _ = tx.send(res);
                },
                Config::default(),
            );

            if let Ok(mut watcher) = watcher_res {
                let handle = app_handle.clone();
                // Spawn background event processor thread with 200ms debouncing
                thread::spawn(move || {
                    let mut last_event_time = std::time::Instant::now();
                    let debounce_duration = Duration::from_millis(200);

                    while let Ok(event_res) = rx.recv() {
                        if let Ok(event) = event_res {
                            if last_event_time.elapsed() < debounce_duration {
                                thread::sleep(Duration::from_millis(50));
                            }
                            last_event_time = std::time::Instant::now();

                            handle_fs_event(event, &handle);
                        }
                    }
                });

                let _ = watcher.watch(&path, RecursiveMode::Recursive);
                self.watcher = Some(watcher);
                self.monitored_paths.insert(folder_path.to_string());
            }
        } else if let Some(ref mut watcher) = self.watcher {
            let _ = watcher.watch(&path, RecursiveMode::Recursive);
            self.monitored_paths.insert(folder_path.to_string());
        }
    }
}

fn handle_fs_event(event: Event, app_handle: &AppHandle) {
    for path in event.paths {
        if is_supported_doc(&path) {
            // Signal frontend to reload document list
            let _ = app_handle.emit(
                "scan-progress",
                ScanProgress {
                    processed: 1,
                    total: 1,
                    current_file: path.to_string_lossy().to_string(),
                    is_scanning: false,
                },
            );
        }
    }
}
