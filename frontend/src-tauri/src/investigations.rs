//! Investigations, who they follow (targets), where content comes from (sources) and
//! what was collected (documents): the commands behind those pages.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::collect::official::{self, OfficialMatch};
use crate::collect::{self, feeds, scheduler, youtube};
use crate::state::AppState;

type CommandResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Investigation {
    pub id: String,
    pub name: String,
    pub description: String,
    /// politics | interest (any other subject: no official data, no claim analysis)
    pub kind: String,
    pub created_at: String,
    pub document_count: i64,
    pub source_count: i64,
}

const INVESTIGATION_SELECT: &str = "SELECT i.id, i.name, i.description, i.kind, i.created_at,
        (SELECT COUNT(*) FROM documents d WHERE d.investigation_id = i.id) AS document_count,
        (SELECT COUNT(*) FROM sources s WHERE s.investigation_id = i.id) AS source_count
     FROM investigations i";

#[tauri::command]
pub async fn list_investigations(state: tauri::State<'_, AppState>) -> CommandResult<Vec<Investigation>> {
    sqlx::query_as(&format!("{INVESTIGATION_SELECT} ORDER BY i.created_at"))
        .fetch_all(state.db_manager.pool())
        .await
        .map_err(err)
}

async fn get_investigation(pool: &SqlitePool, id: &str) -> Result<Investigation, sqlx::Error> {
    sqlx::query_as(&format!("{INVESTIGATION_SELECT} WHERE i.id = ?")).bind(id).fetch_one(pool).await
}

#[tauri::command]
pub async fn create_investigation(state: tauri::State<'_, AppState>, name: String, description: Option<String>, kind: Option<String>) -> CommandResult<Investigation> {
    create(state.db_manager.pool(), &name, description, kind).await
}

async fn create(pool: &SqlitePool, name: &str, description: Option<String>, kind: Option<String>) -> CommandResult<Investigation> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the investigation a name".into());
    }
    let kind = kind.unwrap_or_else(|| "politics".into());
    if !matches!(kind.as_str(), "politics" | "interest") {
        return Err(format!("Unknown investigation kind: {kind}"));
    }
    let id = format!("inv-{}", Uuid::new_v4());
    sqlx::query("INSERT INTO investigations (id, name, description, kind, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(name)
        .bind(description.unwrap_or_default().trim())
        .bind(&kind)
        .bind(now())
        .execute(pool)
        .await
        .map_err(err)?;
    get_investigation(pool, &id).await.map_err(err)
}

#[tauri::command]
pub async fn update_investigation(state: tauri::State<'_, AppState>, id: String, name: String, description: String) -> CommandResult<Investigation> {
    if name.trim().is_empty() {
        return Err("Give the investigation a name".into());
    }
    let pool = state.db_manager.pool();
    sqlx::query("UPDATE investigations SET name = ?, description = ? WHERE id = ?")
        .bind(name.trim())
        .bind(description.trim())
        .bind(&id)
        .execute(pool)
        .await
        .map_err(err)?;
    crate::feed::reset(pool, &id).await.map_err(err)?;
    get_investigation(pool, &id).await.map_err(err)
}

/// Turns a political investigation into an interest or back: its targets' searches follow (an
/// interest's themes are searched as loose words and leave the gazettes out), and the AI's
/// pending suggestions, made for the other kind, go.
#[tauri::command]
pub async fn set_investigation_kind(state: tauri::State<'_, AppState>, id: String, kind: String) -> CommandResult<Investigation> {
    if !matches!(kind.as_str(), "politics" | "interest") {
        return Err(format!("Unknown investigation kind: {kind}"));
    }
    let pool = state.db_manager.pool();
    change_kind(pool, &id, &kind).await.map_err(err)?;
    crate::feed::reset(pool, &id).await.map_err(err)?;
    get_investigation(pool, &id).await.map_err(err)
}

async fn change_kind(pool: &SqlitePool, id: &str, kind: &str) -> Result<(), sqlx::Error> {
    let interest = kind == "interest";
    sqlx::query("UPDATE investigations SET kind = ? WHERE id = ?").bind(kind).bind(id).execute(pool).await?;
    let targets: Vec<(String, String, String)> = sqlx::query_as("SELECT id, kind, name FROM targets WHERE investigation_id = ?").bind(id).fetch_all(pool).await?;
    for (target_id, target_kind, name) in targets {
        sqlx::query(
            "UPDATE OR IGNORE sources SET url = ?1, next_run_at = ?2 WHERE target_id = ?3 AND origin = 'target' AND url != ?1
               AND kind IN ('news_search', 'youtube_search', 'web_search', 'gazette_search')",
        )
        .bind(target_query(interest, &target_kind, &name))
        .bind(collect::schedule_time(Utc::now()))
        .bind(&target_id)
        .execute(pool)
        .await?;
        let gazette: Option<(String,)> =
            sqlx::query_as("SELECT id FROM sources WHERE target_id = ? AND origin = 'target' AND kind = 'gazette_search'").bind(&target_id).fetch_optional(pool).await?;
        match gazette {
            Some((source,)) if interest => {
                sqlx::query("DELETE FROM sources WHERE id = ?").bind(source).execute(pool).await?;
            }
            None if !interest => {
                insert_source(pool, id, Some(&target_id), "gazette_search", &target_query(false, &target_kind, &name), Some(&name), 1440, "target").await?;
            }
            _ => {}
        }
    }
    sqlx::query("DELETE FROM sources WHERE investigation_id = ? AND status = 'suggested'").bind(id).execute(pool).await?;
    Ok(())
}

/// Starts the error of a deletion refused while a live session follows someone in the investigation.
pub const LIVE_IN_USE: &str = "LIVE_IN_USE";

/// Deletes the investigation with everything it collected.
#[tauri::command]
pub async fn delete_investigation(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    delete(state.db_manager.pool(), &id, crate::live::session::active().as_deref()).await.map_err(err)
}

/// The database drops what the investigation holds (sources, documents and their index, the graph,
/// questions, live sessions); its pictures, its sessions' sound and its flowsint link go here.
/// Refused while `recording` (the live session in progress) is one of its own or a debate with
/// someone in it.
async fn delete(pool: &SqlitePool, id: &str, recording: Option<&str>) -> anyhow::Result<()> {
    if let Some(session) = recording {
        let (in_use,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM live_sessions s WHERE s.id = ?1 AND (s.investigation_id = ?2 OR EXISTS (
                SELECT 1 FROM live_participants p JOIN targets t ON t.id = p.target_id WHERE p.session_id = s.id AND t.investigation_id = ?2)))",
        )
        .bind(session)
        .bind(id)
        .fetch_one(pool)
        .await?;
        if in_use {
            anyhow::bail!("{LIVE_IN_USE}: stop the live session before deleting this investigation");
        }
    }
    let sounds: Vec<(String,)> =
        sqlx::query_as("SELECT audio_path FROM live_sessions WHERE investigation_id = ? AND audio_path IS NOT NULL").bind(id).fetch_all(pool).await?;
    collect::thumbs::forget(pool, "investigation_id = ?", id).await;
    sqlx::query("DELETE FROM investigations WHERE id = ?").bind(id).execute(pool).await?;
    sqlx::query("DELETE FROM prefs WHERE key = ?").bind(format!("flowsint_sync:{id}")).execute(pool).await?;
    for (path,) in sounds {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

// ===== Targets =====

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    pub investigation_id: String,
    /// person | organization | topic
    pub kind: String,
    pub name: String,
    pub aliases: Vec<String>,
    /// The official records it is linked to: the sources made for them.
    pub official: Vec<OfficialMatch>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInput {
    /// None: a new target.
    pub id: Option<String>,
    pub investigation_id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[tauri::command]
pub async fn list_targets(state: tauri::State<'_, AppState>, investigation_id: String) -> CommandResult<Vec<Target>> {
    let pool = state.db_manager.pool();
    let rows: Vec<(String, String, String, String, String, String)> =
        sqlx::query_as("SELECT id, investigation_id, kind, name, aliases, created_at FROM targets WHERE investigation_id = ? ORDER BY created_at")
            .bind(&investigation_id)
            .fetch_all(pool)
            .await
            .map_err(err)?;
    let linked: Vec<(String, String, String, Option<String>)> = sqlx::query_as(&format!(
        "SELECT target_id, kind, url, title FROM sources WHERE investigation_id = ? AND kind IN ({}) ORDER BY created_at",
        official::KINDS.map(|k| format!("'{k}'")).join(", ")
    ))
    .bind(&investigation_id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    Ok(rows
        .into_iter()
        .map(|(id, investigation_id, kind, name, aliases, created_at)| Target {
            official: linked
                .iter()
                .filter(|(target, ..)| *target == id)
                .map(|(_, kind, url, title)| {
                    let (label, detail) = title.as_deref().unwrap_or_default().split_once(" · ").unwrap_or((title.as_deref().unwrap_or_default(), ""));
                    OfficialMatch { kind: kind.clone(), id: url.clone(), label: label.to_string(), detail: detail.to_string(), similar: false }
                })
                .collect(),
            id,
            investigation_id,
            kind,
            name,
            aliases: serde_json::from_str(&aliases).unwrap_or_default(),
            created_at,
        })
        .collect())
}

/// The searches a target gets: news, YouTube and municipal gazettes, for its name in quotes (and
/// the web, when a web search service is set up). Interests leave the gazettes out, and search a
/// topic as loose words.
const TARGET_SEARCHES: [(&str, i64); 3] = [("news_search", 360), ("youtube_search", 1440), ("gazette_search", 1440)];

/// What a target's searches look for: its name in quotes, or loose words for an interest's theme.
fn target_query(interest: bool, kind: &str, name: &str) -> String {
    if interest && kind == "topic" {
        name.to_string()
    } else {
        format!("\"{name}\"")
    }
}

/// Creates or updates a target. A new target gets its news and video searches; renaming
/// it updates them.
#[tauri::command]
pub async fn save_target(state: tauri::State<'_, AppState>, target: TargetInput) -> CommandResult<String> {
    save(state.db_manager.pool(), target).await
}

async fn save(pool: &SqlitePool, target: TargetInput) -> CommandResult<String> {
    let name = target.name.trim().to_string();
    if name.is_empty() {
        return Err("Give the target a name".into());
    }
    if !matches!(target.kind.as_str(), "person" | "organization" | "topic") {
        return Err(format!("Unknown target kind: {}", target.kind));
    }
    let aliases: Vec<String> = target.aliases.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty() && *a != name).collect();
    let aliases = serde_json::to_string(&aliases).map_err(err)?;
    let (investigation_kind,): (String,) = sqlx::query_as("SELECT kind FROM investigations WHERE id = ?").bind(&target.investigation_id).fetch_one(pool).await.map_err(err)?;
    let interest = investigation_kind == "interest";
    let query = target_query(interest, &target.kind, &name);
    let id = match target.id {
        Some(id) => {
            sqlx::query("UPDATE targets SET kind = ?, name = ?, aliases = ? WHERE id = ?")
                .bind(&target.kind)
                .bind(&name)
                .bind(&aliases)
                .bind(&id)
                .execute(pool)
                .await
                .map_err(err)?;
            // Only its searches: its official sources keep their ids
            sqlx::query("UPDATE OR IGNORE sources SET url = ?, title = ? WHERE target_id = ? AND origin = 'target' AND kind IN ('news_search', 'youtube_search', 'web_search', 'gazette_search')")
                .bind(&query)
                .bind(&name)
                .bind(&id)
                .execute(pool)
                .await
                .map_err(err)?;
            id
        }
        None => {
            let id = format!("target-{}", Uuid::new_v4());
            sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, aliases, created_at) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(&id)
                .bind(&target.investigation_id)
                .bind(&target.kind)
                .bind(&name)
                .bind(&aliases)
                .bind(now())
                .execute(pool)
                .await
                .map_err(err)?;
            for (kind, every_minutes) in TARGET_SEARCHES.into_iter().filter(|(kind, _)| !(interest && *kind == "gazette_search")) {
                insert_source(pool, &target.investigation_id, Some(&id), kind, &query, Some(&name), every_minutes, "target").await.map_err(err)?;
            }
            if crate::discover::websearch::configured(pool).await {
                insert_source(pool, &target.investigation_id, Some(&id), "web_search", &query, Some(&name), 1440, "target").await.map_err(err)?;
            }
            scheduler::wake();
            id
        }
    };
    crate::feed::reset(pool, &target.investigation_id).await.map_err(err)?;
    Ok(id)
}

/// A person followed somewhere under this name (or alias), the investigation with the most
/// documents first; else a new political investigation named after them, following them (a
/// debater nobody followed yet).
pub async fn follow_person(pool: &SqlitePool, name: &str) -> CommandResult<String> {
    let name = name.trim();
    let wanted = collect::fold(name);
    let people: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT t.id, t.name, t.aliases FROM targets t WHERE t.kind = 'person'
         ORDER BY (SELECT COUNT(*) FROM documents d WHERE d.investigation_id = t.investigation_id) DESC, t.created_at",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let known = people.into_iter().find(|(_, known, aliases)| {
        std::iter::once(known.clone()).chain(serde_json::from_str::<Vec<String>>(aliases).unwrap_or_default()).any(|n| collect::fold(n.trim()) == wanted)
    });
    if let Some((id, ..)) = known {
        return Ok(id);
    }
    let investigation = create(pool, name, None, None).await?;
    save(pool, TargetInput { id: None, investigation_id: investigation.id, kind: "person".into(), name: name.to_string(), aliases: Vec::new() }).await
}

/// Who in the Câmara, the Senado and the TSE (general elections of `years`, by default the
/// last two) may be the target, for the user to confirm.
#[tauri::command]
pub async fn find_official_matches<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, AppState>,
    target_id: String,
    years: Option<Vec<i32>>,
) -> CommandResult<Vec<OfficialMatch>> {
    use tauri::Manager;
    let pool = state.db_manager.pool();
    let (name,): (String,) = sqlx::query_as("SELECT name FROM targets WHERE id = ?").bind(&target_id).fetch_one(pool).await.map_err(err)?;
    let app_data_dir = app.path().app_data_dir().map_err(err)?;
    let years = years.unwrap_or_else(|| vec![2022, 2026]);
    official::find_matches(pool, &app_data_dir, &name, &years).await.map_err(err)
}

/// Links a target to the official records the user confirmed: one source for each, collected
/// from now on.
#[tauri::command]
pub async fn link_official(state: tauri::State<'_, AppState>, target_id: String, matches: Vec<OfficialMatch>) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    let (investigation_id,): (String,) = sqlx::query_as("SELECT investigation_id FROM targets WHERE id = ?").bind(&target_id).fetch_one(pool).await.map_err(err)?;
    for found in &matches {
        if !official::KINDS.contains(&found.kind.as_str()) {
            return Err(format!("Unknown official source: {}", found.kind));
        }
        let title = format!("{} · {}", found.label, found.detail);
        let inserted = insert_source(pool, &investigation_id, Some(&target_id), &found.kind, &found.id, Some(&title), default_every(&found.kind), "target").await;
        if let Err(e) = inserted {
            if !e.to_string().contains("UNIQUE") {
                return Err(e.to_string());
            }
        }
    }
    scheduler::wake();
    Ok(())
}

/// Removes an official source and what it brought: its documents (with their chunks), the facts
/// they show and the entities they describe (candidacies, amendments…), then the entities nothing
/// ties to anymore. A wrong link (a namesake) leaves nothing behind; the people followed stay.
pub async fn forget_official(pool: &SqlitePool, source_id: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let investigation: Option<(String,)> = sqlx::query_as("SELECT investigation_id FROM sources WHERE id = ?").bind(source_id).fetch_optional(&mut *tx).await?;
    let docs = "SELECT id FROM documents WHERE source_id = ?1";
    sqlx::query(&format!("DELETE FROM relations WHERE document_id IN ({docs})")).bind(source_id).execute(&mut *tx).await?;
    sqlx::query(&format!("DELETE FROM entities WHERE document_id IN ({docs}) AND key NOT LIKE 'target:%' AND origin != 'user'")).bind(source_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM documents WHERE source_id = ?").bind(source_id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM sources WHERE id = ?").bind(source_id).execute(&mut *tx).await?;
    if let Some((investigation,)) = investigation {
        sqlx::query(
            "DELETE FROM entities WHERE investigation_id = ? AND key NOT LIKE 'target:%' AND origin != 'user'
               AND NOT EXISTS (SELECT 1 FROM relations r WHERE r.from_id = entities.id OR r.to_id = entities.id)",
        )
        .bind(investigation)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Deletes a target: its searches stop (what they collected stays) and its official records go
/// with what they brought (forget_official).
#[tauri::command]
pub async fn delete_target(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    let official: Vec<(String,)> = sqlx::query_as(&format!(
        "SELECT id FROM sources WHERE target_id = ? AND kind IN ({})",
        official::KINDS.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ")
    ))
    .bind(&id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    for (source,) in official {
        forget_official(pool, &source).await.map_err(err)?;
    }
    let investigation: Option<(String,)> = sqlx::query_as("SELECT investigation_id FROM targets WHERE id = ?").bind(&id).fetch_optional(pool).await.map_err(err)?;
    sqlx::query("DELETE FROM targets WHERE id = ?").bind(id).execute(pool).await.map_err(err)?;
    if let Some((investigation,)) = investigation {
        crate::feed::reset(pool, &investigation).await.map_err(err)?;
    }
    Ok(())
}

// ===== Sources =====

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub investigation_id: String,
    pub target_id: Option<String>,
    pub kind: String,
    pub url: String,
    pub title: Option<String>,
    pub every_minutes: i64,
    pub next_run_at: String,
    pub last_run_at: Option<String>,
    pub last_error: Option<String>,
    /// active | paused | done (a source that runs once) | suggested (by the AI, waiting for
    /// approval) | rejected
    pub status: String,
    /// user | target | llm
    pub origin: String,
    /// The links of what it brings are followed.
    pub follow_links: bool,
    /// Why the AI suggested it.
    pub note: Option<String>,
    pub created_at: String,
    pub document_count: i64,
    pub queued: i64,
}

/// How often each kind runs by default, in minutes; 0: once.
fn default_every(kind: &str) -> i64 {
    match kind {
        "rss" => 60,
        "youtube_channel" => 180,
        "news_search" => 360,
        "social" => 180,
        // a few months of the term per run until it catches up, then yesterday's
        "pncp_prefeitura" => 60,
        "youtube_video" | "tse_candidato" => 0,
        // web_search, youtube_search, page and the official sources: daily
        _ => 1440,
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_source(
    pool: &SqlitePool,
    investigation_id: &str,
    target_id: Option<&str>,
    kind: &str,
    url: &str,
    title: Option<&str>,
    every_minutes: i64,
    origin: &str,
) -> Result<String, sqlx::Error> {
    let id = format!("src-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO sources (id, investigation_id, target_id, kind, url, title, every_minutes, next_run_at, origin, follow_links, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(investigation_id)
    .bind(target_id)
    .bind(kind)
    .bind(url)
    .bind(title)
    .bind(every_minutes)
    .bind(collect::schedule_time(Utc::now()))
    .bind(origin)
    // A site or feed someone added is a seed: its links are followed
    .bind(origin == "user" && matches!(kind, "page" | "rss"))
    .bind(now())
    .execute(pool)
    .await?;
    Ok(id)
}

#[tauri::command]
pub async fn list_sources(state: tauri::State<'_, AppState>, investigation_id: String) -> CommandResult<Vec<Source>> {
    let pool = state.db_manager.pool();
    let schedule = collect::schedule::load(pool, &investigation_id).await.map_err(err)?;
    let mut sources: Vec<Source> = sqlx::query_as(
        "SELECT s.id, s.investigation_id, s.target_id, s.kind, s.url, s.title, s.every_minutes, s.next_run_at, s.last_run_at,
                s.last_error, s.status, s.origin, s.follow_links, s.note, s.created_at,
                (SELECT COUNT(*) FROM documents d WHERE d.source_id = s.id) AS document_count,
                (SELECT COUNT(*) FROM crawl_queue q WHERE q.source_id = s.id AND q.status = 'pending') AS queued
         FROM sources s WHERE s.investigation_id = ? ORDER BY s.origin DESC, s.created_at",
    )
    .bind(&investigation_id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    // The pace it runs at: the investigation's for its kind, when set
    for source in &mut sources {
        source.every_minutes = schedule.every(&source.kind, source.every_minutes);
    }
    Ok(sources)
}

fn http_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let with_scheme = if url.contains("://") { url.to_string() } else { format!("https://{url}") };
    match url::Url::parse(&with_scheme) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() => Ok(parsed.to_string()),
        _ => Err(format!("Not a web address: {url}")),
    }
}

/// Adds a source: a page, a feed, a YouTube channel or video. A channel is kept as its RSS feed.
#[tauri::command]
pub async fn add_source(state: tauri::State<'_, AppState>, investigation_id: String, kind: String, url: String, title: Option<String>) -> CommandResult<String> {
    let pool = state.db_manager.pool();
    let (url, title) = match kind.as_str() {
        "page" | "rss" => (http_url(&url)?, title),
        "youtube_video" => (youtube::canonical_video_url(&http_url(&url)?).ok_or("Not a YouTube video address")?, title),
        "youtube_channel" => (youtube::channel_feed_url(&http_url(&url)?).await.map_err(err)?, title),
        "social" => (feeds::social_feed_url(&url)?, title),
        "news_search" | "youtube_search" | "web_search" | "gazette_search" => {
            let query = url.trim().to_string();
            if query.is_empty() {
                return Err("Type what to search for".into());
            }
            (query, title)
        }
        other => return Err(format!("Unknown source kind: {other}")),
    };
    let title = title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let id = insert_source(pool, &investigation_id, None, &kind, &url, title.as_deref(), default_every(&kind), "user")
        .await
        .map_err(|e| if e.to_string().contains("UNIQUE") { "This source is already in the investigation".to_string() } else { e.to_string() })?;
    scheduler::wake();
    Ok(id)
}

/// Pauses or resumes a source, changes how often it runs, or whether its links are followed.
#[tauri::command]
pub async fn update_source(
    state: tauri::State<'_, AppState>,
    id: String,
    status: Option<String>,
    every_minutes: Option<i64>,
    follow_links: Option<bool>,
) -> CommandResult<()> {
    if status.as_deref().is_some_and(|s| !matches!(s, "active" | "paused")) {
        return Err("Status must be active or paused".into());
    }
    sqlx::query("UPDATE sources SET status = COALESCE(?, status), every_minutes = COALESCE(?, every_minutes), follow_links = COALESCE(?, follow_links) WHERE id = ?")
        .bind(status)
        .bind(every_minutes.map(|m| m.max(15)))
        .bind(follow_links)
        .bind(id)
        .execute(state.db_manager.pool())
        .await
        .map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn delete_source(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    let kind: Option<(String,)> = sqlx::query_as("SELECT kind FROM sources WHERE id = ?").bind(&id).fetch_optional(pool).await.map_err(err)?;
    // An official record goes with what it brought (a namesake linked by mistake); others keep it
    if kind.is_some_and(|(k,)| official::KINDS.contains(&k.as_str())) {
        return forget_official(pool, &id).await.map_err(err);
    }
    sqlx::query("DELETE FROM sources WHERE id = ?").bind(id).execute(pool).await.map_err(err)?;
    Ok(())
}

#[tauri::command]
pub async fn run_source_now(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    sqlx::query("UPDATE sources SET next_run_at = ?, status = CASE WHEN status IN ('paused', 'suggested', 'rejected') THEN status ELSE 'active' END WHERE id = ?")
        .bind(collect::schedule_time(Utc::now()))
        .bind(&id)
        .execute(state.db_manager.pool())
        .await
        .map_err(err)?;
    // Whatever its investigation's schedule says
    collect::schedule::run_now(&id);
    scheduler::wake();
    Ok(())
}

#[tauri::command]
pub fn collect_status() -> scheduler::Progress {
    scheduler::snapshot()
}

// ===== Documents =====

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub id: String,
    pub investigation_id: String,
    pub source_id: Option<String>,
    pub url: String,
    pub kind: String,
    pub title: String,
    pub author: Option<String>,
    pub site: Option<String>,
    pub published_at: Option<String>,
    pub fetched_at: String,
    pub note: Option<String>,
    pub index_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
}

const SUMMARY_COLUMNS: &str = "id, investigation_id, source_id, url, kind, title, author, site, published_at, fetched_at, note, index_error";

/// Newest first (by publication date, else when collected).
#[tauri::command]
pub async fn list_documents(
    state: tauri::State<'_, AppState>,
    investigation_ids: Vec<String>,
    kind: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> CommandResult<Page<DocumentSummary>> {
    let pool = state.db_manager.pool();
    let ids = serde_json::to_string(&investigation_ids).map_err(err)?;
    let items = sqlx::query_as(&format!(
        "SELECT {SUMMARY_COLUMNS} FROM documents WHERE investigation_id IN (SELECT value FROM json_each(?1)) AND (?2 IS NULL OR kind = ?2)
         ORDER BY COALESCE(published_at, fetched_at) DESC LIMIT ?3 OFFSET ?4"
    ))
    .bind(&ids)
    .bind(&kind)
    .bind(limit.unwrap_or(50).clamp(1, 200))
    .bind(offset.unwrap_or(0).max(0))
    .fetch_all(pool)
    .await
    .map_err(err)?;
    let (total,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM documents WHERE investigation_id IN (SELECT value FROM json_each(?1)) AND (?2 IS NULL OR kind = ?2)")
        .bind(&ids)
        .bind(&kind)
        .fetch_one(pool)
        .await
        .map_err(err)?;
    Ok(Page { items, total })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    #[serde(flatten)]
    pub summary: DocumentSummary,
    pub text: String,
    pub segments: Option<Vec<youtube::TimedText>>,
    pub starred: bool,
    /// -1 disliked | 0 | 1 liked (the Feed)
    pub feedback: i64,
}

#[tauri::command]
pub async fn get_document(state: tauri::State<'_, AppState>, id: String) -> CommandResult<Document> {
    let pool = state.db_manager.pool();
    let summary: DocumentSummary = sqlx::query_as(&format!("SELECT {SUMMARY_COLUMNS} FROM documents WHERE id = ?"))
        .bind(&id)
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let (text, segments, starred, feedback): (String, Option<String>, bool, i64) =
        sqlx::query_as("SELECT text, segments, starred, feedback FROM documents WHERE id = ?").bind(&id).fetch_one(pool).await.map_err(err)?;
    Ok(Document { summary, text, segments: segments.and_then(|s| serde_json::from_str(&s).ok()), starred, feedback })
}

#[tauri::command]
pub async fn delete_document(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    collect::thumbs::forget(state.db_manager.pool(), "id = ?", &id).await;
    sqlx::query("DELETE FROM documents WHERE id = ?").bind(id).execute(state.db_manager.pool()).await.map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{entity, relate};
    use serde_json::json;

    #[tokio::test]
    async fn an_investigation_becomes_an_interest_and_back() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Maçonaria', '2026')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t', 'inv', 'topic', 'Maçonaria', '2026')").execute(&pool).await.unwrap();
        for kind in ["news_search", "gazette_search"] {
            insert_source(&pool, "inv", Some("t"), kind, "\"Maçonaria\"", Some("Maçonaria"), 360, "target").await.unwrap();
        }
        sqlx::query("INSERT INTO sources (id, investigation_id, kind, url, next_run_at, status, origin, created_at) VALUES ('s', 'inv', 'news_search', 'promessas maçons', '2026', 'suggested', 'llm', '2026')")
            .execute(&pool)
            .await
            .unwrap();
        let searches = || {
            let pool = pool.clone();
            async move { sqlx::query_as::<_, (String, String)>("SELECT kind, url FROM sources ORDER BY kind").fetch_all(&pool).await.unwrap() }
        };

        change_kind(&pool, "inv", "interest").await.unwrap();
        // Loose words, no gazettes, no suggestions made for politics
        assert_eq!(searches().await, vec![("news_search".to_string(), "Maçonaria".to_string())]);

        change_kind(&pool, "inv", "politics").await.unwrap();
        assert_eq!(
            searches().await,
            vec![("gazette_search".to_string(), "\"Maçonaria\"".to_string()), ("news_search".to_string(), "\"Maçonaria\"".to_string())]
        );
    }

    #[tokio::test]
    async fn a_debater_is_found_by_name_or_followed_in_a_new_investigation() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for sql in [
            "INSERT INTO investigations (id, name, created_at) VALUES ('small', 'Câmara', '2026'), ('big', 'Eleições', '2026')",
            "INSERT INTO targets (id, investigation_id, kind, name, aliases, created_at) VALUES
                ('t-small', 'small', 'person', 'Marina Exemplo', '[]', '2026'), ('t-big', 'big', 'person', 'Marina Exemplo', '[\"Marina do Bairro\"]', '2026')",
            "INSERT INTO documents (id, investigation_id, url, kind, title, text, fetched_at, content_hash) VALUES ('d', 'big', 'u', 'article', 't', 'x', '2026', 'h')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        // By name or alias, accents and case aside: the investigation with more records
        assert_eq!(follow_person(&pool, " marina exemplo ").await.unwrap(), "t-big");
        assert_eq!(follow_person(&pool, "MARINA DO BÁIRRO").await.unwrap(), "t-big");

        // Nobody follows Otávio: an investigation of his own, following him with searches
        let otavio = follow_person(&pool, "Otávio Fictício").await.unwrap();
        let (investigation, kind): (String, String) =
            sqlx::query_as("SELECT i.name, i.kind FROM targets t JOIN investigations i ON i.id = t.investigation_id WHERE t.id = ?").bind(&otavio).fetch_one(&pool).await.unwrap();
        assert_eq!((investigation.as_str(), kind.as_str()), ("Otávio Fictício", "politics"));
        let (searches,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sources WHERE target_id = ?").bind(&otavio).fetch_one(&pool).await.unwrap();
        assert!(searches >= 2);
        // And found the next time, without a second investigation
        assert_eq!(follow_person(&pool, "otavio ficticio").await.unwrap(), otavio);
        let (investigations,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM investigations").fetch_one(&pool).await.unwrap();
        assert_eq!(investigations, 3);
    }

    #[tokio::test]
    async fn forgetting_a_wrong_official_link_removes_what_it_brought() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'T', '2026')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t', 'inv', 'person', 'Ricardo Nunes', '2026')").execute(&pool).await.unwrap();
        for (id, kind) in [("wrong", "tse_candidato"), ("right", "camara_deputado")] {
            sqlx::query("INSERT INTO sources (id, investigation_id, target_id, kind, url, next_run_at, created_at) VALUES (?, 'inv', 't', ?, ?, '2026', '2026')")
                .bind(id)
                .bind(kind)
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let doc = |source: &'static str| {
            let pool = pool.clone();
            async move {
                collect::save_document(
                    &pool,
                    collect::NewDocument { investigation_id: "inv", source_id: Some(source), url: source, kind: "candidacy", title: "t", author: None, site: None, published_at: None, lang: None, text: source, segments: None, note: None },
                )
                .await
                .unwrap()
                .unwrap()
            }
        };
        let (wrong_doc, right_doc) = (doc("wrong").await, doc("right").await);
        let person = entity(&pool, "inv", "person", "target:t", "Ricardo Nunes", json!({}), None).await.unwrap();
        let candidacy = entity(&pool, "inv", "candidacy", "tse:cand:1", "Vereador", json!({}), Some(&wrong_doc)).await.unwrap();
        let party = entity(&pool, "inv", "party", "party:PXX", "PXX", json!({}), None).await.unwrap();
        let company = entity(&pool, "inv", "organization", "cnpj:1", "Gráfica", json!({}), None).await.unwrap();
        relate(&pool, "inv", &person, &candidacy, "CANDIDATE_IN", "", json!({}), Some(&wrong_doc)).await.unwrap();
        relate(&pool, "inv", &person, &party, "MEMBER_OF", "", json!({}), Some(&wrong_doc)).await.unwrap();
        relate(&pool, "inv", &person, &company, "SPENT_WITH", "2026-01", json!({}), Some(&right_doc)).await.unwrap();

        forget_official(&pool, "wrong").await.unwrap();

        let keys: Vec<(String,)> = sqlx::query_as("SELECT key FROM entities ORDER BY key").fetch_all(&pool).await.unwrap();
        assert_eq!(keys.into_iter().map(|(k,)| k).collect::<Vec<_>>(), ["cnpj:1", "target:t"]);
        let docs: Vec<(String,)> = sqlx::query_as("SELECT id FROM documents").fetch_all(&pool).await.unwrap();
        assert_eq!(docs, vec![(right_doc,)]);
        let (sources,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sources").fetch_one(&pool).await.unwrap();
        assert_eq!(sources, 1);
    }

    #[tokio::test]
    async fn deleting_an_investigation_takes_its_files_but_not_while_recording() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let dir = std::env::temp_dir().join(format!("voga-delete-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let (sound, picture) = (dir.join("rec.wav"), dir.join("doc.jpg"));
        std::fs::write(&sound, b"RIFF").unwrap();
        std::fs::write(&picture, b"jpg").unwrap();

        for statement in [
            "INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Mandato', '2026'), ('other', 'Outra', '2026')",
            "INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t', 'inv', 'person', 'Marina Exemplo', '2026')",
            "INSERT INTO live_sessions (id, investigation_id, title, started_at) VALUES ('debate', NULL, 'Debate', '2026'), ('elsewhere', 'other', 'Outra', '2026')",
            "INSERT INTO live_participants (session_id, position, name, target_id) VALUES ('debate', 0, 'Marina Exemplo', 't')",
            "INSERT INTO prefs (key, value) VALUES ('flowsint_sync:inv', '{}'), ('flowsint_sync:other', '{}')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO live_sessions (id, investigation_id, title, started_at, audio_path) VALUES ('rec', 'inv', 'Entrevista', '2026', ?)")
            .bind(sound.to_string_lossy().to_string())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO documents (id, investigation_id, url, kind, title, text, fetched_at, content_hash, image_path) VALUES ('d', 'inv', 'u', 'article', 't', 't', '2026', 'h', ?)")
            .bind(picture.to_string_lossy().to_string())
            .execute(&pool)
            .await
            .unwrap();

        // Its own session and a debate with someone in it hold it; another investigation's does not
        for session in ["rec", "debate"] {
            let refused = delete(&pool, "inv", Some(session)).await.unwrap_err().to_string();
            assert!(refused.starts_with(LIVE_IN_USE), "{refused}");
        }
        assert!(sound.exists() && picture.exists());
        delete(&pool, "inv", Some("elsewhere")).await.unwrap();

        assert!(!sound.exists() && !picture.exists());
        let count = |sql: &'static str| {
            let pool = pool.clone();
            async move { sqlx::query_as::<_, (i64,)>(sql).fetch_one(&pool).await.unwrap().0 }
        };
        assert_eq!(count("SELECT COUNT(*) FROM investigations").await, 1);
        assert_eq!(count("SELECT COUNT(*) FROM documents").await, 0);
        assert_eq!(count("SELECT COUNT(*) FROM prefs WHERE key LIKE 'flowsint_sync:%'").await, 1);
        // The debate stays, with the person's name and no link to the deleted investigation
        assert_eq!(count("SELECT COUNT(*) FROM live_participants WHERE session_id = 'debate' AND target_id IS NULL").await, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
