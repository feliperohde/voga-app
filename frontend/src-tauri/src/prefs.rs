//! Small app preferences kept as key → value (the `prefs` table).

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::state::AppState;

/// "1": download the TSE's whole government-proposal dump of a state (up to 1.4 GB) instead of
/// reading only the candidate's PDF out of it.
pub const FULL_PROPOSALS: &str = "tse_full_proposals";
/// "0": the AI does not read what is collected (no promises, no contradictions).
pub const ANALYSIS: &str = "analysis_enabled";

pub async fn analysis_enabled(pool: &SqlitePool) -> bool {
    get(pool, ANALYSIS).await.ok().flatten().map_or(true, |v| v != "0")
}

pub async fn get(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM prefs WHERE key = ?").bind(key).fetch_optional(pool).await?;
    Ok(row.map(|(value,)| value))
}

pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO prefs (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectSettings {
    /// Download the TSE's whole proposal files (see FULL_PROPOSALS).
    #[serde(default)]
    pub full_proposals: bool,
    /// The AI reads what is collected for promises and contradictions.
    pub analysis: bool,
    /// url | image | both: how news pictures are shown (collect::thumbs).
    #[serde(default = "default_news_images")]
    pub news_images: String,
}

fn default_news_images() -> String {
    crate::collect::thumbs::Mode::Both.as_str().to_string()
}

#[tauri::command]
pub async fn get_collect_settings(state: tauri::State<'_, AppState>) -> Result<CollectSettings, String> {
    let pool = state.db_manager.pool();
    let full_proposals = get(pool, FULL_PROPOSALS).await.map_err(|e| e.to_string())?.is_some_and(|v| v == "1");
    let news_images = crate::collect::thumbs::mode(pool).await.as_str().to_string();
    Ok(CollectSettings { full_proposals, analysis: analysis_enabled(pool).await, news_images })
}

#[tauri::command]
pub async fn save_collect_settings(state: tauri::State<'_, AppState>, settings: CollectSettings) -> Result<(), String> {
    let pool = state.db_manager.pool();
    set(pool, FULL_PROPOSALS, if settings.full_proposals { "1" } else { "0" }).await.map_err(|e| e.to_string())?;
    set(pool, ANALYSIS, if settings.analysis { "1" } else { "0" }).await.map_err(|e| e.to_string())?;
    let images = crate::collect::thumbs::Mode::parse(&settings.news_images).ok_or_else(|| format!("Unknown picture mode: {}", settings.news_images))?;
    set(pool, crate::collect::thumbs::PREF, images.as_str()).await.map_err(|e| e.to_string())?;
    if settings.analysis || images.keeps() {
        crate::collect::scheduler::wake();
    }
    Ok(())
}
