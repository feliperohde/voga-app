//! The collection worker: every minute (or when woken) it runs the sources that are due,
//! fetches queued pages and videos, and embeds passages still without a vector. One worker,
//! one request at a time, so sites are never hit in parallel. Each investigation's schedule
//! (schedule.rs) says whether it may collect now and at what pace.
//!
//! ponytail: sequential on purpose; a per-host pool would collect faster if queues grow long.

use chrono::{Duration, Utc};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::Notify;

use super::schedule::{self, Gate, Schedule, OPEN_ITEM, OPEN_SOURCE};
use super::{process_item, run_source, schedule_time, QueueItem, SourceRow};
use std::collections::HashMap;
use crate::llm::ConfiguredChatModel;
use crate::rag::embeddings::EmbeddingProvider;
use crate::rag::indexer;
use crate::state::AppState;
use crate::{analyze, prefs};

pub const PROGRESS_EVENT: &str = "collect-progress";
const TICK: std::time::Duration = std::time::Duration::from_secs(60);
/// Queued items fetched per tick, so due sources are not kept waiting behind a long queue.
const ITEMS_PER_TICK: usize = 30;
const MAX_ATTEMPTS: i64 = 3;
const EMBED_BATCH: i64 = 64;
/// Documents the AI reads per tick (each is one or more requests), when nothing waits to be collected.
const ANALYZE_PER_TICK: i64 = 2;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub running: bool,
    /// What is being collected now.
    pub current: Option<String>,
    /// Items waiting in the queue.
    pub queued: i64,
    /// New documents in the current (or last) run.
    pub collected: usize,
    /// The AI is reading documents (`current` is the one).
    pub analyzing: bool,
    /// Whose source or page `current` is.
    pub investigation_id: Option<String>,
    /// Per investigation, how far this run is (it lasts until nothing is left to collect).
    pub runs: Vec<RunCount>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCount {
    pub investigation_id: String,
    /// Sources run and pages fetched.
    pub done: i64,
    /// `done` plus what is still due or queued.
    pub total: i64,
}

/// Counts one more done for `done_for` and sets each investigation's total from what it has left.
fn tally(runs: &mut Vec<RunCount>, left: &HashMap<String, i64>, done_for: Option<&str>) {
    if let Some(id) = done_for {
        match runs.iter_mut().find(|r| r.investigation_id == id) {
            Some(run) => run.done += 1,
            None => runs.push(RunCount { investigation_id: id.to_string(), done: 1, total: 0 }),
        }
    }
    let new: Vec<String> = left.keys().filter(|id| !runs.iter().any(|r| &r.investigation_id == *id)).cloned().collect();
    runs.extend(new.into_iter().map(|investigation_id| RunCount { investigation_id, ..Default::default() }));
    for run in runs.iter_mut() {
        run.total = run.done + left.get(&run.investigation_id).copied().unwrap_or(0);
    }
    runs.retain(|r| r.total > 0);
    runs.sort_by(|a, b| a.investigation_id.cmp(&b.investigation_id));
}

fn progress() -> &'static Mutex<Progress> {
    static PROGRESS: OnceLock<Mutex<Progress>> = OnceLock::new();
    PROGRESS.get_or_init(Default::default)
}

fn wake_signal() -> &'static Notify {
    static WAKE: OnceLock<Notify> = OnceLock::new();
    WAKE.get_or_init(Notify::new)
}

/// Runs a tick now instead of waiting for the next minute.
pub fn wake() {
    wake_signal().notify_one();
}

pub fn snapshot() -> Progress {
    progress().lock().map(|p| p.clone()).unwrap_or_default()
}

fn report<R: Runtime>(app: &AppHandle<R>, update: impl FnOnce(&mut Progress)) {
    if let Ok(mut progress) = progress().lock() {
        update(&mut progress);
        let _ = app.emit(PROGRESS_EVENT, progress.clone());
    }
}

pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(e) = tick(&app).await {
                log::error!("Collect: tick failed: {e}");
                report(&app, |p| p.running = false);
            }
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = wake_signal().notified() => {}
            }
        }
    });
}

/// What each investigation has left in this run: the due sources not run yet and the queued
/// pages ready to fetch.
async fn left(pool: &sqlx::SqlitePool, gate: &Gate, due: &[SourceRow]) -> HashMap<String, i64> {
    let rows: Vec<(String, i64)> = sqlx::query_as(&format!(
        "SELECT investigation_id, COUNT(*) FROM crawl_queue WHERE status = 'pending' AND next_attempt_at <= ?3 AND {OPEN_ITEM} GROUP BY investigation_id"
    ))
    .bind(&gate.closed)
    .bind(&gate.asked)
    .bind(schedule_time(Utc::now()))
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut left: HashMap<String, i64> = rows.into_iter().collect();
    for source in due {
        *left.entry(source.investigation_id.clone()).or_default() += 1;
    }
    left
}

/// Pages waiting that the schedules let through (a closed investigation's wait apart).
async fn queued(pool: &sqlx::SqlitePool, gate: &Gate) -> i64 {
    sqlx::query_as::<_, (i64,)>(&format!("SELECT COUNT(*) FROM crawl_queue WHERE status = 'pending' AND {OPEN_ITEM}"))
        .bind(&gate.closed)
        .bind(&gate.asked)
        .fetch_one(pool)
        .await
        .map_or(0, |(n,)| n)
}

async fn tick<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let state = app.state::<AppState>();
    let pool = state.db_manager.pool();
    let app_data_dir = app.path().app_data_dir()?;
    let embedder = indexer::embedder(pool).await?;
    let now = || schedule_time(Utc::now());
    schedule::settle(pool).await;
    let gate = schedule::gate(pool).await?;

    let due: Vec<SourceRow> = sqlx::query_as(&format!(
        "SELECT id, investigation_id, target_id, kind, url, title, etag, last_modified, follow_links FROM sources
         WHERE status = 'active' AND next_run_at <= ?3 AND {OPEN_SOURCE} ORDER BY next_run_at LIMIT 20"
    ))
    .bind(&gate.closed)
    .bind(&gate.asked)
    .bind(now())
    .fetch_all(pool)
    .await?;
    let pending = queued(pool, &gate).await;
    if due.is_empty() && pending == 0 {
        if !snapshot().runs.is_empty() {
            report(app, |p| p.runs.clear());
        }
        indexer::embed_missing(pool, &embedder, EMBED_BATCH).await.ok();
        score(pool, &embedder).await;
        crate::enrich::step(pool, &app_data_dir).await;
        if let Err(e) = super::thumbs::step(pool, &app_data_dir).await {
            log::warn!("Thumbs: {e}");
        }
        return analyze_step(app, pool, &embedder).await;
    }
    let to_do = left(pool, &gate, &due).await;
    report(app, |p| {
        *p = Progress { running: true, queued: pending, runs: std::mem::take(&mut p.runs), ..Default::default() };
        tally(&mut p.runs, &to_do, None);
    });

    let mut schedules: HashMap<String, Schedule> = HashMap::new();
    for (i, source) in due.iter().enumerate() {
        report(app, |p| {
            p.current = Some(source.title.clone().unwrap_or_else(|| source.url.clone()));
            p.investigation_id = Some(source.investigation_id.clone());
        });
        let result = run_source(pool, &app_data_dir, &embedder, source).await;
        let error = result.as_ref().err().map(|e| e.to_string());
        if let Some(e) = &error {
            log::warn!("Collect: source {} ({}) failed: {e}", source.id, source.kind);
        }
        let new = *result.as_ref().unwrap_or(&0);
        if !schedules.contains_key(&source.investigation_id) {
            schedules.insert(source.investigation_id.clone(), schedule::load(pool, &source.investigation_id).await?);
        }
        let pace = schedules[&source.investigation_id].every_for(&source.kind);
        sqlx::query(
            // every_minutes 0 runs once: done after it works, tried again in an hour if not;
            // otherwise at the investigation's pace for its kind, or its own
            "UPDATE sources SET last_run_at = ?1, last_error = ?2,
                status = CASE WHEN every_minutes = 0 AND ?2 IS NULL THEN 'done' ELSE status END,
                next_run_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+' || CASE every_minutes WHEN 0 THEN 60 ELSE COALESCE(?4, every_minutes) END || ' minutes')
             WHERE id = ?3",
        )
        .bind(now())
        .bind(&error)
        .bind(&source.id)
        .bind(pace)
        .execute(pool)
        .await?;
        let pending = queued(pool, &gate).await;
        let to_do = left(pool, &gate, &due[i + 1..]).await;
        report(app, |p| {
            p.queued = pending;
            if matches!(source.kind.as_str(), "page" | "youtube_video") || super::official::KINDS.contains(&source.kind.as_str()) {
                p.collected += new;
            }
            tally(&mut p.runs, &to_do, Some(&source.investigation_id));
        });
    }

    for _ in 0..ITEMS_PER_TICK {
        let item: Option<QueueItem> = sqlx::query_as(&format!(
            "SELECT id, investigation_id, source_id, url, kind, title, published_at, attempts, via, depth FROM crawl_queue
             WHERE status = 'pending' AND next_attempt_at <= ?3 AND {OPEN_ITEM} ORDER BY id LIMIT 1"
        ))
        .bind(&gate.closed)
        .bind(&gate.asked)
        .bind(now())
        .fetch_optional(pool)
        .await?;
        let Some(item) = item else { break };
        report(app, |p| {
            p.current = Some(item.title.clone().unwrap_or_else(|| item.url.clone()));
            p.investigation_id = Some(item.investigation_id.clone());
        });
        let result = process_item(pool, &app_data_dir, &embedder, &item).await;
        let (status, error, retry_at) = match &result {
            Ok(_) => ("done", None, None),
            Err(e) if e.to_string().starts_with("skip:") => ("skipped", Some(e.to_string()), None),
            Err(e) if item.attempts + 1 >= MAX_ATTEMPTS => ("failed", Some(e.to_string()), None),
            Err(e) => ("pending", Some(e.to_string()), Some(schedule_time(Utc::now() + Duration::minutes(30 * (item.attempts + 1))))),
        };
        sqlx::query("UPDATE crawl_queue SET status = ?, error = ?, attempts = attempts + ?, next_attempt_at = COALESCE(?, next_attempt_at) WHERE id = ?")
            .bind(status)
            .bind(&error)
            .bind(i64::from(result.is_err()))
            .bind(retry_at)
            .bind(item.id)
            .execute(pool)
            .await?;
        let pending = queued(pool, &gate).await;
        let to_do = left(pool, &gate, &[]).await;
        report(app, |p| {
            p.queued = pending;
            p.collected += usize::from(matches!(result, Ok(true)));
            tally(&mut p.runs, &to_do, Some(&item.investigation_id));
        });
    }

    indexer::embed_missing(pool, &embedder, EMBED_BATCH).await.ok();
    score(pool, &embedder).await;
    report(app, |p| {
        p.running = false;
        p.current = None;
        p.investigation_id = None;
    });
    // More ready to fetch: go on without the minute's pause
    let ready: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM crawl_queue WHERE status = 'pending' AND next_attempt_at <= ?3 AND {OPEN_ITEM}"))
        .bind(&gate.closed)
        .bind(&gate.asked)
        .bind(now())
        .fetch_one(pool)
        .await?;
    if ready.0 > 0 {
        wake();
        return Ok(());
    }
    report(app, |p| p.runs.clear());
    analyze_step(app, pool, &embedder).await
}

/// The Feed's relevance of what was just embedded.
async fn score(pool: &sqlx::SqlitePool, embedder: &dyn EmbeddingProvider) {
    if let Err(e) = crate::feed::score_step(pool, embedder).await {
        log::warn!("Feed: scoring failed: {e}");
    }
}

/// Lets the AI read a few documents, when it is on and a model is set up; wakes again if more wait.
async fn analyze_step<R: Runtime>(app: &AppHandle<R>, pool: &sqlx::SqlitePool, embedder: &dyn EmbeddingProvider) -> anyhow::Result<()> {
    // A live session's checks come first; what waits is read once it is over
    if crate::live::session::busy() {
        return Ok(());
    }
    if !prefs::analysis_enabled(pool).await || analyze::pending(pool).await? == 0 {
        // Nothing left to read: time for the AI to suggest searches for what is still unproven
        if let Err(e) = crate::discover::suggest::step(pool).await {
            log::warn!("Discover: suggestions failed: {e}");
        }
        return Ok(());
    }
    // No model configured yet: nothing to do until there is one
    let Ok(llm) = ConfiguredChatModel::from_settings(pool).await.map(ConfiguredChatModel::deterministic) else { return Ok(()) };
    report(app, |p| {
        p.running = true;
        p.analyzing = true;
    });
    let result = analyze::run_batch(pool, embedder, &llm, ANALYZE_PER_TICK, |title| report(app, |p| p.current = Some(title.to_string()))).await;
    report(app, |p| {
        p.running = false;
        p.analyzing = false;
        p.current = None;
    });
    result?;
    if analyze::pending(pool).await? > 0 {
        wake();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_counts_what_each_investigation_did_and_has_left() {
        let mut runs = Vec::new();
        tally(&mut runs, &HashMap::from([("a".to_string(), 3), ("b".to_string(), 1)]), None);
        assert_eq!(runs, vec![RunCount { investigation_id: "a".into(), done: 0, total: 3 }, RunCount { investigation_id: "b".into(), done: 0, total: 1 }]);
        // A source of "a" ran and queued 5 pages
        tally(&mut runs, &HashMap::from([("a".to_string(), 7), ("b".to_string(), 1)]), Some("a"));
        assert_eq!(runs[0], RunCount { investigation_id: "a".into(), done: 1, total: 8 });
        // "b" is done: it stays at 1/1 until the run ends
        tally(&mut runs, &HashMap::from([("a".to_string(), 7)]), Some("b"));
        assert_eq!(runs[1], RunCount { investigation_id: "b".into(), done: 1, total: 1 });
    }
}
