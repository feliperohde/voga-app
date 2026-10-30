//! External AI providers, for computers that cannot run the built-in models well: each one's
//! OpenAI-compatible API (chat, embeddings and transcription live under it) and the models it
//! offers, listed by the provider itself (model names change too often to keep a list here).

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::time::Duration;

use crate::state::AppState;

/// The base of a provider's OpenAI-compatible API (`<base>/chat/completions`, `/embeddings`,
/// `/audio/transcriptions`, `/models`). None for a provider without one.
pub fn openai_base(provider: &str, custom_endpoint: Option<&str>) -> Option<String> {
    Some(match provider {
        "openai" => "https://api.openai.com/v1".to_string(),
        "groq" => "https://api.groq.com/openai/v1".to_string(),
        "openrouter" => "https://openrouter.ai/api/v1".to_string(),
        "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai".to_string(),
        "deepseek" => "https://api.deepseek.com".to_string(),
        "custom-openai" => custom_endpoint.map(|e| e.trim().trim_end_matches('/').to_string()).filter(|e| !e.is_empty())?,
        _ => return None,
    })
}

/// The ids of a `/models` answer (`{"data": [{"id"}]}`), without Gemini's "models/" prefix, sorted.
fn model_ids(body: &Value) -> Vec<String> {
    let mut ids: Vec<String> = body["data"]
        .as_array()
        .map(|list| list.iter().filter_map(|m| m["id"].as_str()).map(|id| id.trim_start_matches("models/").to_string()).collect())
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    ids
}

/// The models a provider offers to this key.
pub async fn models(provider: &str, key: &str, custom_endpoint: Option<&str>) -> Result<Vec<String>> {
    let client = reqwest::Client::new();
    let request = if provider == "claude" {
        client.get("https://api.anthropic.com/v1/models?limit=100").header("x-api-key", key).header("anthropic-version", "2023-06-01")
    } else {
        let base = openai_base(provider, custom_endpoint).ok_or_else(|| anyhow!("{provider} has no model list"))?;
        let request = crate::net::client_for(&base).get(format!("{base}/models"));
        if key.is_empty() { request } else { request.bearer_auth(key) }
    };
    let response = request.timeout(Duration::from_secs(20)).send().await.with_context(|| format!("{provider} did not answer"))?;
    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(anyhow!("{provider} refused the key (HTTP {status})"));
    }
    if !status.is_success() {
        return Err(anyhow!("{provider} returned HTTP {status}"));
    }
    Ok(model_ids(&response.json().await.context("Unexpected answer to the model list")?))
}

/// The models of `provider`, with the key typed (`key`) or the one saved.
#[tauri::command]
pub async fn list_provider_models(state: tauri::State<'_, AppState>, provider: String, key: Option<String>, endpoint: Option<String>) -> Result<Vec<String>, String> {
    let pool = state.db_manager.pool();
    let key = match key.filter(|k| !k.trim().is_empty()) {
        Some(key) => key.trim().to_string(),
        None => crate::settings::api_key(pool, &provider).await.map_err(|e| e.to_string())?.unwrap_or_default(),
    };
    let endpoint = match endpoint {
        Some(endpoint) => Some(endpoint),
        None => crate::settings::load(pool).await.map_err(|e| e.to_string())?.custom_endpoint,
    };
    models(&provider, &key, endpoint.as_deref()).await.map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_have_their_api_and_custom_servers_theirs() {
        assert_eq!(openai_base("gemini", None).as_deref(), Some("https://generativelanguage.googleapis.com/v1beta/openai"));
        assert_eq!(openai_base("custom-openai", Some(" http://gpu.lan:8000/v1/ ")).as_deref(), Some("http://gpu.lan:8000/v1"));
        assert_eq!(openai_base("custom-openai", Some("")), None);
        assert_eq!(openai_base("claude", None), None);
    }

    #[test]
    fn model_lists_read_as_plain_ids() {
        let body = serde_json::json!({"object": "list", "data": [{"id": "models/gemini-3.1-flash"}, {"id": "gemini-embedding-2"}, {"id": "gemini-3.1-flash"}]});
        assert_eq!(model_ids(&body), vec!["gemini-3.1-flash", "gemini-embedding-2"]);
        assert!(model_ids(&serde_json::json!({"error": "x"})).is_empty());
    }
}
