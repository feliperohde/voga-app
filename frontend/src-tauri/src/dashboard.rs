//! The Home screen: each investigation's state at a glance (collecting, indexing, schedule,
//! failures, what the AI found and what a person confirmed, the room it takes on this computer).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::analyze;
use crate::collect::schedule::Schedule;
use crate::state::AppState;

/// How long a disk-usage estimate is kept.
const DISK_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize)]
pub struct Tally {
    pub confirmed: i64,
    pub suggested: i64,
}

/// The latest thing found about the people an investigation follows.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Highlight {
    /// contradiction | fulfillment | check_false | check_true | scandal
    pub kind: String,
    pub text: String,
    pub person: Option<String>,
    /// When it was found.
    pub date: String,
    /// confirmed | suggested
    pub status: String,
    pub document_id: Option<String>,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleState {
    pub schedule: Schedule,
    /// It may collect now.
    pub open: bool,
    /// When it may collect again (local time), while closed by its hours or days.
    pub next_open: Option<String>,
    /// The next source due.
    pub next_run_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationStatus {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// Who or what it follows.
    pub targets: Vec<String>,
    pub sources: i64,
    /// Sources whose last run failed (paused and rejected ones apart).
    pub failing: i64,
    pub documents: i64,
    /// News, pages, videos and posts not read yet.
    pub unread: i64,
    pub last_update: Option<String>,
    /// Documents still to be indexed for search.
    pub indexing: i64,
    /// Pages waiting to be fetched.
    pub queued: i64,
    pub schedule: ScheduleState,
    /// Promises and statements a person confirmed contradicted, or that wait for review.
    pub contradictions: Tally,
    pub fulfilled: Tally,
    /// What the AI found that waits for review (findings and suggested sources).
    pub insights: i64,
    pub latest: Option<Highlight>,
    /// Estimate of the room it takes (text, index, pictures, recordings).
    pub disk_bytes: i64,
}

#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    kind: String,
    schedule: String,
    sources: i64,
    failing: i64,
    suggested_sources: i64,
    suggested_scandals: i64,
    documents: i64,
    unread: i64,
    last_update: Option<String>,
    queued: i64,
    next_run_at: Option<String>,
}

pub async fn statuses(pool: &SqlitePool, embedding_model: &str) -> Result<Vec<InvestigationStatus>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT i.id, i.name, i.kind, i.schedule,
            (SELECT COUNT(*) FROM sources s WHERE s.investigation_id = i.id AND s.status NOT IN ('suggested', 'rejected')) AS sources,
            (SELECT COUNT(*) FROM sources s WHERE s.investigation_id = i.id AND s.last_error IS NOT NULL AND s.status NOT IN ('paused', 'suggested', 'rejected')) AS failing,
            (SELECT COUNT(*) FROM sources s WHERE s.investigation_id = i.id AND s.status = 'suggested') AS suggested_sources,
            (SELECT COUNT(*) FROM relations r WHERE r.investigation_id = i.id AND r.label = 'INVOLVED_IN' AND r.status = 'suggested') AS suggested_scandals,
            (SELECT COUNT(*) FROM documents d WHERE d.investigation_id = i.id) AS documents,
            (SELECT COUNT(*) FROM documents d WHERE d.investigation_id = i.id AND d.read_at IS NULL AND d.kind IN ('article', 'page', 'video', 'post')) AS unread,
            (SELECT MAX(fetched_at) FROM documents d WHERE d.investigation_id = i.id) AS last_update,
            (SELECT COUNT(*) FROM crawl_queue q WHERE q.investigation_id = i.id AND q.status = 'pending') AS queued,
            (SELECT MIN(next_run_at) FROM sources s WHERE s.investigation_id = i.id AND s.status = 'active') AS next_run_at
         FROM investigations i ORDER BY i.created_at",
    )
    .fetch_all(pool)
    .await?;
    let now = chrono::Local::now().naive_local();
    let mut statuses = Vec::with_capacity(rows.len());
    for row in rows {
        let targets: Vec<(String,)> = sqlx::query_as("SELECT name FROM targets WHERE investigation_id = ? ORDER BY created_at").bind(&row.id).fetch_all(pool).await?;
        let (indexing,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM documents d WHERE d.investigation_id = ?1
               AND ((d.indexed_at IS NULL AND d.index_error IS NULL)
                    OR EXISTS (SELECT 1 FROM rag_chunks c WHERE c.document_id = d.id AND (c.embedding IS NULL OR (?2 != '' AND c.embedding_model != ?2))))",
        )
        .bind(&row.id)
        .bind(embedding_model)
        .fetch_one(pool)
        .await?;
        let schedule: Schedule = serde_json::from_str(&row.schedule).unwrap_or_default();
        let open = schedule.open_at(now);
        let next_open = (!open).then(|| schedule.next_open(now)).flatten().map(|t| t.format("%Y-%m-%dT%H:%M:00").to_string());
        let (contradictions, fulfilled, findings_to_review) = if row.kind == "politics" { claim_tallies(pool, &row.id).await? } else { Default::default() };
        statuses.push(InvestigationStatus {
            targets: targets.into_iter().map(|(name,)| name).collect(),
            sources: row.sources,
            failing: row.failing,
            documents: row.documents,
            unread: row.unread,
            last_update: row.last_update,
            indexing,
            queued: row.queued,
            schedule: ScheduleState { schedule, open, next_open, next_run_at: row.next_run_at },
            contradictions,
            fulfilled,
            insights: findings_to_review + row.suggested_sources + row.suggested_scandals,
            latest: if row.kind == "politics" { latest(pool, &row.id).await? } else { None },
            disk_bytes: disk_bytes(pool, &row.id).await?,
            id: row.id,
            name: row.name,
            kind: row.kind,
        });
    }
    Ok(statuses)
}

/// Contradicted and fulfilled claims (confirmed, or waiting for review), and findings to review.
async fn claim_tallies(pool: &SqlitePool, investigation_id: &str) -> Result<(Tally, Tally, i64)> {
    let claims = analyze::claims(pool, investigation_id).await?;
    let (mut contradictions, mut fulfilled, mut to_review) = (Tally::default(), Tally::default(), 0);
    for claim in &claims {
        let suggested = |label: &str| claim.findings.iter().any(|f| f.status == "suggested" && f.label == label);
        match analyze::claim_state(claim) {
            "contradicted" => contradictions.confirmed += 1,
            "fulfilled" => fulfilled.confirmed += 1,
            "review" => {
                contradictions.suggested += i64::from(suggested("CONTRADICTS"));
                fulfilled.suggested += i64::from(suggested("FULFILLS"));
            }
            _ => {}
        }
        to_review += claim.findings.iter().filter(|f| f.status == "suggested").count() as i64;
    }
    Ok((contradictions, fulfilled, to_review))
}

/// The latest finding, fact-check or scandal a person confirmed; if none, the latest waiting for review.
async fn latest(pool: &SqlitePool, investigation_id: &str) -> Result<Option<Highlight>> {
    for status in ["confirmed", "suggested"] {
        let found: Option<Highlight> = sqlx::query_as(
            "SELECT * FROM (
                SELECT CASE r.label WHEN 'CONTRADICTS' THEN 'contradiction' ELSE 'fulfillment' END AS kind, e.label AS text, t.name AS person,
                       r.created_at AS date, r.status, e.document_id, NULL AS session_id
                FROM relations r JOIN entities e ON e.id = r.from_id
                LEFT JOIN targets t ON t.id = json_extract(e.attrs, '$.target_id')
                WHERE r.investigation_id = ?1 AND r.label IN ('CONTRADICTS', 'FULFILLS') AND r.status = ?2
                  AND COALESCE(json_extract(r.props, '$.scope'), 'none') IN ('self', 'none')
                UNION ALL
                SELECT CASE c.verdict WHEN 'false' THEN 'check_false' ELSE 'check_true' END, c.claim, t.name, c.created_at, c.status, NULL, c.session_id
                FROM live_checks c JOIN targets t ON t.id = c.target_id
                WHERE t.investigation_id = ?1 AND c.verdict IN ('true', 'false') AND c.status = ?2
                UNION ALL
                SELECT 'scandal', s.label, p.label, r.created_at, r.status, s.document_id, NULL
                FROM relations r JOIN entities s ON s.id = r.to_id JOIN entities p ON p.id = r.from_id
                WHERE r.investigation_id = ?1 AND r.label = 'INVOLVED_IN' AND r.status = ?2
             ) ORDER BY date DESC LIMIT 1",
        )
        .bind(investigation_id)
        .bind(status)
        .fetch_optional(pool)
        .await?;
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

fn disk_cache() -> &'static Mutex<HashMap<String, (Instant, i64)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, i64)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The room an investigation takes: its texts, its search index (the full-text index counted as
/// its text once more), its graph, the pictures kept and its recordings. The official bulk files
/// are shared by every investigation and left out.
async fn disk_bytes(pool: &SqlitePool, investigation_id: &str) -> Result<i64> {
    if let Some((at, bytes)) = disk_cache().lock().ok().and_then(|c| c.get(investigation_id).copied()) {
        if at.elapsed() < DISK_TTL {
            return Ok(bytes);
        }
    }
    let (rows,): (i64,) = sqlx::query_as(
        "SELECT (SELECT COALESCE(SUM(octet_length(text) + COALESCE(octet_length(segments), 0)), 0) FROM documents WHERE investigation_id = ?1)
              + (SELECT COALESCE(SUM(2 * octet_length(text) + COALESCE(octet_length(embedding), 0)), 0) FROM rag_chunks WHERE investigation_id = ?1)
              + (SELECT COALESCE(SUM(octet_length(attrs) + octet_length(label)), 0) FROM entities WHERE investigation_id = ?1)
              + (SELECT COALESCE(SUM(octet_length(props)), 0) FROM relations WHERE investigation_id = ?1)",
    )
    .bind(investigation_id)
    .fetch_one(pool)
    .await?;
    let files: Vec<(String,)> = sqlx::query_as(
        "SELECT image_path FROM documents WHERE investigation_id = ?1 AND image_path IS NOT NULL
         UNION ALL SELECT audio_path FROM live_sessions WHERE investigation_id = ?1 AND audio_path IS NOT NULL",
    )
    .bind(investigation_id)
    .fetch_all(pool)
    .await?;
    let on_disk: u64 = files.iter().filter_map(|(path,)| std::fs::metadata(path).ok()).map(|m| m.len()).sum();
    let bytes = rows + on_disk as i64;
    if let Ok(mut cache) = disk_cache().lock() {
        cache.insert(investigation_id.to_string(), (Instant::now(), bytes));
    }
    Ok(bytes)
}

/// One person followed, by what was confirmed about them (Home's scoreboard). Only what a person
/// confirmed counts; what waits for review is counted apart.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonScore {
    pub target_id: String,
    pub name: String,
    pub investigation_id: String,
    pub investigation_name: String,
    /// Promises and statements confirmed contradicted by their own acts.
    pub contradictions: i64,
    /// Live statements confirmed false.
    pub false_checks: i64,
    pub fulfilled: i64,
    /// Live statements confirmed true.
    pub true_checks: i64,
    /// Scandals they were confirmed involved in.
    pub scandals: i64,
    /// Sanctioned companies they are tied to, from public records.
    pub sanctioned: i64,
    pub to_review: i64,
}

pub async fn scores(pool: &SqlitePool) -> Result<Vec<PersonScore>> {
    let people: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT t.id, t.name, i.id, i.name FROM targets t JOIN investigations i ON i.id = t.investigation_id
         WHERE i.kind = 'politics' AND t.kind = 'person' ORDER BY i.created_at, t.created_at",
    )
    .fetch_all(pool)
    .await?;
    let mut scores: Vec<PersonScore> = people
        .into_iter()
        .map(|(target_id, name, investigation_id, investigation_name)| PersonScore { target_id, name, investigation_id, investigation_name, ..Default::default() })
        .collect();
    let mut investigations: Vec<String> = scores.iter().map(|s| s.investigation_id.clone()).collect();
    investigations.dedup();
    let index: HashMap<String, usize> = scores.iter().enumerate().map(|(i, s)| (s.target_id.clone(), i)).collect();
    for investigation_id in &investigations {
        for claim in analyze::claims(pool, investigation_id).await? {
            let Some(&i) = index.get(&claim.target_id) else { continue };
            match analyze::claim_state(&claim) {
                "contradicted" => scores[i].contradictions += 1,
                "fulfilled" => scores[i].fulfilled += 1,
                _ => {}
            }
            scores[i].to_review += claim.findings.iter().filter(|f| f.status == "suggested").count() as i64;
        }
        for tie in crate::scandals::sanctioned(pool, investigation_id).await? {
            if let Some(&i) = index.get(&tie.target_id) {
                scores[i].sanctioned += 1;
            }
        }
    }
    let checks: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT target_id, verdict, status, COUNT(*) FROM live_checks WHERE target_id IS NOT NULL AND verdict IN ('true', 'false', 'partly') GROUP BY 1, 2, 3",
    )
    .fetch_all(pool)
    .await?;
    for (target_id, verdict, status, count) in checks {
        let Some(&i) = index.get(&target_id) else { continue };
        match (status.as_str(), verdict.as_str()) {
            ("confirmed", "false") => scores[i].false_checks += count,
            ("confirmed", "true") => scores[i].true_checks += count,
            ("suggested", _) => scores[i].to_review += count,
            _ => {}
        }
    }
    let scandals: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT t.id, r.status, COUNT(*) FROM relations r JOIN entities p ON p.id = r.from_id JOIN targets t ON p.key = 'target:' || t.id
         WHERE r.label = 'INVOLVED_IN' AND r.status IN ('confirmed', 'suggested') GROUP BY 1, 2",
    )
    .fetch_all(pool)
    .await?;
    for (target_id, status, count) in scandals {
        let Some(&i) = index.get(&target_id) else { continue };
        if status == "confirmed" {
            scores[i].scandals += count;
        } else {
            scores[i].to_review += count;
        }
    }
    scores.sort_by(|a, b| (b.contradictions + b.false_checks).cmp(&(a.contradictions + a.false_checks)).then_with(|| a.name.cmp(&b.name)));
    Ok(scores)
}

#[tauri::command]
pub async fn scoreboard(state: tauri::State<'_, AppState>) -> Result<Vec<PersonScore>, String> {
    scores(state.db_manager.pool()).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn dashboard(state: tauri::State<'_, AppState>) -> Result<Vec<InvestigationStatus>, String> {
    let pool = state.db_manager.pool();
    let model = match crate::rag::indexer::embedder(pool).await {
        Ok(embedder) => embedder.model_id().to_string(),
        Err(_) => String::new(),
    };
    statuses(pool, &model).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn run(pool: &SqlitePool, sql: &str) {
        sqlx::query(sql).execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn an_investigation_at_a_glance() {
        let pool = pool().await;
        run(&pool, "INSERT INTO investigations (id, name, kind, created_at) VALUES ('inv', 'Prefeita', 'politics', '2026-01-01')").await;
        run(&pool, "INSERT INTO targets (id, investigation_id, name, created_at) VALUES ('t1', 'inv', 'Helena Prado', '2026-01-01')").await;
        run(&pool, "INSERT INTO sources (id, investigation_id, kind, url, next_run_at, created_at, last_error) VALUES ('s1', 'inv', 'rss', 'https://a.example/feed', '2026-01-01T00:00:00Z', '2026', 'HTTP 500')").await;
        run(&pool, "INSERT INTO sources (id, investigation_id, kind, url, next_run_at, created_at, status) VALUES ('s2', 'inv', 'rss', 'https://b.example/feed', '2026-01-01T00:00:00Z', '2026', 'suggested')").await;
        run(&pool, "INSERT INTO documents (id, investigation_id, url, kind, title, text, fetched_at, content_hash) VALUES ('d1', 'inv', 'https://a.example/1', 'article', 'Obra', 'texto', '2026-03-01', 'h')").await;
        run(&pool, "INSERT INTO crawl_queue (investigation_id, url, next_attempt_at, created_at) VALUES ('inv', 'https://a.example/2', '2026', '2026')").await;
        for (id, label) in [("c1", "Vai construir 10 creches"), ("c2", "Não vai aumentar o IPTU")] {
            sqlx::query("INSERT INTO entities (id, investigation_id, type, key, label, attrs, created_at) VALUES (?, 'inv', 'promise', ?, ?, '{\"target_id\":\"t1\"}', '2026')")
                .bind(id)
                .bind(format!("claim:{id}"))
                .bind(label)
                .execute(&pool)
                .await
                .unwrap();
        }
        run(&pool, "INSERT INTO entities (id, investigation_id, type, key, label, created_at) VALUES ('v', 'inv', 'vote_session', 'camara:votacao:1', 'Votação', '2026')").await;
        // A promise kept (confirmed, older) and a contradiction waiting for review (newer)
        run(&pool, "INSERT INTO relations (id, investigation_id, from_id, to_id, label, status, props, created_at) VALUES ('r1', 'inv', 'c1', 'v', 'FULFILLS', 'confirmed', '{\"scope\":\"self\"}', '2026-02-01')").await;
        run(&pool, "INSERT INTO relations (id, investigation_id, from_id, to_id, label, status, props, created_at) VALUES ('r2', 'inv', 'c2', 'v', 'CONTRADICTS', 'suggested', '{\"scope\":\"self\"}', '2026-03-01')").await;

        let status = statuses(&pool, "").await.unwrap().pop().unwrap();
        assert_eq!(status.targets, vec!["Helena Prado"]);
        assert_eq!((status.sources, status.failing, status.documents, status.unread, status.queued), (1, 1, 1, 1, 1));
        assert_eq!(status.indexing, 1, "not indexed yet");
        assert_eq!(status.fulfilled, Tally { confirmed: 1, suggested: 0 });
        assert_eq!(status.contradictions, Tally { confirmed: 0, suggested: 1 });
        assert_eq!(status.insights, 2, "the contradiction to review and the suggested source");
        let latest = status.latest.unwrap();
        assert_eq!((latest.kind.as_str(), latest.status.as_str(), latest.person.as_deref()), ("fulfillment", "confirmed", Some("Helena Prado")), "what a person confirmed comes before a newer suggestion");
        assert!(status.disk_bytes > 0);
        assert!(status.schedule.open && status.schedule.next_open.is_none());

        // Once confirmed, the contradiction is the latest
        run(&pool, "UPDATE relations SET status = 'confirmed' WHERE id = 'r2'").await;
        let status = statuses(&pool, "").await.unwrap().pop().unwrap();
        assert_eq!(status.latest.unwrap().kind, "contradiction");
        assert_eq!(status.contradictions, Tally { confirmed: 1, suggested: 0 });
    }

    #[tokio::test]
    async fn the_scoreboard_counts_only_what_a_person_confirmed() {
        let pool = pool().await;
        for sql in [
            "INSERT INTO investigations (id, name, kind, created_at) VALUES ('inv', 'Prefeita', 'politics', '2026-01-01')",
            "INSERT INTO targets (id, investigation_id, name, created_at) VALUES ('t1', 'inv', 'Helena Prado', '2026-01-01')",
            "INSERT INTO documents (id, investigation_id, url, kind, title, text, fetched_at, content_hash) VALUES ('d1', 'inv', 'https://a.example/1', 'article', 'Operação', 'texto', '2026-03-01', 'h')",
            "INSERT INTO entities (id, investigation_id, type, key, label, created_at) VALUES ('v', 'inv', 'vote_session', 'camara:votacao:1', 'Votação', '2026')",
            "INSERT INTO live_sessions (id, investigation_id, title, started_at) VALUES ('s1', 'inv', 'Sabatina', '2026')",
            "INSERT INTO live_checks (id, session_id, target_id, start_time, end_time, quote, claim, verdict, status, created_at) VALUES ('k1', 's1', 't1', 0, 1, 'q', 'Disse que a dívida caiu', 'false', 'confirmed', '2026')",
            "INSERT INTO live_checks (id, session_id, target_id, start_time, end_time, quote, claim, verdict, status, created_at) VALUES ('k2', 's1', 't1', 2, 3, 'q', 'Disse que fez 10 creches', 'true', 'suggested', '2026')",
        ] {
            run(&pool, sql).await;
        }
        for (claim, status) in [("c1", "confirmed"), ("c2", "suggested")] {
            sqlx::query("INSERT INTO entities (id, investigation_id, type, key, label, attrs, created_at) VALUES (?, 'inv', 'promise', ?, 'Promessa', '{\"target_id\":\"t1\"}', '2026')")
                .bind(claim)
                .bind(format!("claim:{claim}"))
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO relations (id, investigation_id, from_id, to_id, label, status, props, created_at) VALUES (?, 'inv', ?, 'v', 'CONTRADICTS', ?, '{\"scope\":\"self\"}', '2026')")
                .bind(format!("r-{claim}"))
                .bind(claim)
                .bind(status)
                .execute(&pool)
                .await
                .unwrap();
        }
        crate::scandals::mark(&pool, "d1", "Merenda", "2026-02-01", &["t1".to_string()]).await.unwrap();

        let score = scores(&pool).await.unwrap().pop().unwrap();
        assert_eq!(score.name, "Helena Prado");
        assert_eq!((score.contradictions, score.false_checks, score.fulfilled, score.true_checks, score.scandals), (1, 1, 0, 0, 1));
        assert_eq!(score.to_review, 2, "the suggested contradiction and the suggested check");
        let latest = statuses(&pool, "").await.unwrap().pop().unwrap().latest.unwrap();
        assert_eq!(latest.status, "confirmed");
    }
}
