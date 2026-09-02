pub mod commands;
pub mod database;
pub mod filesystem;

use database::{get_folders, init_database};
use filesystem::{scan_folder_files, AppWatcherManager};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use tauri::{Manager, RunEvent};

pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub watcher: Arc<Mutex<AppWatcherManager>>,
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
            let watcher_arc = Arc::new(Mutex::new(AppWatcherManager::new()));

            app.manage(AppState {
                db: db_arc.clone(),
                watcher: watcher_arc.clone(),
            });

            // Start watching folders and perform scan_on_launch
            let app_handle = app.handle().clone();
            let db_clone = db_arc.clone();
            let watcher_clone = watcher_arc.clone();

            tauri::async_runtime::spawn(async move {
                if let Ok(conn) = db_clone.lock() {
                    if let Ok(folders) = get_folders(&conn) {
                        drop(conn);
                        for folder in folders {
                            if let Ok(mut mgr) = watcher_clone.lock() {
                                mgr.watch_folder(&folder.path, app_handle.clone());
                            }

                            // Perform startup background scan
                            let db = db_clone.clone();
                            let handle = app_handle.clone();
                            let f_id = folder.id;
                            let f_path = folder.path.clone();

                            tokio::task::spawn_blocking(move || {
                                let docs = scan_folder_files(f_id, &f_path, Some(&handle));
                                if let Ok(conn) = db.lock() {
                                    for doc in docs {
                                        let _ = database::save_document(&conn, &doc);
                                    }
                                }
                            });
                        }
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
        .run(|_app_handle, event| match event {
            RunEvent::ExitRequested { .. } => {}
            _ => {}
        });
}
