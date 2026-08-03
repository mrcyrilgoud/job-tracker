use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::models::{Document, DocumentListItem, JobDocument};
use crate::util::{create_id, extension_for_mime, now_iso};

const MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

pub fn validate_document_upload(filename: &str, mime_type: &str, size: usize) -> AppResult<()> {
    if size > MAX_DOCUMENT_BYTES {
        return Err(AppError::from("File exceeds the 10MB size limit"));
    }
    let allowed = match mime_type {
        "application/pdf" => &[".pdf"][..],
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => &[".docx"][..],
        "text/plain" => &[".txt"][..],
        _ => {
            return Err(AppError::from(
                "Only PDF, DOCX, and plain text files are allowed",
            ))
        }
    };
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();
    if !allowed.contains(&ext.as_str()) {
        return Err(AppError::from(format!(
            "File extension {} does not match MIME type {mime_type}",
            if ext.is_empty() { "(none)" } else { &ext }
        )));
    }
    Ok(())
}

pub fn import_document(
    conn: &Connection,
    documents_dir: &Path,
    original_filename: &str,
    mime_type: &str,
    buffer: &[u8],
) -> AppResult<Document> {
    validate_document_upload(original_filename, mime_type, buffer.len())?;
    let checksum = hex::encode(Sha256::digest(buffer));

    if let Some(existing) = conn
        .query_row(
            "SELECT id, original_filename, stored_filename, mime_type, checksum, size_bytes, imported_at FROM documents WHERE checksum = ?1",
            params![checksum],
            map_document,
        )
        .optional()
        .map_err(map_sqlite)?
    {
        return Ok(existing);
    }

    let id = create_id();
    let ext = extension_for_mime(mime_type, original_filename);
    let stored_filename = format!("{id}{ext}");
    let tmp_dir = documents_dir.join(".tmp");
    fs::create_dir_all(&tmp_dir)?;
    fs::create_dir_all(documents_dir)?;
    let tmp_path = tmp_dir.join(format!("{id}{ext}"));
    let _dest = documents_dir.join(&stored_filename);

    fs::write(&tmp_path, buffer)?;

    let row = Document {
        id: id.clone(),
        original_filename: original_filename.to_string(),
        stored_filename,
        mime_type: mime_type.to_string(),
        checksum,
        size_bytes: buffer.len() as i64,
        imported_at: now_iso(),
    };

    let insert_result = conn.execute(
        "INSERT INTO documents (id, original_filename, stored_filename, mime_type, checksum, size_bytes, imported_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            row.id,
            row.original_filename,
            row.stored_filename,
            row.mime_type,
            row.checksum,
            row.size_bytes,
            row.imported_at
        ],
    );

    if let Err(err) = insert_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(map_sqlite(err));
    }

    // Leave bytes in `.tmp` until the surrounding transaction commits; callers that
    // are not transactional may finalize immediately via `finalize_staged_document`.
    Ok(row)
}

/// Move a staged document from `.tmp` into its content-addressed final path.
/// On failure, deletes the DB row so the library does not advertise a missing file.
pub fn finalize_staged_document(
    conn: &Connection,
    documents_dir: &Path,
    document: &Document,
) -> AppResult<()> {
    let tmp_path = documents_dir.join(".tmp").join(&document.stored_filename);
    let dest = documents_dir.join(&document.stored_filename);
    if dest.exists() {
        let _ = fs::remove_file(&tmp_path);
        return Ok(());
    }
    if !tmp_path.exists() {
        // Deduplicated import — original file already in place.
        return Ok(());
    }
    if let Err(err) = fs::rename(&tmp_path, &dest) {
        let _ = conn.execute(
            "DELETE FROM job_documents WHERE document_id = ?1",
            params![document.id],
        );
        let _ = conn.execute("DELETE FROM documents WHERE id = ?1", params![document.id]);
        let _ = fs::remove_file(&tmp_path);
        return Err(AppError::from(format!(
            "Failed to finalize document file: {err}"
        )));
    }
    Ok(())
}

pub fn discard_staged_document(documents_dir: &Path, document: &Document) {
    let tmp_path = documents_dir.join(".tmp").join(&document.stored_filename);
    let _ = fs::remove_file(tmp_path);
}

fn map_document(row: &rusqlite::Row<'_>) -> rusqlite::Result<Document> {
    Ok(Document {
        id: row.get(0)?,
        original_filename: row.get(1)?,
        stored_filename: row.get(2)?,
        mime_type: row.get(3)?,
        checksum: row.get(4)?,
        size_bytes: row.get(5)?,
        imported_at: row.get(6)?,
    })
}

pub fn attach_document_to_job(
    conn: &Connection,
    job_id: &str,
    document_id: &str,
    kind: &str,
) -> AppResult<JobDocument> {
    let exists: Option<String> = conn
        .query_row(
            "SELECT id FROM documents WHERE id = ?1",
            params![document_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite)?;
    if exists.is_none() {
        return Err(AppError::from("Document not found"));
    }
    let row = JobDocument {
        id: create_id(),
        job_id: job_id.to_string(),
        document_id: document_id.to_string(),
        kind: kind.to_string(),
        used_at: now_iso(),
    };
    conn.execute(
        "INSERT INTO job_documents (id, job_id, document_id, kind, used_at) VALUES (?1,?2,?3,?4,?5)",
        params![row.id, row.job_id, row.document_id, row.kind, row.used_at],
    )
    .map_err(map_sqlite)?;
    Ok(row)
}

pub fn detach_document(conn: &Connection, attachment_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM job_documents WHERE id = ?1",
        params![attachment_id],
    )
    .map_err(map_sqlite)?;
    Ok(())
}

pub fn get_document_file_path(
    conn: &Connection,
    documents_dir: &Path,
    document_id: &str,
) -> AppResult<(Document, PathBuf)> {
    let document = conn
        .query_row(
            "SELECT id, original_filename, stored_filename, mime_type, checksum, size_bytes, imported_at FROM documents WHERE id = ?1",
            params![document_id],
            map_document,
        )
        .optional()
        .map_err(map_sqlite)?
        .ok_or_else(|| AppError::from("Document not found"))?;
    let file_path = documents_dir.join(&document.stored_filename);
    if !file_path.exists() {
        return Err(AppError::from("Document file is missing from disk"));
    }
    Ok((document, file_path))
}

pub fn list_documents(conn: &Connection) -> AppResult<Vec<DocumentListItem>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT
              d.id,
              d.original_filename,
              d.stored_filename,
              d.mime_type,
              d.checksum,
              d.size_bytes,
              d.imported_at,
              jd.kind,
              jd.used_at,
              c.name,
              j.title
            FROM documents d
            LEFT JOIN job_documents jd ON jd.document_id = d.id
            LEFT JOIN jobs j ON j.id = jd.job_id
            LEFT JOIN companies c ON c.id = j.company_id
            ORDER BY d.imported_at DESC, jd.used_at DESC
            "#,
        )
        .map_err(map_sqlite)?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                Document {
                    id: row.get(0)?,
                    original_filename: row.get(1)?,
                    stored_filename: row.get(2)?,
                    mime_type: row.get(3)?,
                    checksum: row.get(4)?,
                    size_bytes: row.get(5)?,
                    imported_at: row.get(6)?,
                },
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        })
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, DocumentListItem> = HashMap::new();

    for (document, kind, _used_at, company_name, job_title) in rows {
        let id = document.id.clone();
        if !by_id.contains_key(&id) {
            order.push(id.clone());
            by_id.insert(
                id.clone(),
                DocumentListItem {
                    document,
                    kinds: Vec::new(),
                    used_by: Vec::new(),
                },
            );
        }

        let entry = by_id.get_mut(&id).expect("document just inserted");
        if let Some(kind) = kind {
            if !entry.kinds.contains(&kind) {
                entry.kinds.push(kind);
            }
        }
        if let (Some(company), Some(title)) = (company_name, job_title) {
            let label = format!("{company} — {title}");
            if !entry.used_by.contains(&label) {
                entry.used_by.push(label);
            }
        }
    }

    Ok(order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::migrate;
    use crate::util::now_iso;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        migrate(&connection).unwrap();
        connection
    }

    fn seed_company_job(
        conn: &Connection,
        company: &str,
        title: &str,
        url: &str,
    ) -> (String, String) {
        let company_id = create_id();
        let job_id = create_id();
        let now = now_iso();
        conn.execute(
            "INSERT INTO companies (id, name, careers_url, created_at, updated_at) VALUES (?1,?2,NULL,?3,?3)",
            params![company_id, company, now],
        )
        .unwrap();
        conn.execute(
            r#"INSERT INTO jobs (
                id, company_id, title, url, canonical_url, source_external_id, status, applied_at,
                posting_state, last_checked_at, last_check_result, source, notes, location,
                is_new_from_watch, missing_from_sync_count, created_at, updated_at
            ) VALUES (?1,?2,?3,?4,?4,NULL,'wishlist',NULL,'unknown',NULL,NULL,'manual',NULL,NULL,0,0,?5,?5)"#,
            params![job_id, company_id, title, url, now],
        )
        .unwrap();
        (company_id, job_id)
    }

    fn seed_document(conn: &Connection, filename: &str, imported_at: &str) -> String {
        let id = create_id();
        conn.execute(
            "INSERT INTO documents (id, original_filename, stored_filename, mime_type, checksum, size_bytes, imported_at) VALUES (?1,?2,?3,'application/pdf',?4,10,?5)",
            params![id, filename, format!("{id}.pdf"), format!("checksum-{id}"), imported_at],
        )
        .unwrap();
        id
    }

    #[test]
    fn accepts_pdf() {
        assert!(validate_document_upload("resume.pdf", "application/pdf", 1024).is_ok());
    }

    #[test]
    fn rejects_oversize() {
        let err = validate_document_upload("resume.pdf", "application/pdf", 20 * 1024 * 1024)
            .unwrap_err();
        assert!(err.to_string().contains("10MB"));
    }

    #[test]
    fn rejects_mismatch() {
        let err = validate_document_upload("resume.txt", "application/pdf", 1024).unwrap_err();
        assert!(err.to_string().contains("extension"));
    }

    #[test]
    fn list_documents_returns_flattened_shape_with_attachments() {
        let conn = test_connection();
        let unattached_id = seed_document(&conn, "unused.pdf", "2026-01-02T00:00:00Z");
        let attached_id = seed_document(&conn, "resume.pdf", "2026-01-03T00:00:00Z");
        let (_, job_a) = seed_company_job(&conn, "Acme", "Engineer", "https://example.com/a");
        let (_, job_b) = seed_company_job(&conn, "Beta", "Designer", "https://example.com/b");

        attach_document_to_job(&conn, &job_a, &attached_id, "resume").unwrap();
        attach_document_to_job(&conn, &job_b, &attached_id, "cover_letter").unwrap();

        let listed = list_documents(&conn).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].document.id, attached_id);
        assert_eq!(listed[0].document.original_filename, "resume.pdf");
        let mut kinds = listed[0].kinds.clone();
        kinds.sort();
        assert_eq!(kinds, vec!["cover_letter".to_string(), "resume".to_string()]);
        let mut used_by = listed[0].used_by.clone();
        used_by.sort();
        assert_eq!(
            used_by,
            vec![
                "Acme — Engineer".to_string(),
                "Beta — Designer".to_string()
            ]
        );
        assert_eq!(listed[1].document.id, unattached_id);
        assert!(listed[1].kinds.is_empty());
        assert!(listed[1].used_by.is_empty());

        let json = serde_json::to_value(&listed[0]).unwrap();
        assert_eq!(json["id"], attached_id);
        assert_eq!(json["originalFilename"], "resume.pdf");
        assert!(json.get("document").is_none());
        assert_eq!(json["kinds"][0], "resume");
        assert_eq!(json["usedBy"][0], "Acme — Engineer");
    }
}
