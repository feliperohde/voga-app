//! Hybrid retrieval: vector (semantic) + BM25 (exact terms such as names and bill numbers),
//! merged with Reciprocal Rank Fusion so neither score scale dominates.

use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;

use super::embeddings::EmbeddingProvider;
use super::store::{ChunkHit, RagStore, SearchFilters};

/// Standard RRF damping constant.
const RRF_K: f64 = 60.0;
/// Candidates taken from each retriever before fusion.
const CANDIDATES_PER_RETRIEVER: usize = 20;
/// Cosine similarity below which a passage is considered unrelated. Deliberately
/// low: it only drops clear noise; the answer step judges relevance beyond that.
const MIN_VECTOR_SIMILARITY: f64 = 0.3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    pub hit: ChunkHit,
    /// Which retrievers found this chunk: "vector", "lexical".
    pub sources: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    /// Set when semantic search was skipped (e.g. Ollama unavailable); results are keyword only.
    pub vector_error: Option<String>,
}

/// Fuses ranked lists with RRF: score = Σ 1 / (k + rank).
pub fn reciprocal_rank_fusion(lists: Vec<(&'static str, Vec<ChunkHit>)>, limit: usize) -> Vec<SearchResult> {
    let mut fused: HashMap<String, (f64, SearchResult)> = HashMap::new();
    for (source, hits) in lists {
        for (rank, hit) in hits.into_iter().enumerate() {
            let contribution = 1.0 / (RRF_K + rank as f64 + 1.0);
            fused
                .entry(hit.chunk_id.clone())
                .and_modify(|(score, result)| {
                    *score += contribution;
                    result.sources.push(source);
                })
                .or_insert_with(|| (contribution, SearchResult { hit, sources: vec![source] }));
        }
    }
    let mut results: Vec<(f64, SearchResult)> = fused.into_values().collect();
    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    results
        .into_iter()
        .take(limit)
        .map(|(score, mut result)| {
            result.hit.score = score;
            result
        })
        .collect()
}

pub async fn hybrid_search(
    pool: &SqlitePool,
    embedder: &dyn EmbeddingProvider,
    scope: &[String],
    query: &str,
    filters: &SearchFilters,
    limit: usize,
) -> Result<SearchResponse, sqlx::Error> {
    let lexical = RagStore::lexical_search(pool, scope, query, filters, CANDIDATES_PER_RETRIEVER).await?;
    let (vector, vector_error) = match embedder.embed(&[query.to_string()]).await {
        Ok(mut vectors) if !vectors.is_empty() => {
            let query_vector = vectors.remove(0);
            let hits = RagStore::vector_search(
                pool,
                scope,
                &query_vector,
                embedder.model_id(),
                filters,
                CANDIDATES_PER_RETRIEVER,
                MIN_VECTOR_SIMILARITY,
            )
            .await?;
            (hits, None)
        }
        Ok(_) => (Vec::new(), Some("Empty embedding response".to_string())),
        Err(e) => {
            log::warn!("RAG: semantic search unavailable, using keywords only: {e}");
            (Vec::new(), Some(e.to_string()))
        }
    };
    Ok(SearchResponse { results: reciprocal_rank_fusion(vec![("vector", vector), ("lexical", lexical)], limit), vector_error })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str) -> ChunkHit {
        ChunkHit {
            chunk_id: id.to_string(),
            document_id: "d".into(),
            title: "t".into(),
            url: "https://example.com".into(),
            kind: "article".into(),
            site: None,
            doc_date: "2026-09-29".into(),
            text: id.to_string(),
            start_time: None,
            end_time: None,
            score: 0.0,
        }
    }

    #[test]
    fn rrf_rewards_agreement_between_retrievers() {
        let fused = reciprocal_rank_fusion(vec![("vector", vec![hit("a"), hit("b"), hit("c")]), ("lexical", vec![hit("c"), hit("d")])], 10);
        // "c" appears in both lists and outranks single-list items below the top
        assert_eq!(fused[0].hit.chunk_id, "c");
        assert_eq!(fused[0].sources, vec!["vector", "lexical"]);
        assert_eq!(fused.len(), 4);
        assert!(fused.windows(2).all(|w| w[0].hit.score >= w[1].hit.score));
    }

    #[test]
    fn rrf_respects_limit() {
        let fused = reciprocal_rank_fusion(vec![("lexical", vec![hit("a"), hit("b"), hit("c")])], 2);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].hit.chunk_id, "a");
    }
}
