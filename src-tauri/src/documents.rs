use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::{map_sqlite, AppError, AppResult};
use crate::models::{Document, JobDocument};
use crate::util::{create_id, extension_for_mime, now_iso};

const MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

pub fn validate_document_upload(filename: &str, mime_type: &str, size: usize) -> AppResult<()> {
    if size > MAX_DOCUMENT_BYTES {
        return Err(AppError::from("File exceeds the 10MB size limit"));
    }
    let allowed = match mime_type {
        "application/pdf" => &[".pdf"][..],
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            &[".docx"][..]
        }
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
    fs::create_dir_all(documents_dir)?;
    let dest = documents_dir.join(&stored_filename);
    fs::write(&dest, buffer)?;

    let row = Document {
        id,
        original_filename: original_filename.to_string(),
        stored_filename,
        mime_type: mime_type.to_string(),
        checksum,
        size_bytes: buffer.len() as i64,
        imported_at: now_iso(),
    };
    conn.execute(
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
    )
    .map_err(map_sqlite)?;
    Ok(row)
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

pub fn list_documents(conn: &Connection) -> AppResult<Vec<serde_json::Value>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, original_filename, stored_filename, mime_type, checksum, size_bytes, imported_at FROM documents ORDER BY imported_at DESC",
        )
        .map_err(map_sqlite)?;
    let docs = stmt
        .query_map([], map_document)
        .map_err(map_sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_sqlite)?;

    let mut out = Vec::new();
    for doc in docs {
        let usage: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM job_documents WHERE document_id = ?1",
                params![doc.id],
                |r| r.get(0),
            )
            .map_err(map_sqlite)?;
        out.push(serde_json::json!({
            "document": doc,
            "usageCount": usage
        }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let err =
            validate_document_upload("resume.txt", "application/pdf", 1024).unwrap_err();
        assert!(err.to_string().contains("extension"));
    }
}
