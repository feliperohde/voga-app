//! Turns a collected document into searchable chunks: split, embed (when the embedding model
//! is available),
//! store. A document whose embedding failed is still found by keywords and gets its
//! vectors on a later pass (`embed_missing`).

use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;
use tokio::sync::Mutex;

use super::chunker::{chunk_markdown, chunk_transcript, ChunkOptions, Segment};
use super::embeddings::{BuiltinEmbedder, EmbeddingProvider, OllamaEmbedder, OpenAiEmbedder};
use super::store::{encode_vector, DocumentRef, NewChunk, RagStore};
use crate::collect::youtube::TimedText;

/// Characters per chunk of an article.
const MAX_CHUNK_CHARS: usize = 1500;
/// One indexing at a time: the model serves one batch at a time anyway.
static INDEX_LOCK: Mutex<()> = Mutex::const_new(());

/// The embedding model of Settings › AI.
pub async fn embedder(pool: &SqlitePool) -> Result<Box<dyn EmbeddingProvider>> {
    let settings = crate::settings::load(pool).await?;
    Ok(match settings.embedding_provider.as_str() {
        "ollama" => Box::new(OllamaEmbedder::new(settings.ollama_endpoint.as_deref(), &settings.embedding_model)),
        provider @ ("openai" | "gemini" | "custom-openai") => {
            let base = crate::providers::openai_base(provider, settings.custom_endpoint.as_deref())
                .ok_or_else(|| anyhow::anyhow!("Set the address of the OpenAI-compatible server in Settings › AI"))?;
            let key = crate::settings::api_key(pool, provider).await?.unwrap_or_default();
            Box::new(OpenAiEmbedder::new(provider, &base, &key, &settings.embedding_model))
        }
        _ => Box::new(BuiltinEmbedder::new(&settings.embedding_model)),
    })
}

#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: String,
    investigation_id: String,
    title: String,
    kind: String,
    site: Option<String>,
    text: String,
    segments: Option<String>,
    doc_date: String,
}

/// The context line put in front of each passage before embedding, so a passage that
/// says "ela votou contra" still carries where and when it was said.
fn embedding_text(title: &str, site: Option<&str>, date: &str, text: &str) -> String {
    let date = date.get(..10).unwrap_or(date);
    let site = site.map(|s| format!(" | {s}")).unwrap_or_default();
    format!("[{title}{site} | {date}]\n{text}")
}

fn drafts(doc: &DocumentRow) -> Vec<NewChunk> {
    let timed: Option<Vec<TimedText>> = doc.segments.as_deref().and_then(|json| serde_json::from_str(json).ok());
    match timed {
        Some(timed) if !timed.is_empty() => {
            let segments: Vec<Segment> = timed
                .into_iter()
                .map(|t| Segment { text: t.text, start_time: Some(t.start), end_time: Some(t.end), speaker: None })
                .collect();
            chunk_transcript(&segments, ChunkOptions::default())
                .into_iter()
                .map(|c| NewChunk { text: c.text, start_time: c.start_time, end_time: c.end_time, embedding: None })
                .collect()
        }
        _ => chunk_markdown(&doc.text, MAX_CHUNK_CHARS)
            .into_iter()
            .map(|text| NewChunk { text, start_time: None, end_time: None, embedding: None })
            .collect(),
    }
}

/// Indexes one document. Ok(true) when every chunk got its vector.
pub async fn index_document(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, document_id: &str) -> Result<bool> {
    let _guard = INDEX_LOCK.lock().await;
    let doc: DocumentRow = sqlx::query_as(
        "SELECT id, investigation_id, title, kind, site, text, segments, COALESCE(published_at, fetched_at) AS doc_date
         FROM documents WHERE id = ?",
    )
    .bind(document_id)
    .fetch_one(pool)
    .await?;

    let mut chunks = drafts(&doc);
    let inputs: Vec<String> = chunks.iter().map(|c| embedding_text(&doc.title, doc.site.as_deref(), &doc.doc_date, &c.text)).collect();
    let embed_error = match embedder.embed(&inputs).await {
        Ok(vectors) => {
            for (chunk, vector) in chunks.iter_mut().zip(vectors) {
                chunk.embedding = Some(vector);
            }
            None
        }
        Err(e) => Some(e.to_string()),
    };
    let reference = DocumentRef { document_id: doc.id.clone(), investigation_id: doc.investigation_id, title: doc.title, doc_date: doc.doc_date };
    RagStore::replace_document_chunks(pool, &reference, &chunks, embedder.model_id()).await?;
    sqlx::query("UPDATE documents SET indexed_at = ?, index_error = ?, relevance = NULL WHERE id = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(&embed_error)
        .bind(&doc.id)
        .execute(pool)
        .await?;
    if let Some(e) = &embed_error {
        log::warn!("Index: {} ({}) indexed by keywords only: {e}", doc.id, doc.kind);
    }
    Ok(embed_error.is_none())
}

/// Gives a vector to chunks that have none (or one from another model), a batch at a time.
/// Returns how many were embedded.
pub async fn embed_missing(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, limit: i64) -> Result<usize> {
    let _guard = INDEX_LOCK.lock().await;
    let rows: Vec<(String, String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT c.id, c.text, d.title, d.site, c.doc_date FROM rag_chunks c JOIN documents d ON d.id = c.document_id
         WHERE c.embedding IS NULL OR c.embedding_model != ? LIMIT ?",
    )
    .bind(embedder.model_id())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let inputs: Vec<String> = rows.iter().map(|(_, text, title, site, date)| embedding_text(title, site.as_deref(), date, text)).collect();
    let vectors = embedder.embed(&inputs).await?;
    for ((id, ..), vector) in rows.iter().zip(&vectors) {
        sqlx::query("UPDATE rag_chunks SET embedding = ?, embedding_model = ?, dims = ? WHERE id = ?")
            .bind(encode_vector(vector))
            .bind(embedder.model_id())
            .bind(vector.len() as i64)
            .bind(id)
            .execute(pool)
            .await?;
        // New vectors: the Feed scores the document again
        sqlx::query("UPDATE documents SET relevance = NULL WHERE id = (SELECT document_id FROM rag_chunks WHERE id = ?)").bind(id).execute(pool).await?;
    }
    sqlx::query("UPDATE documents SET index_error = NULL WHERE index_error IS NOT NULL AND id NOT IN (SELECT document_id FROM rag_chunks WHERE embedding IS NULL)")
        .execute(pool)
        .await?;
    Ok(vectors.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{save_document, NewDocument};
    use crate::rag::retriever::hybrid_search;
    use crate::rag::store::SearchFilters;
    use async_trait::async_trait;

    /// One dimension per keyword, plus one for texts with none.
    struct KeywordEmbedder;

    #[async_trait]
    impl EmbeddingProvider for KeywordEmbedder {
        fn model_id(&self) -> &str {
            "kw"
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            const VOCAB: &[&str] = &["reforma", "escola", "saúde"];
            Ok(texts
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    let mut v: Vec<f32> = VOCAB.iter().map(|w| t.matches(w).count() as f32).collect();
                    v.push(if v.iter().all(|x| *x == 0.0) { 1.0 } else { 0.0 });
                    v
                })
                .collect())
        }
    }

    async fn pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Teste', '2026-10-15')").execute(&pool).await.unwrap();
        pool
    }

    fn doc<'a>(url: &'a str, title: &'a str, text: &'a str, segments: Option<String>) -> NewDocument<'a> {
        NewDocument {
            investigation_id: "inv",
            source_id: None,
            url,
            kind: if segments.is_some() { "video" } else { "article" },
            title,
            author: None,
            site: Some("Jornal"),
            published_at: Some("2023-07-06"),
            lang: None,
            text,
            segments,
            note: None,
        }
    }

    #[tokio::test]
    async fn documents_are_indexed_found_and_deleted_with_their_index() {
        let pool = pool().await;
        let article = save_document(&pool, doc("https://j.example/a", "Votação da reforma", "A deputada votou contra a reforma tributária.", None))
            .await
            .unwrap()
            .unwrap();
        // The same text again is nothing new
        assert!(save_document(&pool, doc("https://j.example/a", "Votação da reforma", "A deputada votou contra a reforma tributária.", None)).await.unwrap().is_none());
        let segments = serde_json::to_string(&[
            TimedText { start: 12.0, end: 15.0, text: "Vou construir uma escola em cada bairro".into() },
            TimedText { start: 15.0, end: 18.0, text: "e cuidar da saúde".into() },
        ])
        .unwrap();
        let video = save_document(&pool, doc("https://www.youtube.com/watch?v=abcdefghijk", "Comício", "x", Some(segments))).await.unwrap().unwrap();
        assert!(index_document(&pool, &KeywordEmbedder, &article).await.unwrap());
        assert!(index_document(&pool, &KeywordEmbedder, &video).await.unwrap());

        let found = hybrid_search(&pool, &KeywordEmbedder, &["inv".to_string()], "escola", &SearchFilters::default(), 5).await.unwrap();
        assert_eq!(found.results[0].hit.document_id, video);
        assert_eq!(found.results[0].hit.start_time, Some(12.0));
        assert_eq!(found.results[0].sources, vec!["vector", "lexical"]);

        let only_videos = SearchFilters { kind: Some("video".into()), ..Default::default() };
        let found = hybrid_search(&pool, &KeywordEmbedder, &["inv".to_string()], "reforma", &only_videos, 5).await.unwrap();
        assert!(found.results.iter().all(|r| r.hit.kind == "video"));
        let before_2023 = SearchFilters { date_to: Some("2023-01-01".into()), ..Default::default() };
        assert!(hybrid_search(&pool, &KeywordEmbedder, &["inv".to_string()], "reforma", &before_2023, 5).await.unwrap().results.is_empty());

        // Another investigation's school: found only when both are searched together
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('other', 'Outra', '2026-10-15')").execute(&pool).await.unwrap();
        let other = save_document(&pool, NewDocument { investigation_id: "other", ..doc("https://j.example/b", "Escola nova", "A escola do bairro foi inaugurada.", None) }).await.unwrap().unwrap();
        assert!(index_document(&pool, &KeywordEmbedder, &other).await.unwrap());
        let only = hybrid_search(&pool, &KeywordEmbedder, &["inv".to_string()], "escola", &SearchFilters::default(), 5).await.unwrap();
        assert!(only.results.iter().all(|r| r.hit.document_id != other));
        let both = hybrid_search(&pool, &KeywordEmbedder, &["inv".to_string(), "other".to_string()], "escola", &SearchFilters::default(), 5).await.unwrap();
        assert!(both.results.iter().any(|r| r.hit.document_id == other) && both.results.iter().any(|r| r.hit.document_id == video));

        // Deleting the document takes its chunks and their keyword entries
        sqlx::query("DELETE FROM documents WHERE id = ?").bind(&article).execute(&pool).await.unwrap();
        let (fts,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rag_chunks_fts WHERE text MATCH 'reforma'").fetch_one(&pool).await.unwrap();
        assert_eq!(fts, 0);
        // And the investigation takes everything
        sqlx::query("DELETE FROM investigations WHERE id IN ('inv', 'other')").execute(&pool).await.unwrap();
        let (chunks,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rag_chunks_fts").fetch_one(&pool).await.unwrap();
        assert_eq!(chunks, 0);
    }

    #[tokio::test]
    async fn chunks_without_vectors_get_them_later() {
        struct Down;
        #[async_trait]
        impl EmbeddingProvider for Down {
            fn model_id(&self) -> &str {
                "kw"
            }
            async fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>> {
                anyhow::bail!("Ollama is down")
            }
        }
        let pool = pool().await;
        let id = save_document(&pool, doc("https://j.example/b", "Saúde", "Promessa sobre saúde.", None)).await.unwrap().unwrap();
        assert!(!index_document(&pool, &Down, &id).await.unwrap());
        let (error,): (Option<String>,) = sqlx::query_as("SELECT index_error FROM documents WHERE id = ?").bind(&id).fetch_one(&pool).await.unwrap();
        assert!(error.unwrap().contains("down"));
        assert_eq!(embed_missing(&pool, &KeywordEmbedder, 10).await.unwrap(), 1);
        let (error,): (Option<String>,) = sqlx::query_as("SELECT index_error FROM documents WHERE id = ?").bind(&id).fetch_one(&pool).await.unwrap();
        assert_eq!(error, None);
    }
}
