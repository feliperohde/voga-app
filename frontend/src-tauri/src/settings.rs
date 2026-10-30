//! Settings › AI: the chat model every feature uses and the embedding model of the index.
//! API keys are kept apart and never sent back to the UI, only whether each one is set.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;

use crate::llm::{ChatModel, ConfiguredChatModel};
use crate::state::AppState;

pub const DEFAULT_OLLAMA: &str = "http://localhost:11434";

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    /// builtin-ai | openai | claude | groq | ollama | openrouter | custom-openai | claude-code
    pub provider: String,
    /// Empty with builtin-ai: the best built-in model downloaded.
    pub model: String,
    pub ollama_endpoint: Option<String>,
    /// Base URL of an OpenAI-compatible server (provider custom-openai).
    pub custom_endpoint: Option<String>,
    /// builtin | ollama: what embeds passages for search.
    pub embedding_provider: String,
    /// A built-in embedding model (bge-m3-q8) or an Ollama one (bge-m3).
    pub embedding_model: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsView {
    #[serde(flatten)]
    pub settings: AiSettings,
    /// Providers that have an API key saved.
    pub keys: Vec<String>,
}

pub async fn load(pool: &SqlitePool) -> Result<AiSettings> {
    Ok(sqlx::query_as("SELECT provider, model, ollama_endpoint, custom_endpoint, embedding_provider, embedding_model FROM ai_settings WHERE id = 1")
        .fetch_one(pool)
        .await?)
}

async fn keys(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let (json,): (String,) = sqlx::query_as("SELECT api_keys FROM ai_settings WHERE id = 1").fetch_one(pool).await?;
    Ok(serde_json::from_str(&json).unwrap_or_default())
}

pub async fn api_key(pool: &SqlitePool, provider: &str) -> Result<Option<String>> {
    Ok(keys(pool).await?.remove(provider).filter(|k| !k.is_empty()))
}

pub fn ollama_endpoint(settings: &AiSettings) -> &str {
    settings.ollama_endpoint.as_deref().map(str::trim).filter(|e| !e.is_empty()).unwrap_or(DEFAULT_OLLAMA)
}

#[tauri::command]
pub async fn get_ai_settings(state: tauri::State<'_, AppState>) -> Result<AiSettingsView, String> {
    let pool = state.db_manager.pool();
    let settings = load(pool).await.map_err(|e| e.to_string())?;
    let mut keys: Vec<String> = keys(pool).await.map_err(|e| e.to_string())?.into_iter().filter(|(_, k)| !k.is_empty()).map(|(p, _)| p).collect();
    keys.sort();
    Ok(AiSettingsView { settings, keys })
}

#[tauri::command]
pub async fn save_ai_settings(state: tauri::State<'_, AppState>, settings: AiSettings) -> Result<(), String> {
    sqlx::query("UPDATE ai_settings SET provider = ?, model = ?, ollama_endpoint = ?, custom_endpoint = ?, embedding_provider = ?, embedding_model = ? WHERE id = 1")
        .bind(settings.provider.trim())
        .bind(settings.model.trim())
        .bind(settings.ollama_endpoint.as_deref().map(str::trim))
        .bind(settings.custom_endpoint.as_deref().map(str::trim))
        .bind(settings.embedding_provider.trim())
        .bind(settings.embedding_model.trim())
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Saves the key of `provider`; an empty one removes it.
#[tauri::command]
pub async fn save_api_key(state: tauri::State<'_, AppState>, provider: String, key: String) -> Result<(), String> {
    let pool = state.db_manager.pool();
    let mut all = keys(pool).await.map_err(|e| e.to_string())?;
    match key.trim() {
        "" => all.remove(&provider),
        key => all.insert(provider, key.to_string()),
    };
    sqlx::query("UPDATE ai_settings SET api_keys = ? WHERE id = 1")
        .bind(serde_json::to_string(&all).map_err(|e| e.to_string())?)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Models installed on the Ollama server at `endpoint` (default: the saved one).
#[tauri::command]
pub async fn list_ollama_models(state: tauri::State<'_, AppState>, endpoint: Option<String>) -> Result<Vec<String>, String> {
    let saved = load(state.db_manager.pool()).await.map_err(|e| e.to_string())?;
    let endpoint = endpoint.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| ollama_endpoint(&saved).to_string());
    let url = format!("{}/api/tags", endpoint.trim().trim_end_matches('/'));

    #[derive(Deserialize)]
    struct Tags {
        models: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        name: String,
    }
    let tags: Tags = crate::net::client_for(&url)
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Ollama não respondeu em {endpoint}: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let mut names: Vec<String> = tags.models.into_iter().map(|t| t.name).collect();
    names.sort();
    Ok(names)
}

/// Sends a tiny prompt to the configured model: Ok(its answer) when it works.
#[tauri::command]
pub async fn test_ai_model(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let model = ConfiguredChatModel::from_settings(state.db_manager.pool()).await.map_err(|e| e.to_string())?;
    model
        .complete("Você é um teste de conexão.", "Responda apenas com a palavra: ok")
        .await
        .map_err(|e| e.to_string())
}
