use crate::database::{
    add_folder, get_folders, query_documents, query_documents_like, remove_folder, DocumentRecord,
    WatchedFolder,
};
use crate::indexer::IndexCommand;
use crate::AppState;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn cmd_add_folder(
    path: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<WatchedFolder, String> {
    // 1. Persist folder to DB
    let folder = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        add_folder(&conn, &path).map_err(|e| format!("Failed to add folder: {e}"))?
    };

    // 2. Register with watcher
    {
        let mut watcher = state.watcher.lock().map_err(|e| e.to_string())?;
        if let Err(e) = watcher.watch_folder(&folder.path, app_handle) {
            eprintln!("[cmd_add_folder] watcher warning: {e}");
        }
    }

    // 3. Kick off reconciliation (non-blocking — IndexWorker handles it)
    state.index_tx.send(IndexCommand::ReconcileFolder {
        folder_id: folder.id,
        folder_path: folder.path.clone(),
    })?;

    Ok(folder)
}

#[tauri::command]
pub async fn cmd_get_folders(state: State<'_, AppState>) -> Result<Vec<WatchedFolder>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    get_folders(&conn).map_err(|e| format!("Failed to get folders: {e}"))
}

#[tauri::command]
pub async fn cmd_remove_folder(folder_id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    // 1. Look up path before deleting (needed for unwatch)
    let folder_path = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let folders = get_folders(&conn).map_err(|e| format!("Failed to look up folder: {e}"))?;
        folders
            .into_iter()
            .find(|f| f.id == folder_id)
            .map(|f| f.path)
    };

    // 2. Stop watching BEFORE removing DB records
    if let Some(ref path) = folder_path {
        let mut watcher = state.watcher.lock().map_err(|e| e.to_string())?;
        if let Err(e) = watcher.unwatch_folder(path) {
            eprintln!("[cmd_remove_folder] unwatch warning: {e}");
            // Non-fatal: continue with DB removal
        }
    }

    // 3. Remove from DB (cascade deletes child documents)
    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        remove_folder(&conn, folder_id)
            .map_err(|e| format!("Failed to remove folder from DB: {e}"))?;
    }

    Ok(true)
}

#[tauri::command]
pub async fn cmd_rescan_folder(folder_id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    let folder = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let folders = get_folders(&conn).map_err(|e| format!("Failed to look up folder: {e}"))?;
        folders.into_iter().find(|f| f.id == folder_id)
    };

    match folder {
        Some(f) => {
            // Non-blocking: IndexWorker runs reconciliation in background
            state.index_tx.send(IndexCommand::ReconcileFolder {
                folder_id: f.id,
                folder_path: f.path,
            })?;
            Ok(true)
        }
        None => Err(format!("Folder with id={folder_id} not found")),
    }
}

#[tauri::command]
pub async fn cmd_get_documents(
    folder_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<DocumentRecord>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    query_documents(&conn, folder_id).map_err(|e| format!("Failed to query documents: {e}"))
}

#[tauri::command]
pub async fn cmd_search_documents(
    query: String,
    folder_id: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<DocumentRecord>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    query_documents_like(&conn, &query, folder_id).map_err(|e| format!("Search failed: {e}"))
}

/// Open a file using the OS default application.
///
/// Safety: does NOT invoke a shell interpreter.
/// - Windows: uses `explorer.exe <path>` directly (ShellExecute equivalent via Process)
/// - macOS:   `open <path>`
/// - Linux:   `xdg-open <path>`
///
/// The path is passed as a single argument — no shell interpolation occurs.
/// Filenames containing `&`, `;`, spaces, quotes, or Unicode are handled correctly.
#[tauri::command]
pub async fn cmd_open_document(path: String) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open file: {e}"))?;
    }
    Ok(true)
}

/// Sole write path for document deletion — routes through IndexWorker to uphold
/// the Single Writer invariant on the `documents` table.
#[tauri::command]
pub async fn cmd_remove_document(doc_id: i64, state: State<'_, AppState>) -> Result<bool, String> {
    let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
    state.index_tx.send(IndexCommand::RemoveDocument {
        doc_id,
        reply: Some(reply_tx),
    })?;
    reply_rx
        .recv()
        .map_err(|e| format!("Failed to receive confirmation from IndexWorker: {e}"))?
}
