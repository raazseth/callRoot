use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
    /// "ACTIVE" | "MISSING" | "IGNORED"
    pub status: String,
}

pub fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn init_database(db_path: &Path) -> SqlResult<Connection> {
    if let Some(parent) = db_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("[db] could not create data dir {:?}: {e}", parent);
        }
    }

    let conn = Connection::open(db_path)?;

    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        ",
    )?;

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
        CREATE INDEX IF NOT EXISTS idx_documents_filename       ON documents(filename ASC);
        CREATE INDEX IF NOT EXISTS idx_documents_status        ON documents(status);

        DROP TABLE IF EXISTS app_settings;
        ",
    )?;

    Ok(conn)
}

pub fn add_folder(conn: &Connection, folder_path: &str) -> SqlResult<WatchedFolder> {
    let now = current_timestamp();
    conn.execute(
        "INSERT INTO watched_folders (path, created_at, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET updated_at = ?3",
        params![folder_path, now, now],
    )?;

    let mut stmt = conn
        .prepare("SELECT id, path, created_at, updated_at FROM watched_folders WHERE path = ?1")?;
    stmt.query_row(params![folder_path], |row| {
        Ok(WatchedFolder {
            id: row.get(0)?,
            path: row.get(1)?,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })
}

pub fn get_folders(conn: &Connection) -> SqlResult<Vec<WatchedFolder>> {
    let mut stmt = conn
        .prepare("SELECT id, path, created_at, updated_at FROM watched_folders ORDER BY id ASC")?;
    let rows = stmt.query_map([], |row| {
        Ok(WatchedFolder {
            id: row.get(0)?,
            path: row.get(1)?,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    })?;

    let mut folders = Vec::new();
    for f in rows {
        folders.push(f?);
    }
    Ok(folders)
}

pub fn remove_folder(conn: &Connection, folder_id: i64) -> SqlResult<()> {
    let affected = conn.execute(
        "DELETE FROM watched_folders WHERE id = ?1",
        params![folder_id],
    )?;
    if affected == 0 {
        eprintln!("[db] remove_folder: no folder found with id={folder_id}");
    }
    Ok(())
}

pub fn save_document(conn: &Connection, doc: &DocumentRecord) -> SqlResult<()> {
    conn.execute(
        "INSERT INTO documents
            (watched_folder_id, path, filename, extension, size, created_at, modified_at, indexed_at, status)
         VALUES
            (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(path) DO UPDATE SET
            filename    = ?3,
            extension   = ?4,
            size        = ?5,
            modified_at = ?7,
            indexed_at  = ?8,
            status      = ?9",
        params![
            doc.watched_folder_id,
            doc.path,
            doc.filename,
            doc.extension,
            doc.size as i64,
            doc.created_at,
            doc.modified_at,
            doc.indexed_at,
            doc.status,
        ],
    )?;
    Ok(())
}

pub fn get_active_paths_for_folder(
    conn: &Connection,
    folder_id: i64,
) -> SqlResult<HashSet<String>> {
    let mut stmt = conn
        .prepare("SELECT path FROM documents WHERE watched_folder_id = ?1 AND status = 'ACTIVE'")?;
    let rows = stmt.query_map(params![folder_id], |row| row.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for r in rows {
        set.insert(r?);
    }
    Ok(set)
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

fn map_document_row(row: &rusqlite::Row) -> rusqlite::Result<DocumentRecord> {
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
}

pub fn query_documents(
    conn: &Connection,
    folder_id: Option<i64>,
) -> SqlResult<Vec<DocumentRecord>> {
    let mut docs = Vec::new();
    match folder_id {
        Some(fid) => {
            let mut stmt = conn.prepare(
                "SELECT id, watched_folder_id, path, filename, extension, size,
                        created_at, modified_at, indexed_at, status
                 FROM documents
                 WHERE watched_folder_id = ?1 AND status = 'ACTIVE'
                 ORDER BY modified_at DESC",
            )?;
            for row in stmt.query_map(params![fid], map_document_row)? {
                docs.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, watched_folder_id, path, filename, extension, size,
                        created_at, modified_at, indexed_at, status
                 FROM documents
                 WHERE status = 'ACTIVE'
                 ORDER BY modified_at DESC",
            )?;
            for row in stmt.query_map([], map_document_row)? {
                docs.push(row?);
            }
        }
    }
    Ok(docs)
}

pub fn query_documents_like(
    conn: &Connection,
    query: &str,
    folder_id: Option<i64>,
) -> SqlResult<Vec<DocumentRecord>> {
    let escaped = query
        .to_lowercase()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");

    let mut docs = Vec::new();
    match folder_id {
        Some(fid) => {
            let mut stmt = conn.prepare(
                "SELECT id, watched_folder_id, path, filename, extension, size,
                        created_at, modified_at, indexed_at, status
                 FROM documents
                 WHERE watched_folder_id = ?1
                   AND LOWER(filename) LIKE ?2 ESCAPE '\\'
                   AND status = 'ACTIVE'
                 ORDER BY modified_at DESC",
            )?;
            for row in stmt.query_map(params![fid, pattern], map_document_row)? {
                docs.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT id, watched_folder_id, path, filename, extension, size,
                        created_at, modified_at, indexed_at, status
                 FROM documents
                 WHERE LOWER(filename) LIKE ?1 ESCAPE '\\'
                   AND status = 'ACTIVE'
                 ORDER BY modified_at DESC",
            )?;
            for row in stmt.query_map(params![pattern], map_document_row)? {
                docs.push(row?);
            }
        }
    }
    Ok(docs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
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

    fn make_doc(folder_id: i64, path: &str) -> DocumentRecord {
        DocumentRecord {
            id: 0,
            watched_folder_id: folder_id,
            path: path.to_string(),
            filename: "file.pdf".to_string(),
            extension: "pdf".to_string(),
            size: 512,
            created_at: 100,
            modified_at: 200,
            indexed_at: 300,
            status: "ACTIVE".to_string(),
        }
    }

    #[test]
    fn test_save_document_inserts() {
        let conn = test_conn();
        let fid = insert_folder(&conn, "/tmp/f");
        let doc = make_doc(fid, "/tmp/f/a.pdf");
        save_document(&conn, &doc).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_save_document_upsert_same_path() {
        let conn = test_conn();
        let fid = insert_folder(&conn, "/tmp/f");
        let mut doc = make_doc(fid, "/tmp/f/a.pdf");
        save_document(&conn, &doc).unwrap();
        doc.size = 9999;
        doc.modified_at = 999;
        save_document(&conn, &doc).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let (size, modified): (i64, i64) = conn
            .query_row(
                "SELECT size, modified_at FROM documents WHERE path = '/tmp/f/a.pdf'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(size, 9999);
        assert_eq!(modified, 999);
    }

    #[test]
    fn test_mark_document_status_missing() {
        let conn = test_conn();
        let fid = insert_folder(&conn, "/tmp/f");
        save_document(&conn, &make_doc(fid, "/tmp/f/a.pdf")).unwrap();
        mark_document_status(&conn, "/tmp/f/a.pdf", "MISSING").unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM documents WHERE path = '/tmp/f/a.pdf'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "MISSING");
    }

    #[test]
    fn test_get_active_paths_excludes_missing() {
        let conn = test_conn();
        let fid = insert_folder(&conn, "/tmp/f");
        save_document(&conn, &make_doc(fid, "/tmp/f/a.pdf")).unwrap();
        save_document(&conn, &make_doc(fid, "/tmp/f/b.pdf")).unwrap();
        mark_document_status(&conn, "/tmp/f/b.pdf", "MISSING").unwrap();
        let active = get_active_paths_for_folder(&conn, fid).unwrap();
        assert!(active.contains("/tmp/f/a.pdf"));
        assert!(!active.contains("/tmp/f/b.pdf"));
    }

    #[test]
    fn test_remove_folder_cascades_documents() {
        let conn = test_conn();
        let fid = insert_folder(&conn, "/tmp/f");
        save_document(&conn, &make_doc(fid, "/tmp/f/a.pdf")).unwrap();
        remove_folder(&conn, fid).unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_query_documents_like_case_insensitive() {
        let conn = test_conn();
        let fid = insert_folder(&conn, "/tmp/f");
        let mut doc = make_doc(fid, "/tmp/f/Report_2024.pdf");
        doc.filename = "Report_2024.pdf".to_string();
        save_document(&conn, &doc).unwrap();
        let results = query_documents_like(&conn, "report", Some(fid)).unwrap();
        assert_eq!(results.len(), 1);
        let results_upper = query_documents_like(&conn, "REPORT", Some(fid)).unwrap();
        assert_eq!(results_upper.len(), 1);
    }

    #[test]
    fn test_query_documents_excludes_missing() {
        let conn = test_conn();
        let fid = insert_folder(&conn, "/tmp/f");
        save_document(&conn, &make_doc(fid, "/tmp/f/a.pdf")).unwrap();
        save_document(&conn, &make_doc(fid, "/tmp/f/b.pdf")).unwrap();
        mark_document_status(&conn, "/tmp/f/b.pdf", "MISSING").unwrap();
        let docs = query_documents(&conn, Some(fid)).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].path, "/tmp/f/a.pdf");
    }

    #[test]
    fn test_add_folder_idempotent() {
        let conn = test_conn();
        let f1 = add_folder(&conn, "/tmp/f").unwrap();
        let f2 = add_folder(&conn, "/tmp/f").unwrap();
        assert_eq!(f1.id, f2.id);
    }

    #[test]
    fn test_query_like_escapes_special_characters() {
        let conn = test_conn();
        let fid = insert_folder(&conn, "/tmp/f");
        let mut doc = make_doc(fid, "/tmp/f/100%_done.pdf");
        doc.filename = "100%_done.pdf".to_string();
        save_document(&conn, &doc).unwrap();
        let results = query_documents_like(&conn, "100%", Some(fid)).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].filename, "100%_done.pdf");
    }
}
