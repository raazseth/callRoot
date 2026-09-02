use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedFolder {
    pub id: i64,
    pub path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRecord {
    pub id: i64,
    pub watched_folder_id: i64,
    pub path: String,
    pub filename: String,
    pub extension: String,
    pub size: u64,
    pub created_at: i64,
    pub modified_at: i64,
    pub indexed_at: i64,
    pub status: String, // "ACTIVE", "MISSING", "IGNORED"
}

pub fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn init_database(db_path: &Path) -> SqlResult<Connection> {
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let conn = Connection::open(db_path)?;

    // Enable WAL mode and Foreign Keys
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        ",
    )?;

    // Initialize Schema
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS watched_folders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS documents (
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
        );

        CREATE INDEX IF NOT EXISTS idx_documents_watched_folder ON documents(watched_folder_id);
        CREATE INDEX IF NOT EXISTS idx_documents_filename ON documents(filename ASC);
        CREATE INDEX IF NOT EXISTS idx_documents_status ON documents(status);

        CREATE TABLE IF NOT EXISTS app_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            scan_on_launch BOOLEAN NOT NULL DEFAULT 1
        );

        INSERT OR IGNORE INTO app_settings (id, scan_on_launch) VALUES (1, 1);
        ",
    )?;

    Ok(conn)
}

pub fn add_folder(conn: &Connection, folder_path: &str) -> SqlResult<WatchedFolder> {
    let now = current_timestamp();
    conn.execute(
        "INSERT INTO watched_folders (path, created_at, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET updated_at=?3",
        params![folder_path, now, now],
    )?;

    let mut stmt = conn.prepare("SELECT id, path, created_at, updated_at FROM watched_folders WHERE path = ?1")?;
    let folder = stmt.query_row(params![folder_path], |row| {
        Ok(WatchedFolder {
            id: row.get(0)?,
            path: row.get(1)?,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })?;

    Ok(folder)
}

pub fn get_folders(conn: &Connection) -> SqlResult<Vec<WatchedFolder>> {
    let mut stmt = conn.prepare("SELECT id, path, created_at, updated_at FROM watched_folders ORDER BY id ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok(WatchedFolder {
            id: row.get(0)?,
            path: row.get(1)?,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })?;

    let mut folders = Vec::new();
    for folder in rows {
        folders.push(folder?);
    }
    Ok(folders)
}

pub fn remove_folder(conn: &Connection, folder_id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM watched_folders WHERE id = ?1", params![folder_id])?;
    Ok(())
}

pub fn save_document(conn: &Connection, doc: &DocumentRecord) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO documents 
            (watched_folder_id, path, filename, extension, size, created_at, modified_at, indexed_at, status) 
         VALUES 
            (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(path) DO UPDATE SET
            filename=?3,
            extension=?4,
            size=?5,
            modified_at=?7,
            indexed_at=?8,
            status=?9",
        params![
            doc.watched_folder_id,
            doc.path,
            doc.filename,
            doc.extension,
            doc.size,
            doc.created_at,
            doc.modified_at,
            doc.indexed_at,
            doc.status,
        ],
    )?;
    Ok(())
}

pub fn mark_document_status(conn: &Connection, doc_path: &str, status: &str) -> SqlResult<()> {
    conn.execute(
        "UPDATE documents SET status = ?1 WHERE path = ?2",
        params![status, doc_path],
    )?;
    Ok(())
}

pub fn remove_document(conn: &Connection, doc_id: i64) -> SqlResult<()> {
    conn.execute("DELETE FROM documents WHERE id = ?1", params![doc_id])?;
    Ok(())
}

pub fn query_documents(conn: &Connection, folder_id: Option<i64>) -> SqlResult<Vec<DocumentRecord>> {
    let query_str = match folder_id {
        Some(_) => "SELECT id, watched_folder_id, path, filename, extension, size, created_at, modified_at, indexed_at, status 
                    FROM documents WHERE watched_folder_id = ?1 AND status = 'ACTIVE' ORDER BY modified_at DESC",
        None => "SELECT id, watched_folder_id, path, filename, extension, size, created_at, modified_at, indexed_at, status 
                 FROM documents WHERE status = 'ACTIVE' ORDER BY modified_at DESC",
    };

    let mut stmt = conn.prepare(query_str)?;
    let row_mapper = |row: &rusqlite::Row| {
        Ok(DocumentRecord {
            id: row.get(0)?,
            watched_folder_id: row.get(1)?,
            path: row.get(2)?,
            filename: row.get(3)?,
            extension: row.get(4)?,
            size: row.get::<_, i64>(5)? as u64,
            created_at: row.get(6)?,
            modified_at: row.get(7)?,
            indexed_at: row.get(8)?,
            status: row.get(9)?,
        })
    };

    let docs_iter = match folder_id {
        Some(fid) => stmt.query_map(params![fid], row_mapper)?,
        None => stmt.query_map([], row_mapper)?,
    };

    let mut docs = Vec::new();
    for doc in docs_iter {
        docs.push(doc?);
    }
    Ok(docs)
}

pub fn query_documents_like(conn: &Connection, query: &str, folder_id: Option<i64>) -> SqlResult<Vec<DocumentRecord>> {
    let pattern = format!("%{}%", query.to_lowercase());
    
    let query_str = match folder_id {
        Some(_) => "SELECT id, watched_folder_id, path, filename, extension, size, created_at, modified_at, indexed_at, status 
                    FROM documents WHERE watched_folder_id = ?1 AND LOWER(filename) LIKE ?2 AND status = 'ACTIVE' ORDER BY modified_at DESC",
        None => "SELECT id, watched_folder_id, path, filename, extension, size, created_at, modified_at, indexed_at, status 
                 FROM documents WHERE LOWER(filename) LIKE ?1 AND status = 'ACTIVE' ORDER BY modified_at DESC",
    };

    let mut stmt = conn.prepare(query_str)?;
    let row_mapper = |row: &rusqlite::Row| {
        Ok(DocumentRecord {
            id: row.get(0)?,
            watched_folder_id: row.get(1)?,
            path: row.get(2)?,
            filename: row.get(3)?,
            extension: row.get(4)?,
            size: row.get::<_, i64>(5)? as u64,
            created_at: row.get(6)?,
            modified_at: row.get(7)?,
            indexed_at: row.get(8)?,
            status: row.get(9)?,
        })
    };

    let docs_iter = match folder_id {
        Some(fid) => stmt.query_map(params![fid, pattern], row_mapper)?,
        None => stmt.query_map(params![pattern], row_mapper)?,
    };

    let mut docs = Vec::new();
    for doc in docs_iter {
        docs.push(doc?);
    }
    Ok(docs)
}
