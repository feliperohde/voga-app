//! The AI suggests searches for what nothing proves yet: promises with no record that keeps or
//! breaks them; in an interest, for more of what the reader liked. Only searches (news, videos, the web): the AI writes good queries but invents
//! addresses. Suggestions wait for a person's approval, unless the investigation approves them
//! by itself; a dismissed one is kept so it is never suggested again.

use anyhow::{bail, Result};
use chrono::Utc;
use serde::Deserialize;
use sqlx::SqlitePool;

use crate::collect::{fold, schedule_time};
use crate::llm::ChatModel;
use crate::prefs;

const SYSTEM: &str = "You help an investigation of Brazilian public figures find evidence. Given who it follows, the promises still without evidence and the searches it already runs, propose up to 5 NEW searches likely to find records that keep or break those promises (news, videos or web pages): name the person and the specific bill, program or policy, in Portuguese, the way people search. Never repeat or rephrase a search it already runs. Reply with ONLY a JSON object: {\"suggestions\": [{\"kind\": \"news\" | \"video\" | \"web\", \"query\": \"...\", \"reason\": \"one short sentence in Portuguese saying what it looks for\"}]}.";

const INTEREST_SYSTEM: &str = "You help someone follow a subject in the news (any subject: health, science, technology, business, culture, a product). Given what they follow, the documents they liked and the searches already running, propose up to 5 NEW searches likely to find recent news, research, launches and developments on the subject or on what the liked documents are about (news, videos or web pages), in Portuguese, the way people search. Never repeat or rephrase a search already running. Reply with ONLY a JSON object: {\"suggestions\": [{\"kind\": \"news\" | \"video\" | \"web\", \"query\": \"...\", \"reason\": \"one short sentence in Portuguese saying what it looks for\"}]}.";

const MAX_SUGGESTIONS: usize = 5;
/// Promises shown to the AI.
const MAX_GAPS: i64 = 12;
/// By itself, the AI suggests at most once a day per investigation.
const EVERY_HOURS: i64 = 24;

fn last_run_key(investigation_id: &str) -> String {
    format!("discovery_suggested:{investigation_id}")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// news_search | youtube_search | web_search
    pub kind: String,
    pub query: String,
    pub reason: String,
}

/// The suggestions in the AI's reply; web searches become news searches without a web search service.
pub fn parse(reply: &str, web_search: bool) -> Vec<Suggestion> {
    #[derive(Deserialize)]
    struct Reply {
        #[serde(default)]
        suggestions: Vec<Raw>,
    }
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        kind: String,
        #[serde(default)]
        query: String,
        #[serde(default)]
        reason: String,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return Vec::new() };
    let Ok(parsed) = serde_json::from_str::<Reply>(reply.get(start..=end).unwrap_or_default()) else { return Vec::new() };
    let mut seen = std::collections::HashSet::new();
    parsed
        .suggestions
        .into_iter()
        .filter_map(|raw| {
            let kind = match raw.kind.trim().to_lowercase().as_str() {
                "video" | "youtube" => "youtube_search",
                "web" if web_search => "web_search",
                _ => "news_search",
            };
            let query = raw.query.trim().to_string();
            if query.is_empty() || query.chars().count() > 200 || !seen.insert((kind, fold(&query))) {
                return None;
            }
            Some(Suggestion { kind: kind.to_string(), query, reason: raw.reason.trim().to_string() })
        })
        .take(MAX_SUGGESTIONS)
        .collect()
}

async fn is_interest(pool: &SqlitePool, investigation_id: &str) -> Result<bool> {
    let (kind,): (String,) = sqlx::query_as("SELECT kind FROM investigations WHERE id = ?").bind(investigation_id).fetch_one(pool).await?;
    Ok(kind == "interest")
}

/// Promises with no record that keeps or breaks them (the ones a person rejected do not count);
/// in an interest, the titles of what the reader liked.
async fn gaps(pool: &SqlitePool, investigation_id: &str, interest: bool) -> Result<Vec<String>> {
    if interest {
        let rows: Vec<(String,)> = sqlx::query_as("SELECT title FROM documents WHERE investigation_id = ? AND feedback = 1 ORDER BY COALESCE(read_at, fetched_at) DESC LIMIT ?")
            .bind(investigation_id)
            .bind(MAX_GAPS)
            .fetch_all(pool)
            .await?;
        return Ok(rows.into_iter().map(|(title,)| title).collect());
    }
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT e.label FROM entities e
         WHERE e.investigation_id = ? AND e.type = 'promise'
           AND NOT EXISTS (SELECT 1 FROM relations r WHERE r.from_id = e.id AND r.label IN ('CONTRADICTS', 'FULFILLS') AND r.status != 'rejected')
         ORDER BY e.created_at DESC LIMIT ?",
    )
    .bind(investigation_id)
    .bind(MAX_GAPS)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(label,)| label).collect())
}

fn prompt(brief: &str, gaps: &[String], existing: &[(String, String)], interest: bool) -> String {
    let (heading, none) = if interest {
        ("Documents the reader liked", "(none yet: suggest searches about the subject's latest developments)")
    } else {
        ("Promises without evidence", "(none extracted yet: suggest searches about what the people followed promised, voted and did)")
    };
    let gaps = if gaps.is_empty() { none.to_string() } else { gaps.iter().map(|g| format!("- {g}")).collect::<Vec<_>>().join("\n") };
    let existing = existing.iter().map(|(kind, query)| format!("- {kind}: {query}")).collect::<Vec<_>>().join("\n");
    format!("{brief}\n\n{heading}:\n{gaps}\n\nSearches it already runs:\n{existing}")
}

/// Asks the AI for searches and adds them (waiting for approval unless the investigation
/// approves by itself). `asked`: a person asked now (also when there are no gaps). Returns how many.
pub async fn suggest(pool: &SqlitePool, llm: &dyn ChatModel, investigation_id: &str, asked: bool) -> Result<usize> {
    let interest = is_interest(pool, investigation_id).await?;
    let gaps = gaps(pool, investigation_id, interest).await?;
    if gaps.is_empty() && !asked {
        return Ok(0);
    }
    let (_, ai_left) = super::left(pool, investigation_id).await?;
    if ai_left == 0 {
        if asked {
            bail!("Today's AI limit for discovery is used up; raise it in the investigation's discovery settings");
        }
        return Ok(0);
    }
    let existing: Vec<(String, String)> = sqlx::query_as(
        "SELECT kind, url FROM sources WHERE investigation_id = ? AND kind IN ('news_search', 'youtube_search', 'web_search') ORDER BY created_at",
    )
    .bind(investigation_id)
    .fetch_all(pool)
    .await?;
    let brief = super::brief(pool, investigation_id).await?;
    let system = if interest { INTEREST_SYSTEM } else { SYSTEM };
    let reply = llm.complete(system, &prompt(&brief, &gaps, &existing, interest)).await;
    super::spend(pool, investigation_id, 0, 1).await?;
    prefs::set(pool, &last_run_key(investigation_id), &Utc::now().to_rfc3339()).await?;
    let suggestions = parse(&reply?, super::websearch::configured(pool).await);

    let status = if super::limits(pool, investigation_id).await?.auto_approve { "active" } else { "suggested" };
    let mut added = 0;
    for suggestion in suggestions {
        let every = if suggestion.kind == "news_search" { 360 } else { 1440 };
        let result = sqlx::query(
            "INSERT OR IGNORE INTO sources (id, investigation_id, kind, url, every_minutes, next_run_at, status, origin, note, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 'llm', ?, ?)",
        )
        .bind(format!("src-{}", uuid::Uuid::new_v4()))
        .bind(investigation_id)
        .bind(&suggestion.kind)
        .bind(&suggestion.query)
        .bind(every)
        .bind(schedule_time(Utc::now()))
        .bind(status)
        .bind(Some(suggestion.reason.as_str()).filter(|r| !r.is_empty()))
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await?;
        added += result.rows_affected() as usize;
    }
    if added > 0 {
        log::info!("Discover: the AI suggested {added} searches for {investigation_id}");
        if status == "active" {
            crate::collect::scheduler::wake();
        }
    }
    Ok(added)
}

/// From the scheduler when there is nothing else to do: one investigation whose promises lack
/// evidence and that had no suggestions in the last day.
pub async fn step(pool: &SqlitePool) -> Result<()> {
    let investigations: Vec<(String,)> = sqlx::query_as("SELECT id FROM investigations ORDER BY created_at").fetch_all(pool).await?;
    for (investigation_id,) in investigations {
        let last = prefs::get(pool, &last_run_key(&investigation_id)).await?;
        let recent = last
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
            .is_some_and(|at| Utc::now().signed_duration_since(at) < chrono::Duration::hours(EVERY_HOURS));
        if recent || gaps(pool, &investigation_id, is_interest(pool, &investigation_id).await?).await?.is_empty() {
            continue;
        }
        let Some(llm) = super::model(pool).await else { return Ok(()) };
        if let Err(e) = suggest(pool, &llm, &investigation_id, false).await {
            log::warn!("Discover: suggestions for {investigation_id} failed: {e}");
        }
        // One investigation per pass: the next one waits for the next idle moment
        break;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_are_read_safely() {
        let reply = r#"Aqui: {"suggestions": [
            {"kind": "news", "query": "Marina Exemplo creches zona norte", "reason": "Promessa das creches."},
            {"kind": "web", "query": "PL 1234/2023 tramitação", "reason": "Projeto prometido."},
            {"kind": "video", "query": "Marina Exemplo cesta básica discurso", "reason": ""},
            {"kind": "news", "query": "  marina exemplo CRECHES zona norte ", "reason": "Repetida."},
            {"kind": "news", "query": "", "reason": "Vazia."}
        ]}"#;
        let found = parse(reply, false);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0], Suggestion { kind: "news_search".into(), query: "Marina Exemplo creches zona norte".into(), reason: "Promessa das creches.".into() });
        // No web search service: the web search becomes a news search
        assert_eq!(found[1].kind, "news_search");
        assert_eq!(found[2].kind, "youtube_search");
        assert_eq!(parse(reply, true)[1].kind, "web_search");
        assert!(parse("sem ideias", true).is_empty());
    }

    struct Suggests;

    #[async_trait::async_trait]
    impl ChatModel for Suggests {
        async fn complete(&self, _system: &str, user: &str) -> Result<String> {
            assert!(user.contains("- Prometeu creches em todos os bairros"), "{user}");
            Ok(r#"{"suggestions": [{"kind": "news", "query": "Marina Exemplo creches", "reason": "Para a promessa das creches."},
                                    {"kind": "video", "query": "Marina Exemplo creches discurso", "reason": ""}]}"#
                .into())
        }
    }

    #[tokio::test]
    async fn suggestions_wait_for_approval_and_are_not_repeated() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Mandato', '2026')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', 'Marina Exemplo', '2026')").execute(&pool).await.unwrap();
        // Nothing to look for yet: by itself, the AI is not asked
        assert_eq!(suggest(&pool, &Suggests, "inv", false).await.unwrap(), 0);

        sqlx::query("INSERT INTO entities (id, investigation_id, type, key, label, created_at) VALUES ('c1', 'inv', 'promise', 'claim:1', 'Prometeu creches em todos os bairros', '2026')")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(suggest(&pool, &Suggests, "inv", false).await.unwrap(), 2);
        let rows: Vec<(String, String, String, String, Option<String>)> =
            sqlx::query_as("SELECT kind, url, status, origin, note FROM sources ORDER BY kind").fetch_all(&pool).await.unwrap();
        assert_eq!(rows[0], ("news_search".into(), "Marina Exemplo creches".into(), "suggested".into(), "llm".into(), Some("Para a promessa das creches.".into())));
        assert_eq!(rows[1].0, "youtube_search");
        assert_eq!(rows[1].4, None);

        // Dismissed ones stay dismissed; the same suggestions add nothing
        sqlx::query("UPDATE sources SET status = 'rejected' WHERE kind = 'news_search'").execute(&pool).await.unwrap();
        assert_eq!(suggest(&pool, &Suggests, "inv", true).await.unwrap(), 0);
        let (rejected,): (String,) = sqlx::query_as("SELECT status FROM sources WHERE kind = 'news_search'").fetch_one(&pool).await.unwrap();
        assert_eq!(rejected, "rejected");

        // With auto-approve, new ones start at once
        sqlx::query("DELETE FROM sources").execute(&pool).await.unwrap();
        sqlx::query("UPDATE investigations SET discovery = '{\"autoApprove\": true}'").execute(&pool).await.unwrap();
        assert_eq!(suggest(&pool, &Suggests, "inv", true).await.unwrap(), 2);
        let (active,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sources WHERE status = 'active'").fetch_one(&pool).await.unwrap();
        assert_eq!(active, 2);
        assert_eq!(super::super::usage(&pool, "inv").await.unwrap().ai_calls, 3);
    }

    #[test]
    fn the_prompt_lists_gaps_and_existing_searches() {
        let text = prompt("Investigation: X", &["Prometeu creches".into()], &[("news_search".into(), "\"Marina\"".into())], false);
        assert!(text.contains("- Prometeu creches"));
        assert!(text.contains("- news_search: \"Marina\""));
        assert!(prompt("I", &[], &[], false).contains("none extracted yet"));
        // An interest shows what the reader liked, not promises
        let text = prompt("I", &["Nova insulina semanal".into()], &[], true);
        assert!(text.contains("Documents the reader liked:\n- Nova insulina semanal") && !text.contains("Promises"));
    }
}
