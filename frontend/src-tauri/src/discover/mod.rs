//! Discovery: finding what to collect beyond the sources a person added. Each target gets its
//! searches (news and YouTube, and the web when a search service is set up); the links of what
//! the sources bring are followed (links.rs); and the AI suggests searches for the promises
//! nothing proves yet (suggest.rs), which wait for a person's approval. Each investigation has
//! daily limits, so discovery never runs away.

pub mod links;
pub mod suggest;
pub mod websearch;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::collect::{self, scheduler};
use crate::llm::ConfiguredChatModel;
use crate::state::AppState;

/// Links are followed at most this many pages away from a source.
pub const MAX_DEPTH: i64 = 2;

/// An investigation's discovery limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Limits {
    /// Pages queued from followed links each day.
    pub pages_per_day: i64,
    /// AI requests discovery makes each day (choosing links, suggesting searches).
    pub ai_calls_per_day: i64,
    /// How many links away from a source (0: links are not followed).
    pub depth: i64,
    /// The AI's suggestions become sources without waiting for approval.
    pub auto_approve: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self { pages_per_day: 100, ai_calls_per_day: 20, depth: 1, auto_approve: false }
    }
}

impl Limits {
    fn clamped(self) -> Self {
        Self {
            pages_per_day: self.pages_per_day.clamp(0, 2000),
            ai_calls_per_day: self.ai_calls_per_day.clamp(0, 500),
            depth: self.depth.clamp(0, MAX_DEPTH),
            auto_approve: self.auto_approve,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub pages: i64,
    pub ai_calls: i64,
}

pub async fn limits(pool: &SqlitePool, investigation_id: &str) -> Result<Limits> {
    let (json,): (String,) = sqlx::query_as("SELECT discovery FROM investigations WHERE id = ?").bind(investigation_id).fetch_one(pool).await?;
    Ok(serde_json::from_str::<Limits>(&json).unwrap_or_default().clamped())
}

/// Limits are per local day.
fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub async fn usage(pool: &SqlitePool, investigation_id: &str) -> Result<Usage> {
    Ok(sqlx::query_as("SELECT pages, ai_calls FROM discovery_usage WHERE investigation_id = ? AND day = ?")
        .bind(investigation_id)
        .bind(today())
        .fetch_optional(pool)
        .await?
        .unwrap_or_default())
}

pub async fn spend(pool: &SqlitePool, investigation_id: &str, pages: i64, ai_calls: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO discovery_usage (investigation_id, day, pages, ai_calls) VALUES (?, ?, ?, ?)
         ON CONFLICT(investigation_id, day) DO UPDATE SET pages = pages + excluded.pages, ai_calls = ai_calls + excluded.ai_calls",
    )
    .bind(investigation_id)
    .bind(today())
    .bind(pages)
    .bind(ai_calls)
    .execute(pool)
    .await?;
    Ok(())
}

/// What is left today: (pages, AI requests).
pub async fn left(pool: &SqlitePool, investigation_id: &str) -> Result<(i64, i64)> {
    let limits = limits(pool, investigation_id).await?;
    let used = usage(pool, investigation_id).await?;
    Ok(((limits.pages_per_day - used.pages).max(0), (limits.ai_calls_per_day - used.ai_calls).max(0)))
}

/// What the investigation is about, for the AI: its name, description and who it follows.
pub(crate) async fn brief(pool: &SqlitePool, investigation_id: &str) -> Result<String> {
    let (name, description): (String, String) =
        sqlx::query_as("SELECT name, description FROM investigations WHERE id = ?").bind(investigation_id).fetch_one(pool).await?;
    let targets: Vec<(String, String, String)> = sqlx::query_as("SELECT kind, name, aliases FROM targets WHERE investigation_id = ? ORDER BY created_at")
        .bind(investigation_id)
        .fetch_all(pool)
        .await?;
    let followed = targets
        .iter()
        .map(|(kind, name, aliases)| {
            let aliases: Vec<String> = serde_json::from_str(aliases).unwrap_or_default();
            let also = if aliases.is_empty() { String::new() } else { format!(" (também: {})", aliases.join(", ")) };
            format!("- {name}{also} [{kind}]")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!("Investigation: {name}\n{description}\nFollowed:\n{followed}"))
}

/// The configured model at temperature 0, if one is ready. None while a live session needs it.
pub(crate) async fn model(pool: &SqlitePool) -> Option<ConfiguredChatModel> {
    if crate::live::session::busy() {
        return None;
    }
    ConfiguredChatModel::from_settings(pool).await.ok().map(ConfiguredChatModel::deterministic)
}

// ===== Commands =====

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryView {
    pub limits: Limits,
    pub usage: Usage,
}

#[tauri::command]
pub async fn get_discovery(state: tauri::State<'_, AppState>, investigation_id: String) -> Result<DiscoveryView, String> {
    let pool = state.db_manager.pool();
    let limits = limits(pool, &investigation_id).await.map_err(|e| e.to_string())?;
    let usage = usage(pool, &investigation_id).await.map_err(|e| e.to_string())?;
    Ok(DiscoveryView { limits, usage })
}

#[tauri::command]
pub async fn save_discovery(state: tauri::State<'_, AppState>, investigation_id: String, limits: Limits) -> Result<(), String> {
    let json = serde_json::to_string(&limits.clamped()).map_err(|e| e.to_string())?;
    sqlx::query("UPDATE investigations SET discovery = ? WHERE id = ?")
        .bind(json)
        .bind(&investigation_id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// A suggested source starts running.
#[tauri::command]
pub async fn approve_source(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("UPDATE sources SET status = 'active', next_run_at = ? WHERE id = ? AND status = 'suggested'")
        .bind(collect::schedule_time(chrono::Utc::now()))
        .bind(id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    scheduler::wake();
    Ok(())
}

/// A suggested source is dismissed; it is kept so the AI does not suggest it again.
#[tauri::command]
pub async fn reject_source(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("UPDATE sources SET status = 'rejected' WHERE id = ? AND status = 'suggested'")
        .bind(id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Asks the AI for searches now (it also does so by itself once a day). Returns how many it suggested.
#[tauri::command]
pub async fn suggest_sources(state: tauri::State<'_, AppState>, investigation_id: String) -> Result<usize, String> {
    let pool = state.db_manager.pool();
    let llm = ConfiguredChatModel::from_settings(pool).await.map_err(|e| e.to_string())?.deterministic();
    suggest::suggest(pool, &llm, &investigation_id, true).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_kept_in_range_and_default_when_missing() {
        let parsed: Limits = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, Limits::default());
        let parsed: Limits = serde_json::from_str(r#"{"depth": 7, "pagesPerDay": -3}"#).unwrap();
        let clamped = parsed.clamped();
        assert_eq!(clamped.depth, MAX_DEPTH);
        assert_eq!(clamped.pages_per_day, 0);
        assert_eq!(clamped.ai_calls_per_day, Limits::default().ai_calls_per_day);
    }
}
