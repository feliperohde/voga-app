//! Web search, an option: Brave Search's API (a key, free tier available) or a SearXNG the user
//! runs (on this computer or elsewhere; its JSON format must be enabled). Without one, discovery
//! still has the news and YouTube searches, which need no setup. Bing's web results cannot be
//! used: its robots.txt disallows /search.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use std::time::Duration;

use crate::collect::feeds::Entry;
use crate::prefs;
use crate::state::AppState;

const PROVIDER: &str = "websearch_provider";
const ENDPOINT: &str = "websearch_endpoint";
// ponytail: kept in the local database like the AI keys; move to the Keychain when Voga ships widely
const KEY: &str = "websearch_key";
/// Results taken from each search.
const RESULTS: usize = 20;
const BRAVE: &str = "https://api.search.brave.com/res/v1/web/search";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchSettings {
    /// "" (off) | brave | searxng
    pub provider: String,
    /// SearXNG's address.
    #[serde(default)]
    pub endpoint: String,
    /// Brave's key. Empty: keep the saved one. Never sent back to the UI.
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub has_key: bool,
}

async fn get(pool: &SqlitePool, key: &str) -> String {
    prefs::get(pool, key).await.ok().flatten().unwrap_or_default()
}

/// The search service set up, if any: (provider, endpoint, key).
async fn service(pool: &SqlitePool) -> Option<(String, String, String)> {
    let provider = get(pool, PROVIDER).await;
    let (endpoint, key) = (get(pool, ENDPOINT).await, get(pool, KEY).await);
    match provider.as_str() {
        "brave" if !key.is_empty() => Some((provider, endpoint, key)),
        "searxng" if !endpoint.is_empty() => Some((provider, endpoint, key)),
        _ => None,
    }
}

pub async fn configured(pool: &SqlitePool) -> bool {
    service(pool).await.is_some()
}

/// Brave: {"web": {"results": [{url, title, page_age}]}}; SearXNG: {"results": [{url, title, publishedDate}]}.
pub fn parse_results(provider: &str, body: &Value) -> Vec<Entry> {
    let (list, date) = match provider {
        "brave" => (&body["web"]["results"], "page_age"),
        _ => (&body["results"], "publishedDate"),
    };
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let url = r["url"].as_str()?.to_string();
            url.starts_with("http").then(|| Entry {
                url,
                title: r["title"].as_str().map(str::to_string),
                published: r[date].as_str().and_then(crate::collect::extract::normalize_date),
                summary: None,
                image: None,
            })
        })
        .take(RESULTS)
        .collect()
}

/// Searches the web with the service set up in Settings › Collection.
pub async fn search(pool: &SqlitePool, query: &str) -> Result<Vec<Entry>> {
    let (provider, endpoint, key) = service(pool).await.ok_or_else(|| anyhow!("Set up a web search service in Settings › Collection"))?;
    let request = match provider.as_str() {
        "brave" => crate::net::client_for(BRAVE)
            .get(BRAVE)
            .query(&[("q", query), ("count", "20"), ("country", "BR"), ("search_lang", "pt-br")])
            .header("Accept", "application/json")
            .header("X-Subscription-Token", key),
        _ => {
            let base = endpoint.trim().trim_end_matches('/');
            let base = if base.contains("://") { base.to_string() } else { format!("http://{base}") };
            crate::net::client_for(&base).get(format!("{base}/search")).query(&[("q", query), ("format", "json"), ("language", "pt-BR")])
        }
    };
    let response = request.timeout(Duration::from_secs(30)).send().await.with_context(|| format!("{provider} did not answer"))?;
    let status = response.status();
    if status.as_u16() == 403 && provider == "searxng" {
        bail!("SearXNG refused the JSON format: enable it in its settings.yml (search: formats: [html, json])");
    }
    if !status.is_success() {
        bail!("{provider} returned HTTP {status}");
    }
    let body: Value = response.json().await.context("Unexpected answer from the search service")?;
    Ok(parse_results(&provider, &body))
}

/// Each target gets a web search while a service is set up; without one they are paused.
async fn sync_target_searches(pool: &SqlitePool) -> Result<()> {
    if !configured(pool).await {
        sqlx::query("UPDATE sources SET status = 'paused' WHERE kind = 'web_search' AND origin = 'target'").execute(pool).await?;
        return Ok(());
    }
    sqlx::query("UPDATE sources SET status = 'active' WHERE kind = 'web_search' AND origin = 'target' AND status = 'paused'").execute(pool).await?;
    let missing: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT t.id, t.investigation_id, t.name FROM targets t
         WHERE NOT EXISTS (SELECT 1 FROM sources s WHERE s.target_id = t.id AND s.kind = 'web_search')",
    )
    .fetch_all(pool)
    .await?;
    for (target_id, investigation_id, name) in missing {
        sqlx::query(
            "INSERT OR IGNORE INTO sources (id, investigation_id, target_id, kind, url, title, every_minutes, next_run_at, origin, created_at)
             VALUES (?, ?, ?, 'web_search', ?, ?, 1440, ?, 'target', ?)",
        )
        .bind(format!("src-{}", uuid::Uuid::new_v4()))
        .bind(&investigation_id)
        .bind(&target_id)
        .bind(format!("\"{name}\""))
        .bind(&name)
        .bind(crate::collect::schedule_time(chrono::Utc::now()))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_websearch_settings(state: tauri::State<'_, AppState>) -> Result<WebSearchSettings, String> {
    let pool = state.db_manager.pool();
    Ok(WebSearchSettings { provider: get(pool, PROVIDER).await, endpoint: get(pool, ENDPOINT).await, key: String::new(), has_key: !get(pool, KEY).await.is_empty() })
}

#[tauri::command]
pub async fn save_websearch_settings(state: tauri::State<'_, AppState>, settings: WebSearchSettings) -> Result<(), String> {
    let pool = state.db_manager.pool();
    let provider = if matches!(settings.provider.as_str(), "brave" | "searxng") { settings.provider.as_str() } else { "" };
    prefs::set(pool, PROVIDER, provider).await.map_err(|e| e.to_string())?;
    prefs::set(pool, ENDPOINT, settings.endpoint.trim()).await.map_err(|e| e.to_string())?;
    if !settings.key.trim().is_empty() {
        prefs::set(pool, KEY, settings.key.trim()).await.map_err(|e| e.to_string())?;
    }
    sync_target_searches(pool).await.map_err(|e| e.to_string())?;
    crate::collect::scheduler::wake();
    Ok(())
}

/// Searches with the saved settings: how many results came back.
#[tauri::command]
pub async fn test_websearch(state: tauri::State<'_, AppState>, query: String) -> Result<usize, String> {
    search(state.db_manager.pool(), &query).await.map(|r| r.len()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn results_of_both_services() {
        let brave = json!({"web": {"results": [
            {"url": "https://a.example/x", "title": "A", "page_age": "2023-07-06T10:00:00"},
            {"url": "javascript:void(0)", "title": "Bad"}
        ]}});
        let found = parse_results("brave", &brave);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].url, "https://a.example/x");
        assert_eq!(found[0].title.as_deref(), Some("A"));
        assert!(found[0].published.as_deref().is_some_and(|d| d.starts_with("2023-07-06")));

        let searx = json!({"results": [{"url": "https://b.example/y", "title": "B"}]});
        assert_eq!(parse_results("searxng", &searx)[0].url, "https://b.example/y");
        assert!(parse_results("searxng", &json!({})).is_empty());
    }
}
