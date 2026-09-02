use crate::database::{
    add_folder, get_folders, query_documents, query_documents_like, remove_document, remove_folder, save_document,
    WatchedFolder, DocumentRecord,
};
use crate::filesystem::scan_folder_files;
use crate::AppState;
use tauri::{AppHandle, State};
use std::process::Command;

#[tauri::command]
pub async fn cmd_add_folder(
    path: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<WatchedFolder, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;

    let folder = add_folder(&conn, &path).map_err(|e| e.to_string())?;
    
    // Watch folder
    if let Ok(mut watcher_mgr) = state.watcher.lock() {
        watcher_mgr.watch_folder(&folder.path, app_handle.clone());
    }

    // Perform initial scan backgrounded
    let folder_id = folder.id;
    let folder_path = folder.path.clone();

    // Release lock before scan
    drop(conn);

    let state_db = state.db.clone();
    let app = app_handle.clone();
    tokio::task::spawn_blocking(move || {
        let docs = scan_folder_files(folder_id, &folder_path, Some(&app));
        if let Ok(conn) = state_db.lock() {
            for doc in docs {
                let _ = save_document(&conn, &doc);
            }
        }
    });

    Ok(folder)
}

#[tauri::command]
pub async fn cmd_get_folders(state: State<'_, AppState>) -> Result<Vec<WatchedFolder>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    get_folders(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cmd_remove_folder(folder_id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    remove_folder(&conn, folder_id).map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn cmd_rescan_folder(
    folder_id: i64,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<bool, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    let folders = get_folders(&conn).map_err(|e| e.to_string())?;
    
    let folder = folders.into_iter().find(|f| f.id == folder_id);
    drop(conn);

    if let Some(target) = folder {
        let state_db = state.db.clone();
        let app = app_handle.clone();
        tokio::task::spawn_blocking(move || {
            let docs = scan_folder_files(target.id, &target.path, Some(&app));
            if let Ok(conn) = state_db.lock() {
                for doc in docs {
                    let _ = save_document(&conn, &doc);
                }
            }
        });
    }

    Ok(true)
}

#[tauri::command]
pub async fn cmd_get_documents(
    folder_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<DocumentRecord>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    query_documents(&conn, folder_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cmd_search_documents(
    query: String,
    folder_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<DocumentRecord>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    query_documents_like(&conn, &query, folder_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cmd_open_document(path: String) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(true)
}

#[tauri::command]
pub async fn cmd_remove_document(doc_id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    remove_document(&conn, doc_id).map_err(|e| e.to_string())?;
    Ok(true)
}
