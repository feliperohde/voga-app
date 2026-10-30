//! Persistence for the search index: chunks + embeddings (SQLite BLOB) and the FTS5
//! keyword index.
//!
//! Vector search is brute-force cosine similarity in Rust, scoped to one investigation.
//! At personal scale (tens of thousands of chunks) this takes milliseconds and avoids a
//! native SQLite extension.

use chrono::Utc;
use serde::Serialize;
use sqlx::{Connection, Error as SqlxError, FromRow, SqlitePool};
use uuid::Uuid;

/// A chunk ready to be written, with its embedding when one was computed.
#[derive(Debug, Clone)]
pub struct NewChunk {
    pub text: String,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub embedding: Option<Vec<f32>>,
}

/// The document a set of chunks belongs to.
#[derive(Debug, Clone)]
pub struct DocumentRef {
    pub document_id: String,
    pub investigation_id: String,
    pub title: String,
    pub doc_date: String,
}

/// A chunk returned by retrieval, with what a citation needs.
#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ChunkHit {
    pub chunk_id: String,
    pub document_id: String,
    pub title: String,
    pub url: String,
    pub kind: String,
    pub site: Option<String>,
    pub doc_date: String,
    pub text: String,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    #[sqlx(skip)]
    pub score: f64,
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilters {
    /// Inclusive lower bound on the document date (YYYY-MM-DD prefix).
    pub date_from: Option<String>,
    /// Exclusive upper bound on the document date.
    pub date_to: Option<String>,
    /// article | page | video
    pub kind: Option<String>,
}

pub fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as f64, *y as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Turns free text into a safe FTS5 query: each word quoted, OR-ed together.
/// Returns `None` when the text has no searchable words.
pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .map(|t| t.trim_matches(|c| c == '-' || c == '_'))
        .filter(|t| t.chars().count() >= 2)
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// The filter part shared by both searches; binds the investigations (a JSON array), kind, date_from, date_to.
const FILTERS: &str = "c.investigation_id IN (SELECT value FROM json_each(?1)) AND (?2 IS NULL OR d.kind = ?2) AND (?3 IS NULL OR c.doc_date >= ?3) AND (?4 IS NULL OR c.doc_date < ?4)";

pub struct RagStore;

impl RagStore {
    /// Replaces all chunks of a document (rows + FTS entries) atomically.
    pub async fn replace_document_chunks(pool: &SqlitePool, doc: &DocumentRef, chunks: &[NewChunk], embedding_model: &str) -> Result<(), SqlxError> {
        let mut conn = pool.acquire().await?;
        let mut tx = conn.begin().await?;
        // The delete trigger removes their FTS rows
        sqlx::query("DELETE FROM rag_chunks WHERE document_id = ?").bind(&doc.document_id).execute(&mut *tx).await?;

        let now = Utc::now().to_rfc3339();
        for (index, chunk) in chunks.iter().enumerate() {
            let id = format!("chunk-{}", Uuid::new_v4());
            let (blob, model, dims) = match &chunk.embedding {
                Some(v) => (Some(encode_vector(v)), Some(embedding_model), Some(v.len() as i64)),
                None => (None, None, None),
            };
            sqlx::query(
                "INSERT INTO rag_chunks (id, investigation_id, document_id, chunk_index, text, start_time, end_time, doc_date, embedding, embedding_model, dims, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&doc.investigation_id)
            .bind(&doc.document_id)
            .bind(index as i64)
            .bind(&chunk.text)
            .bind(chunk.start_time)
            .bind(chunk.end_time)
            .bind(&doc.doc_date)
            .bind(blob)
            .bind(model)
            .bind(dims)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO rag_chunks_fts (chunk_id, title, text) VALUES (?, ?, ?)")
                .bind(&id)
                .bind(&doc.title)
                .bind(&chunk.text)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }

    /// Top-k chunks by cosine similarity among chunks embedded with `embedding_model`,
    /// ignoring chunks below `min_similarity` (so unrelated questions find nothing).
    pub async fn vector_search(
        pool: &SqlitePool,
        scope: &[String],
        query_vector: &[f32],
        embedding_model: &str,
        filters: &SearchFilters,
        k: usize,
        min_similarity: f64,
    ) -> Result<Vec<ChunkHit>, SqlxError> {
        let rows: Vec<(String, Vec<u8>)> = sqlx::query_as(&format!(
            "SELECT c.id, c.embedding FROM rag_chunks c JOIN documents d ON d.id = c.document_id
             WHERE {FILTERS} AND c.embedding IS NOT NULL AND c.embedding_model = ?5"
        ))
        .bind(serde_json::to_string(scope).unwrap_or_default())
        .bind(&filters.kind)
        .bind(&filters.date_from)
        .bind(&filters.date_to)
        .bind(embedding_model)
        .fetch_all(pool)
        .await?;
        let mut scored: Vec<(String, f64)> = rows
            .into_iter()
            .map(|(id, blob)| (id, cosine_similarity(query_vector, &decode_vector(&blob))))
            .filter(|(_, score)| *score >= min_similarity)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Self::hydrate(pool, scored).await
    }

    /// Top-k chunks by BM25 over chunk text and document title.
    pub async fn lexical_search(pool: &SqlitePool, scope: &[String], query: &str, filters: &SearchFilters, k: usize) -> Result<Vec<ChunkHit>, SqlxError> {
        let Some(match_query) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let rows: Vec<(String, f64)> = sqlx::query_as(&format!(
            "SELECT f.chunk_id, bm25(rag_chunks_fts) AS rank
             FROM rag_chunks_fts f JOIN rag_chunks c ON c.id = f.chunk_id JOIN documents d ON d.id = c.document_id
             WHERE {FILTERS} AND rag_chunks_fts MATCH ?5
             ORDER BY rank LIMIT ?6"
        ))
        .bind(serde_json::to_string(scope).unwrap_or_default())
        .bind(&filters.kind)
        .bind(&filters.date_from)
        .bind(&filters.date_to)
        .bind(&match_query)
        .bind(k as i64)
        .fetch_all(pool)
        .await?;
        // bm25() is lower-is-better; expose a positive higher-is-better score
        Self::hydrate(pool, rows.into_iter().map(|(id, rank)| (id, -rank)).collect()).await
    }

    /// Loads citation metadata for scored chunk ids, preserving their order.
    pub async fn hydrate(pool: &SqlitePool, scored: Vec<(String, f64)>) -> Result<Vec<ChunkHit>, SqlxError> {
        let mut hits = Vec::with_capacity(scored.len());
        for (id, score) in scored {
            let hit: Option<ChunkHit> = sqlx::query_as(
                "SELECT c.id AS chunk_id, c.document_id, d.title, d.url, d.kind, d.site, c.doc_date, c.text, c.start_time, c.end_time
                 FROM rag_chunks c JOIN documents d ON d.id = c.document_id WHERE c.id = ?",
            )
            .bind(&id)
            .fetch_optional(pool)
            .await?;
            if let Some(mut hit) = hit {
                hit.score = score;
                hits.push(hit);
            }
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_roundtrip_and_cosine() {
        let v = vec![0.5f32, -1.25, 3.0];
        assert_eq!(decode_vector(&encode_vector(&v)), v);
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-9);
        assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-9);
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn fts_query_quotes_terms_and_drops_noise() {
        assert_eq!(
            fts_query("O que a deputada disse sobre o PL-2630?").unwrap(),
            "\"que\" OR \"deputada\" OR \"disse\" OR \"sobre\" OR \"PL-2630\""
        );
        assert_eq!(fts_query("\"a\" * ( )"), None);
    }
}
