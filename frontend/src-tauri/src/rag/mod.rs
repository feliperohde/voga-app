//! Search and Ask over what an investigation collected.

pub mod answer;
pub mod chunker;
pub mod embeddings;
pub mod indexer;
pub mod retriever;
pub mod store;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm::ConfiguredChatModel;
use crate::state::AppState;
use answer::{Answer, ConversationTurn};
use retriever::SearchResponse;
use store::SearchFilters;

#[tauri::command]
pub async fn search_documents(
    state: tauri::State<'_, AppState>,
    investigation_ids: Vec<String>,
    query: String,
    limit: Option<usize>,
) -> Result<SearchResponse, String> {
    let pool = state.db_manager.pool();
    let embedder = indexer::embedder(pool).await.map_err(|e| e.to_string())?;
    retriever::hybrid_search(pool, &embedder, &investigation_ids, &query, &SearchFilters::default(), limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskRequest {
    /// One investigation, or several asked about together.
    pub investigation_ids: Vec<String>,
    pub question: String,
    #[serde(default)]
    pub history: Vec<ConversationTurn>,
}

#[tauri::command]
pub async fn ask_question(state: tauri::State<'_, AppState>, request: AskRequest) -> Result<Answer, String> {
    let pool = state.db_manager.pool();
    if request.question.trim().is_empty() || request.investigation_ids.is_empty() {
        return Err("Empty question".into());
    }
    let embedder = indexer::embedder(pool).await.map_err(|e| e.to_string())?;
    let llm = ConfiguredChatModel::from_settings(pool).await.map_err(|e| e.to_string())?;
    let today = Utc::now().date_naive();
    let mut answer = answer::ask(pool, &embedder, &llm, &request.investigation_ids, &request.question, &request.history, today)
        .await
        .map_err(|e| e.to_string())?;

    // History is a convenience: a failed save must not lose the answer
    let id = format!("ask-{}", Uuid::new_v4());
    answer.history_id = Some(id.clone());
    let saved = sqlx::query("INSERT INTO ask_history (id, investigation_id, scope, question, answer_json, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(&request.investigation_ids[0])
        .bind(history_scope(&request.investigation_ids))
        .bind(request.question.trim())
        .bind(serde_json::to_string(&answer).unwrap_or_default())
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await;
    if let Err(e) = saved {
        log::error!("Ask: could not save to history: {e}");
        answer.history_id = None;
    }
    Ok(answer)
}

#[derive(Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AskHistoryEntry {
    pub id: String,
    pub question: String,
    pub created_at: String,
}

/// Questions asked about several investigations together are kept under that set (sorted ids);
/// those about one, under it alone (NULL).
fn history_scope(ids: &[String]) -> Option<String> {
    let mut ids = ids.to_vec();
    ids.sort();
    ids.dedup();
    (ids.len() > 1).then(|| serde_json::to_string(&ids).unwrap_or_default())
}

#[tauri::command]
pub async fn list_ask_history(state: tauri::State<'_, AppState>, investigation_ids: Vec<String>) -> Result<Vec<AskHistoryEntry>, String> {
    sqlx::query_as(
        "SELECT id, question, created_at FROM ask_history WHERE (?2 IS NULL AND investigation_id = ?1 AND scope IS NULL) OR scope = ?2
         ORDER BY created_at DESC LIMIT 50",
    )
        .bind(investigation_ids.first())
        .bind(history_scope(&investigation_ids))
        .fetch_all(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ask_answer(state: tauri::State<'_, AppState>, id: String) -> Result<Option<Answer>, String> {
    let row: Option<(String,)> = sqlx::query_as("SELECT answer_json FROM ask_history WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.and_then(|(json,)| serde_json::from_str(&json).ok()))
}
