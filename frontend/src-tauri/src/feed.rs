//! The Feed: what every investigation collected to be read (news, pages, videos, posts), newest
//! or most relevant first, with read, starred and liked/disliked marks.
//! Relevance is how close a document is to its investigation's interest (its name, description
//! and targets), moved toward what was liked and away from what was disliked (Rocchio), from the
//! vectors the index already has: no AI call but one embedding per investigation.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::investigations::Page;
use crate::rag::embeddings::EmbeddingProvider;
use crate::rag::store::{cosine_similarity, decode_vector};
use crate::state::AppState;

/// What the Feed shows: what people read, not the official records.
const FEED_KINDS: &str = "'article', 'page', 'video', 'post'";
/// Documents scored per worker step.
const SCORE_BATCH: i64 = 200;
/// How far liked and disliked documents move the interest.
const LIKED: f32 = 0.75;
const DISLIKED: f32 = 0.25;

fn normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        v.to_vec()
    } else {
        v.iter().map(|x| x / norm).collect()
    }
}

/// The mean direction of `vectors` (those of another size are left out).
fn mean(vectors: &[Vec<f32>], dims: usize) -> Option<Vec<f32>> {
    let same: Vec<&Vec<f32>> = vectors.iter().filter(|v| v.len() == dims).collect();
    if same.is_empty() {
        return None;
    }
    let mut sum = vec![0.0; dims];
    for v in &same {
        for (s, x) in sum.iter_mut().zip(normalized(v)) {
            *s += x;
        }
    }
    Some(sum.into_iter().map(|s| s / same.len() as f32).collect())
}

/// The interest moved toward the documents liked and away from the ones disliked.
pub fn profile(interest: &[f32], liked: &[Vec<f32>], disliked: &[Vec<f32>]) -> Vec<f32> {
    let mut profile = normalized(interest);
    if let Some(m) = mean(liked, profile.len()) {
        profile.iter_mut().zip(m).for_each(|(p, x)| *p += LIKED * x);
    }
    if let Some(m) = mean(disliked, profile.len()) {
        profile.iter_mut().zip(m).for_each(|(p, x)| *p -= DISLIKED * x);
    }
    profile
}

/// A document's relevance: how close its passage nearest to the interest is.
pub fn score(passages: &[Vec<f32>], profile: &[f32]) -> f64 {
    passages.iter().map(|p| cosine_similarity(p, profile)).fold(-1.0, f64::max)
}

async fn passages(pool: &SqlitePool, document_id: &str, model: &str) -> Result<Vec<Vec<f32>>> {
    let rows: Vec<(Vec<u8>,)> = sqlx::query_as("SELECT embedding FROM rag_chunks WHERE document_id = ? AND embedding_model = ? AND embedding IS NOT NULL")
        .bind(document_id)
        .bind(model)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(bytes,)| decode_vector(&bytes)).collect())
}

/// The documents of an investigation marked `feedback`, each as the mean of its passages.
async fn marked(pool: &SqlitePool, investigation_id: &str, feedback: i64, model: &str) -> Result<Vec<Vec<f32>>> {
    let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM documents WHERE investigation_id = ? AND feedback = ?").bind(investigation_id).bind(feedback).fetch_all(pool).await?;
    let mut vectors = Vec::new();
    for (id,) in ids {
        let passages = passages(pool, &id, model).await?;
        if let Some(dims) = passages.first().map(Vec::len) {
            vectors.extend(mean(&passages, dims));
        }
    }
    Ok(vectors)
}

/// What the investigation is about, as the embedding model sees it, moved by the feedback given.
pub(crate) async fn interest(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, investigation_id: &str) -> Result<Vec<f32>> {
    let (name, description): (String, String) = sqlx::query_as("SELECT name, description FROM investigations WHERE id = ?").bind(investigation_id).fetch_one(pool).await?;
    let targets: Vec<(String, String)> = sqlx::query_as("SELECT name, aliases FROM targets WHERE investigation_id = ?").bind(investigation_id).fetch_all(pool).await?;
    let mut text = vec![name, description];
    for (name, aliases) in targets {
        text.push(name);
        text.extend(serde_json::from_str::<Vec<String>>(&aliases).unwrap_or_default());
    }
    let text = text.into_iter().filter(|t| !t.trim().is_empty()).collect::<Vec<_>>().join("\n");
    let query = embedder.embed(&[text]).await?.into_iter().next().unwrap_or_default();
    let model = embedder.model_id();
    Ok(profile(&query, &marked(pool, investigation_id, 1, model).await?, &marked(pool, investigation_id, -1, model).await?))
}

/// Scores documents that have every passage embedded with the current model and no relevance
/// yet (new ones, re-indexed ones, or all of an investigation after `reset`). Returns how many.
pub async fn score_step(pool: &SqlitePool, embedder: &dyn EmbeddingProvider) -> Result<usize> {
    let model = embedder.model_id();
    let docs: Vec<(String, String)> = sqlx::query_as(&format!(
        "SELECT d.id, d.investigation_id FROM documents d WHERE d.relevance IS NULL AND d.kind IN ({FEED_KINDS})
           AND EXISTS (SELECT 1 FROM rag_chunks c WHERE c.document_id = d.id)
           AND NOT EXISTS (SELECT 1 FROM rag_chunks c WHERE c.document_id = d.id AND (c.embedding IS NULL OR c.embedding_model != ?1))
         LIMIT ?2"
    ))
    .bind(model)
    .bind(SCORE_BATCH)
    .fetch_all(pool)
    .await?;
    let mut profiles: HashMap<String, Vec<f32>> = HashMap::new();
    for (id, investigation) in &docs {
        if !profiles.contains_key(investigation) {
            profiles.insert(investigation.clone(), interest(pool, embedder, investigation).await?);
        }
        let relevance = score(&passages(pool, id, model).await?, &profiles[investigation]);
        sqlx::query("UPDATE documents SET relevance = ? WHERE id = ?").bind(relevance).bind(id).execute(pool).await?;
    }
    Ok(docs.len())
}

/// The investigation's interest changed (feedback, name, targets): its documents are scored again.
// ponytail: rescores the whole investigation per change; score only what moved if it gets slow
pub async fn reset(pool: &SqlitePool, investigation_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE documents SET relevance = NULL WHERE investigation_id = ?").bind(investigation_id).execute(pool).await?;
    crate::collect::scheduler::wake();
    Ok(())
}

// ===== Commands =====

type CommandResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    pub id: String,
    pub investigation_id: String,
    pub investigation_name: String,
    pub url: String,
    pub kind: String,
    pub title: String,
    pub site: Option<String>,
    pub published_at: Option<String>,
    pub fetched_at: String,
    /// The start of the text (Markdown).
    pub excerpt: String,
    /// None: not scored yet.
    pub relevance: Option<f64>,
    pub read: bool,
    pub starred: bool,
    /// -1 disliked | 0 | 1 liked
    pub feedback: i64,
    /// The picture's address, when Settings › Collection lets the app load it from its site.
    pub image_url: Option<String>,
    /// The copy kept on this computer (collect::thumbs).
    pub image_path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedQuery {
    /// None: every investigation.
    pub investigation_ids: Option<Vec<String>>,
    /// politics | interest: only investigations of this kind (the Journal shows the interests).
    pub investigation_kind: Option<String>,
    /// date | relevance
    pub sort: String,
    /// Only what was published (else collected) from this time on (RFC 3339).
    pub since: Option<String>,
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default)]
    pub starred_only: bool,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[tauri::command]
pub async fn list_feed(state: tauri::State<'_, AppState>, query: FeedQuery) -> CommandResult<Page<FeedItem>> {
    let pool = state.db_manager.pool();
    let ids = query.investigation_ids.as_ref().map(|ids| serde_json::to_string(ids).unwrap_or_default());
    let filters = format!(
        "d.kind IN ({FEED_KINDS}) AND (?1 IS NULL OR d.investigation_id IN (SELECT value FROM json_each(?1))) AND (?2 IS NULL OR COALESCE(d.published_at, d.fetched_at) >= ?2)
         AND (?3 = 0 OR d.read_at IS NULL) AND (?4 = 0 OR d.starred = 1)
         AND (?7 IS NULL OR d.investigation_id IN (SELECT id FROM investigations WHERE kind = ?7))"
    );
    let order = match query.sort.as_str() {
        "relevance" => "d.relevance IS NULL, d.relevance DESC, COALESCE(d.published_at, d.fetched_at) DESC",
        _ => "COALESCE(d.published_at, d.fetched_at) DESC",
    };
    let mut items: Vec<FeedItem> = sqlx::query_as(&format!(
        "SELECT d.id, d.investigation_id, i.name AS investigation_name, d.url, d.kind, d.title, d.site, d.published_at, d.fetched_at,
                substr(d.text, 1, 600) AS excerpt, d.relevance, d.read_at IS NOT NULL AS read, d.starred = 1 AS starred, d.feedback,
                d.image_url, d.image_path
         FROM documents d JOIN investigations i ON i.id = d.investigation_id
         WHERE {filters} ORDER BY {order} LIMIT ?5 OFFSET ?6"
    ))
    .bind(&ids)
    .bind(&query.since)
    .bind(query.unread_only)
    .bind(query.starred_only)
    .bind(query.limit.unwrap_or(50).clamp(1, 200))
    .bind(query.offset.unwrap_or(0).max(0))
    .bind(&query.investigation_kind)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let mode = crate::collect::thumbs::mode(pool).await;
    for item in &mut items {
        if !mode.shows_remote() {
            item.image_url = None;
        }
        if !mode.keeps() || !item.image_path.as_deref().is_some_and(|p| std::path::Path::new(p).exists()) {
            item.image_path = None;
        }
    }
    let (total,): (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM documents d WHERE {filters}"))
        .bind(&ids)
        .bind(&query.since)
        .bind(query.unread_only)
        .bind(query.starred_only)
        .bind(0)
        .bind(0)
        .bind(&query.investigation_kind)
        .fetch_one(pool)
        .await
        .map_err(err)?;
    Ok(Page { items, total })
}

#[tauri::command]
pub async fn mark_read(state: tauri::State<'_, AppState>, ids: Vec<String>) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    let now = chrono::Utc::now().to_rfc3339();
    for id in ids {
        sqlx::query("UPDATE documents SET read_at = COALESCE(read_at, ?) WHERE id = ?").bind(&now).bind(id).execute(pool).await.map_err(err)?;
    }
    Ok(())
}

/// Marks everything in the Feed as read (in some investigations, or in all of a kind, or in all).
#[tauri::command]
pub async fn mark_all_read(state: tauri::State<'_, AppState>, investigation_ids: Option<Vec<String>>, investigation_kind: Option<String>) -> CommandResult<()> {
    sqlx::query(&format!(
        "UPDATE documents SET read_at = ?1 WHERE read_at IS NULL AND kind IN ({FEED_KINDS}) AND (?2 IS NULL OR investigation_id IN (SELECT value FROM json_each(?2)))
           AND (?3 IS NULL OR investigation_id IN (SELECT id FROM investigations WHERE kind = ?3))"
    ))
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(investigation_ids.map(|ids| serde_json::to_string(&ids).unwrap_or_default()))
    .bind(investigation_kind)
    .execute(state.db_manager.pool())
    .await
    .map_err(err)?;
    Ok(())
}

/// Stars a document, or says more (1) or less (-1) of this, or neither (0): the investigation's
/// documents are scored again with it.
#[tauri::command]
pub async fn set_document_flags(state: tauri::State<'_, AppState>, id: String, starred: Option<bool>, feedback: Option<i64>) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    if let Some(starred) = starred {
        sqlx::query("UPDATE documents SET starred = ? WHERE id = ?").bind(starred).bind(&id).execute(pool).await.map_err(err)?;
    }
    if let Some(feedback) = feedback {
        if !(-1..=1).contains(&feedback) {
            return Err(format!("Unknown feedback: {feedback}"));
        }
        sqlx::query("UPDATE documents SET feedback = ? WHERE id = ?").bind(feedback).bind(&id).execute(pool).await.map_err(err)?;
        let (investigation_id,): (String,) = sqlx::query_as("SELECT investigation_id FROM documents WHERE id = ?").bind(&id).fetch_one(pool).await.map_err(err)?;
        reset(pool, &investigation_id).await.map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_moves_the_interest() {
        let interest = vec![1.0, 0.0, 0.0];
        let gadgets = vec![vec![0.6, 0.8, 0.0]];
        let gossip = vec![vec![0.6, 0.0, 0.8]];
        let neutral = profile(&interest, &[], &[]);
        let liked = profile(&interest, &gadgets, &[]);
        let disliked = profile(&interest, &[], &gossip);
        assert!(score(&gadgets, &liked) > score(&gadgets, &neutral));
        assert!(score(&gossip, &disliked) < score(&gossip, &neutral));
        // A document is as close as its closest passage
        assert_eq!(score(&[vec![0.0, 1.0, 0.0], vec![2.0, 0.0, 0.0]], &neutral), 1.0);
    }

    #[tokio::test]
    async fn scores_documents_and_rescores_after_feedback() {
        struct Fixed;
        #[async_trait::async_trait]
        impl EmbeddingProvider for Fixed {
            fn model_id(&self) -> &str {
                "fixed"
            }
            async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
                Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
            }
        }
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, kind, created_at) VALUES ('inv', 'IA', 'interest', '2026')").execute(&pool).await.unwrap();
        for (doc, vector) in [("near", [0.9f32, 0.1]), ("far", [0.1, 0.9])] {
            sqlx::query("INSERT INTO documents (id, investigation_id, url, kind, title, text, fetched_at, content_hash) VALUES (?, 'inv', ?, 'article', ?, 't', '2026', ?)")
                .bind(doc)
                .bind(doc)
                .bind(doc)
                .bind(doc)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO rag_chunks (id, investigation_id, document_id, chunk_index, text, doc_date, embedding, embedding_model, dims, created_at) VALUES (?, 'inv', ?, 0, 't', '2026', ?, 'fixed', 2, '2026')")
                .bind(format!("c-{doc}"))
                .bind(doc)
                .bind(crate::rag::store::encode_vector(&vector))
                .execute(&pool)
                .await
                .unwrap();
        }
        assert_eq!(score_step(&pool, &Fixed).await.unwrap(), 2);
        let relevance = |doc: &'static str| {
            let pool = pool.clone();
            async move { sqlx::query_as::<_, (f64,)>("SELECT relevance FROM documents WHERE id = ?").bind(doc).fetch_one(&pool).await.unwrap().0 }
        };
        let before = relevance("far").await;
        assert!(relevance("near").await > before);
        sqlx::query("UPDATE documents SET feedback = 1 WHERE id = 'far'").execute(&pool).await.unwrap();
        reset(&pool, "inv").await.unwrap();
        assert_eq!(score_step(&pool, &Fixed).await.unwrap(), 2);
        assert!(relevance("far").await > before);
    }
}
