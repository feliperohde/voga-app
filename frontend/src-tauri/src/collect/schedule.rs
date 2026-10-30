//! When each investigation collects: paused or not, how often each group of sources runs, and
//! the local hours and days it may go online. Kept as JSON in `investigations.schedule`. The
//! worker leaves out the investigations that may not collect now (their due sources and queued
//! pages wait for the next opening) and runs each source at its group's pace instead of its own.
//! "Run now" on a source goes through anyway, with the pages it queues.

use anyhow::{bail, Result};
use chrono::{Datelike, NaiveDateTime, NaiveTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use super::{official, scheduler, schedule_time};
use crate::state::AppState;

/// Least and most time between two runs of a source, in minutes.
const MIN_EVERY: i64 = 15;
const MAX_EVERY: i64 = 30 * 1440;

/// Sources of investigations that may collect now, or asked to run now (?1: the closed
/// investigations, ?2: the sources asked, both JSON arrays).
pub const OPEN_SOURCE: &str = "(investigation_id NOT IN (SELECT value FROM json_each(?1)) OR id IN (SELECT value FROM json_each(?2)))";
/// Queued pages of investigations that may collect now, or brought by a source asked to run now.
pub const OPEN_ITEM: &str = "(investigation_id NOT IN (SELECT value FROM json_each(?1)) OR source_id IN (SELECT value FROM json_each(?2)))";

/// Minutes between runs for each group of sources; None keeps each source's own pace.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Every {
    /// RSS feeds and social profiles.
    pub feeds: Option<i64>,
    /// News and web searches.
    pub news: Option<i64>,
    /// YouTube searches and channels.
    pub videos: Option<i64>,
    /// Web pages.
    pub pages: Option<i64>,
    /// Municipal gazettes.
    pub gazettes: Option<i64>,
    /// Official records.
    pub official: Option<i64>,
}

/// Local hours, "HH:MM": from `start` to `end`, across midnight when `end` comes first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Window {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Schedule {
    pub paused: bool,
    pub every: Every,
    /// None: any time.
    pub window: Option<Window>,
    /// Days of the week it may collect, 0 = Sunday; empty: every day.
    pub days: Vec<u32>,
}

fn time(text: &str) -> chrono::ParseResult<NaiveTime> {
    NaiveTime::parse_from_str(text, "%H:%M")
}

impl Schedule {
    /// The investigation's pace for a source of `kind`, when it sets one for its group.
    pub fn every_for(&self, kind: &str) -> Option<i64> {
        let every = &self.every;
        match kind {
            "rss" | "social" => every.feeds,
            "news_search" | "web_search" => every.news,
            "youtube_search" | "youtube_channel" => every.videos,
            "page" => every.pages,
            "gazette_search" => every.gazettes,
            // The city hall catches up on its term an hour at a time, whatever the pace
            kind if kind != official::pncp::KIND && official::KINDS.contains(&kind) => every.official,
            _ => None,
        }
    }

    /// How often a source runs: its group's pace, or its own. A source that runs once (0) still does.
    pub fn every(&self, kind: &str, own: i64) -> i64 {
        if own == 0 {
            0
        } else {
            self.every_for(kind).unwrap_or(own)
        }
    }

    /// Whether the investigation may collect at `now` (local time).
    pub fn open_at(&self, now: NaiveDateTime) -> bool {
        if self.paused || (!self.days.is_empty() && !self.days.contains(&now.weekday().num_days_from_sunday())) {
            return false;
        }
        let Some(window) = &self.window else { return true };
        let (Ok(start), Ok(end)) = (time(&window.start), time(&window.end)) else { return true };
        let now = now.time();
        match start.cmp(&end) {
            std::cmp::Ordering::Equal => true,
            std::cmp::Ordering::Less => start <= now && now < end,
            std::cmp::Ordering::Greater => now >= start || now < end,
        }
    }

    /// The first minute after `now` it may collect again, within a week; None while paused.
    pub fn next_open(&self, now: NaiveDateTime) -> Option<NaiveDateTime> {
        if self.paused {
            return None;
        }
        let start = now.with_second(0)?.with_nanosecond(0)?;
        (1..=8 * 24 * 60).map(|m| start + chrono::Duration::minutes(m)).find(|t| self.open_at(*t))
    }

    fn checked(self) -> Result<Schedule> {
        if let Some(window) = &self.window {
            if time(&window.start).is_err() || time(&window.end).is_err() {
                bail!("Hours must be HH:MM: {} – {}", window.start, window.end);
            }
        }
        let clamp = |minutes: Option<i64>| minutes.map(|m| m.clamp(MIN_EVERY, MAX_EVERY));
        let every = self.every;
        let mut days: Vec<u32> = self.days.into_iter().filter(|d| *d < 7).collect();
        days.sort_unstable();
        days.dedup();
        if days.len() == 7 {
            days.clear();
        }
        Ok(Schedule {
            paused: self.paused,
            every: Every {
                feeds: clamp(every.feeds),
                news: clamp(every.news),
                videos: clamp(every.videos),
                pages: clamp(every.pages),
                gazettes: clamp(every.gazettes),
                official: clamp(every.official),
            },
            window: self.window,
            days,
        })
    }
}

pub async fn load(pool: &SqlitePool, investigation_id: &str) -> Result<Schedule> {
    let row: Option<(String,)> = sqlx::query_as("SELECT schedule FROM investigations WHERE id = ?").bind(investigation_id).fetch_optional(pool).await?;
    Ok(row.and_then(|(json,)| serde_json::from_str(&json).ok()).unwrap_or_default())
}

/// What the worker may collect now: the closed investigations and the sources asked to run now,
/// as JSON arrays for OPEN_SOURCE and OPEN_ITEM.
pub struct Gate {
    pub closed: String,
    pub asked: String,
}

pub async fn gate(pool: &SqlitePool) -> Result<Gate> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, schedule FROM investigations WHERE schedule != '{}'").fetch_all(pool).await?;
    let now = chrono::Local::now().naive_local();
    let closed: Vec<String> =
        rows.into_iter().filter(|(_, json)| !serde_json::from_str::<Schedule>(json).unwrap_or_default().open_at(now)).map(|(id, _)| id).collect();
    let asked: Vec<String> = asked().lock().map(|set| set.iter().cloned().collect()).unwrap_or_default();
    Ok(Gate { closed: serde_json::to_string(&closed)?, asked: serde_json::to_string(&asked)? })
}

// ===== Run now =====

fn asked() -> &'static Mutex<HashSet<String>> {
    static ASKED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    ASKED.get_or_init(Default::default)
}

/// A source someone asked to run now: it runs, and the pages it queues are fetched, whatever its
/// investigation's schedule.
pub fn run_now(source_id: &str) {
    if let Ok(mut set) = asked().lock() {
        set.insert(source_id.to_string());
    }
}

/// Forgets the sources asked that ran and whose queued pages were all fetched.
pub async fn settle(pool: &SqlitePool) {
    let ids: Vec<String> = asked().lock().map(|set| set.iter().cloned().collect()).unwrap_or_default();
    for id in ids {
        let waiting: Result<(bool,), _> = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM sources WHERE id = ?1 AND status = 'active' AND next_run_at <= ?2)
                 OR EXISTS(SELECT 1 FROM crawl_queue WHERE source_id = ?1 AND status = 'pending')",
        )
        .bind(&id)
        .bind(schedule_time(Utc::now()))
        .fetch_one(pool)
        .await;
        if matches!(waiting, Ok((false,))) {
            if let Ok(mut set) = asked().lock() {
                set.remove(&id);
            }
        }
    }
}

/// Saves an investigation's schedule and moves each source's next run to its new pace, counted
/// from its last run (a source that never ran is already due).
pub async fn save(pool: &SqlitePool, investigation_id: &str, schedule: Schedule) -> Result<()> {
    let schedule = schedule.checked()?;
    let json = if schedule == Schedule::default() { "{}".to_string() } else { serde_json::to_string(&schedule)? };
    sqlx::query("UPDATE investigations SET schedule = ? WHERE id = ?").bind(json).bind(investigation_id).execute(pool).await?;
    let sources: Vec<(String, String, i64, String)> =
        sqlx::query_as("SELECT id, kind, every_minutes, last_run_at FROM sources WHERE investigation_id = ? AND every_minutes > 0 AND last_run_at IS NOT NULL")
            .bind(investigation_id)
            .fetch_all(pool)
            .await?;
    for (id, kind, own, last_run) in sources {
        let Ok(last_run) = chrono::DateTime::parse_from_rfc3339(&last_run) else { continue };
        let next = last_run.with_timezone(&Utc) + chrono::Duration::minutes(schedule.every(&kind, own));
        sqlx::query("UPDATE sources SET next_run_at = ? WHERE id = ?").bind(schedule_time(next)).bind(&id).execute(pool).await?;
    }
    Ok(())
}

// ===== Commands =====

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleView {
    pub schedule: Schedule,
    /// It may collect right now.
    pub open: bool,
}

#[tauri::command]
pub async fn get_collect_schedule(state: tauri::State<'_, AppState>, investigation_id: String) -> Result<ScheduleView, String> {
    let schedule = load(state.db_manager.pool(), &investigation_id).await.map_err(|e| e.to_string())?;
    let open = schedule.open_at(chrono::Local::now().naive_local());
    Ok(ScheduleView { schedule, open })
}

#[tauri::command]
pub async fn save_collect_schedule(state: tauri::State<'_, AppState>, investigation_id: String, schedule: Schedule) -> Result<(), String> {
    save(state.db_manager.pool(), &investigation_id, schedule).await.map_err(|e| e.to_string())?;
    scheduler::wake();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        // 2026-11-01 is a Sunday
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap().and_hms_opt(hour, minute, 0).unwrap()
    }

    fn window(start: &str, end: &str) -> Option<Window> {
        Some(Window { start: start.into(), end: end.into() })
    }

    #[test]
    fn hours_days_and_pause() {
        let night = Schedule { window: window("22:00", "06:00"), ..Default::default() };
        assert!(night.open_at(at(2, 23, 0)) && night.open_at(at(2, 5, 59)));
        assert!(!night.open_at(at(2, 6, 0)) && !night.open_at(at(2, 12, 0)));
        let office = Schedule { window: window("08:00", "18:00"), days: vec![1, 2, 3, 4, 5], ..Default::default() };
        assert!(office.open_at(at(2, 9, 0)), "Monday morning");
        assert!(!office.open_at(at(2, 18, 0)) && !office.open_at(at(1, 9, 0)), "Monday evening, Sunday");
        assert!(Schedule { window: window("10:00", "10:00"), ..Default::default() }.open_at(at(1, 3, 0)), "the same hour twice: any time");
        assert!(!Schedule { paused: true, ..Default::default() }.open_at(at(2, 9, 0)));
        assert!(Schedule::default().open_at(at(1, 3, 0)));
    }

    #[test]
    fn when_it_opens_again() {
        let office = Schedule { window: window("08:00", "18:00"), days: vec![1, 2, 3, 4, 5], ..Default::default() };
        assert_eq!(office.next_open(at(2, 19, 30)), Some(at(3, 8, 0)), "Monday evening: Tuesday morning");
        assert_eq!(office.next_open(at(7, 10, 0)), Some(at(9, 8, 0)), "Saturday: Monday morning");
        assert_eq!(Schedule { paused: true, ..Default::default() }.next_open(at(2, 9, 0)), None);
    }

    #[test]
    fn a_group_pace_replaces_each_source_own() {
        let schedule = Schedule { every: Every { news: Some(30), official: Some(10080), ..Default::default() }, ..Default::default() };
        assert_eq!(schedule.every("news_search", 360), 30);
        assert_eq!(schedule.every("web_search", 1440), 30);
        assert_eq!(schedule.every("camara_deputado", 1440), 10080);
        assert_eq!(schedule.every("pncp_prefeitura", 60), 60, "the city hall keeps catching up hourly");
        assert_eq!(schedule.every("tse_candidato", 0), 0, "once stays once");
        assert_eq!(schedule.every("rss", 60), 60, "no pace set for feeds");
    }

    #[test]
    fn saved_schedules_are_checked() {
        let checked = Schedule { every: Every { feeds: Some(1), pages: Some(999_999), ..Default::default() }, days: vec![0, 1, 2, 3, 4, 5, 6, 6], ..Default::default() }.checked().unwrap();
        assert_eq!((checked.every.feeds, checked.every.pages), (Some(MIN_EVERY), Some(MAX_EVERY)));
        assert!(checked.days.is_empty(), "every day");
        assert!(Schedule { window: window("25:00", "06:00"), ..Default::default() }.checked().is_err());
    }

    #[tokio::test]
    async fn closed_investigations_wait_unless_a_source_was_asked_to_run_now() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for id in ["open", "shut"] {
            sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES (?, ?, '2026')").bind(id).bind(id).execute(&pool).await.unwrap();
            for source in ["a", "b"] {
                sqlx::query("INSERT INTO sources (id, investigation_id, kind, url, every_minutes, next_run_at, last_run_at, created_at) VALUES (?, ?, 'news_search', ?, 360, '2026-01-01T00:00:00Z', '2026-10-29T12:00:00Z', '2026')")
                    .bind(format!("{id}-{source}"))
                    .bind(id)
                    .bind(format!("q {id} {source}"))
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            sqlx::query("INSERT INTO crawl_queue (investigation_id, source_id, url, kind, next_attempt_at, created_at) VALUES (?, ?, ?, 'page', '2026', '2026')")
                .bind(id)
                .bind(format!("{id}-a"))
                .bind(format!("https://example.com/{id}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        save(&pool, "shut", Schedule { paused: true, every: Every { news: Some(30), ..Default::default() }, ..Default::default() }).await.unwrap();
        let (next,): (String,) = sqlx::query_as("SELECT next_run_at FROM sources WHERE id = 'shut-a'").fetch_one(&pool).await.unwrap();
        assert_eq!(next, "2026-10-29T12:30:00Z", "the new pace counts from the last run");

        let open = |gate: Gate| {
            let pool = pool.clone();
            async move {
                let sources: Vec<(String,)> = sqlx::query_as(&format!("SELECT id FROM sources WHERE {OPEN_SOURCE} ORDER BY id")).bind(&gate.closed).bind(&gate.asked).fetch_all(&pool).await.unwrap();
                let items: Vec<(String,)> = sqlx::query_as(&format!("SELECT url FROM crawl_queue WHERE {OPEN_ITEM} ORDER BY url")).bind(&gate.closed).bind(&gate.asked).fetch_all(&pool).await.unwrap();
                (sources.into_iter().map(|(s,)| s).collect::<Vec<_>>(), items.len())
            }
        };
        assert_eq!(open(gate(&pool).await.unwrap()).await, (vec!["open-a".to_string(), "open-b".to_string()], 1));
        run_now("shut-a");
        assert_eq!(open(gate(&pool).await.unwrap()).await, (vec!["open-a".to_string(), "open-b".to_string(), "shut-a".to_string()], 2));
        // Ran, and its page was fetched: the schedule holds again
        sqlx::query("UPDATE sources SET next_run_at = '2999-01-01T00:00:00Z' WHERE id = 'shut-a'").execute(&pool).await.unwrap();
        sqlx::query("UPDATE crawl_queue SET status = 'done' WHERE source_id = 'shut-a'").execute(&pool).await.unwrap();
        settle(&pool).await;
        assert_eq!(open(gate(&pool).await.unwrap()).await.0.len(), 2);

        save(&pool, "shut", Schedule::default()).await.unwrap();
        let (json,): (String,) = sqlx::query_as("SELECT schedule FROM investigations WHERE id = 'shut'").fetch_one(&pool).await.unwrap();
        assert_eq!(json, "{}");
    }
}
