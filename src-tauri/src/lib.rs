pub mod commands;
pub mod database;
pub mod filesystem;
pub mod indexer;

use database::{get_folders, init_database};
use filesystem::AppWatcherManager;
use indexer::{IndexCommand, IndexSender, IndexWorker};
use rusqlite::Connection;
use std::sync::{mpsc, Arc, Mutex};
use tauri::{Manager, RunEvent};

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub watcher: Arc<Mutex<AppWatcherManager>>,
    /// Sender end of the IndexWorker channel. Clone freely; the worker owns the Receiver.
    pub index_tx: IndexSender,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let db_path = app_dir.join("root.db");

            let conn = init_database(&db_path).expect("Failed to initialize SQLite database");
            let db_arc = Arc::new(Mutex::new(conn));

            // Create the IndexWorker channel (bounded 1024 to bound burst events)
            let (tx, rx) = mpsc::sync_channel::<IndexCommand>(1024);
            let index_sender = IndexSender(tx.clone());

            // Spawn the IndexWorker on a dedicated std::thread (not Tokio's blocking pool)
            let worker_db = db_arc.clone();
            let worker_handle = app.handle().clone();
            std::thread::Builder::new()
                .name("root-index-worker".to_string())
                .spawn(move || {
                    IndexWorker::new(worker_db, worker_handle, rx).run();
                })
                .expect("Failed to spawn IndexWorker thread");

            let watcher_arc = Arc::new(Mutex::new(AppWatcherManager::new(index_sender.clone())));

            app.manage(AppState {
                db: db_arc.clone(),
                watcher: watcher_arc.clone(),
                index_tx: index_sender.clone(),
            });

            // Re-watch all previously saved folders and kick off reconciliation
            let app_handle = app.handle().clone();
            let db_clone = db_arc.clone();
            let watcher_clone = watcher_arc.clone();
            let tx_clone = index_sender.clone();

            tauri::async_runtime::spawn(async move {
                let folders = {
                    match db_clone.lock() {
                        Ok(conn) => match get_folders(&conn) {
                            Ok(f) => f,
                            Err(e) => {
                                eprintln!("[startup] failed to read folders: {e}");
                                return;
                            }
                        },
                        Err(e) => {
                            eprintln!("[startup] DB lock error: {e}");
                            return;
                        }
                    }
                };

                for folder in folders {
                    // Register with watcher
                    if let Ok(mut mgr) = watcher_clone.lock() {
                        if let Err(e) = mgr.watch_folder(&folder.path, app_handle.clone()) {
                            eprintln!("[startup] watcher warning for {}: {e}", folder.path);
                        }
                    }

                    // Queue reconciliation for each folder
                    if let Err(e) = tx_clone.send(IndexCommand::ReconcileFolder {
                        folder_id: folder.id,
                        folder_path: folder.path,
                    }) {
                        eprintln!("[startup] failed to queue reconcile: {e}");
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::cmd_add_folder,
            commands::cmd_get_folders,
            commands::cmd_remove_folder,
            commands::cmd_rescan_folder,
            commands::cmd_get_documents,
            commands::cmd_search_documents,
            commands::cmd_open_document,
            commands::cmd_remove_document
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            RunEvent::ExitRequested { .. } => {
                // Signal the IndexWorker to shut down cleanly
                if let Some(state) = app_handle.try_state::<AppState>() {
                    if let Err(e) = state.index_tx.send(IndexCommand::Shutdown) {
                        eprintln!("[shutdown] failed to signal worker shutdown: {e}");
                    }
                }
            }
            _ => {}
        });
}
