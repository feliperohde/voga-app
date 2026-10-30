//! Ask: answers questions about an investigation's collected documents, with citations.
//!
//! 1. Planner: one LLM call turns the question into search terms plus an optional date
//!    range and document kind, resolving "last year" against today.
//! 2. Retrieval: hybrid search with those filters; if they leave nothing, retry without
//!    them (a planner mistake must not hide evidence).
//! 3. Evidence gate: no passages means "not found", without asking the LLM to guess.
//! 4. Answer: the LLM answers only from numbered excerpts and cites them as [n].

use anyhow::Result;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::embeddings::EmbeddingProvider;
use super::retriever::{hybrid_search, SearchResult};
use super::store::SearchFilters;
use crate::llm::ChatModel;

const PASSAGES_FOR_ANSWER: usize = 10;
const HISTORY_TURNS: usize = 4;
const HISTORY_ANSWER_CHARS: usize = 600;
/// Document kinds the planner may filter on.
const KINDS: [&str; 14] = [
    "article", "page", "video", "speech", "bill", "vote", "expense", "profile", "candidacy", "proposal", "amendment", "gazette", "contract", "procurement",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    /// The [n] number used in the answer.
    pub index: usize,
    pub chunk_id: String,
    pub document_id: String,
    pub title: String,
    pub url: String,
    pub site: Option<String>,
    pub kind: String,
    pub doc_date: String,
    /// Seconds into a video.
    pub start_time: Option<f64>,
    pub excerpt: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueryPlan {
    pub search_query: String,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub answer: String,
    /// False when no relevant passage was found (the answer is then empty).
    pub found: bool,
    pub citations: Vec<Citation>,
    pub plan: QueryPlan,
    /// True when the planner's filters returned nothing and the search was retried without them.
    pub filters_relaxed: bool,
    /// Set when semantic search was unavailable (keyword results only).
    pub notice: Option<String>,
    /// Id in the Ask history, once saved.
    pub history_id: Option<String>,
}

fn format_history(history: &[ConversationTurn]) -> String {
    let start = history.len().saturating_sub(HISTORY_TURNS);
    history[start..]
        .iter()
        .map(|turn| {
            // Old [n] markers point at previous excerpts; drop them so they can't be
            // confused with the numbering of the current ones.
            let answer: String = strip_citations(&turn.answer).chars().take(HISTORY_ANSWER_CHARS).collect();
            format!("Q: {}\nA: {}", turn.question, answer)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) const PLANNER_SYSTEM: &str = "You convert a question about collected news, pages and videos (on public figures, or on any subject an investigation follows) into a search plan. \
Reply with ONLY a JSON object, no prose: \
{\"search_query\": string, \"date_from\": \"YYYY-MM-DD\" or null, \"date_to\": \"YYYY-MM-DD\" or null, \"kind\": string or null}. \
search_query: the key terms to search for (keep names, bill numbers and places verbatim; resolve pronouns using the conversation). \
date_from is inclusive and date_to is exclusive; set them only when the question refers to a time (e.g. \"in 2022\", \"last month\", \"during the campaign\" when its dates are known). \
kind: set only when the question asks for one kind of record: \"article\" (news), \"video\", \"speech\" (speeches in Congress), \"vote\" (how they voted), \"bill\" (bills they authored), \"expense\" (parliamentary quota spending), \"candidacy\" (elections, declared assets), \"proposal\" (campaign government plan), \"amendment\" (parliamentary amendments: where the money went, who received it), \"gazette\" (municipal official gazettes), \"contract\" (contracts a city hall signed, by supplier), \"procurement\" (a city hall's public tenders), \"profile\".";

fn planner_prompt(question: &str, today: NaiveDate, context: &str, history: &[ConversationTurn]) -> String {
    let history = format_history(history);
    format!(
        "Today is {today}.\n\n<investigation>\n{context}\n</investigation>\n\n{history_block}Question: {question}",
        history_block = if history.is_empty() { String::new() } else { format!("Conversation so far:\n{history}\n\n") },
    )
}

/// Parses the planner's reply, discarding anything invalid. Never fails: the fallback
/// is the raw question with no filters.
pub fn parse_plan(reply: &str, question: &str) -> QueryPlan {
    #[derive(Deserialize)]
    struct RawPlan {
        search_query: Option<String>,
        date_from: Option<String>,
        date_to: Option<String>,
        kind: Option<String>,
    }
    let fallback = QueryPlan { search_query: question.to_string(), ..Default::default() };
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
        return fallback;
    };
    if end <= start {
        return fallback;
    }
    let Ok(raw) = serde_json::from_str::<RawPlan>(&reply[start..=end]) else {
        return fallback;
    };
    let valid_date = |d: Option<String>| d.map(|d| d.trim().to_string()).filter(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok());
    let (mut date_from, mut date_to) = (valid_date(raw.date_from), valid_date(raw.date_to));
    if let (Some(from), Some(to)) = (&date_from, &date_to) {
        if from >= to {
            // An inverted or empty range is unusable rather than a guess
            date_from = None;
            date_to = None;
        }
    }
    QueryPlan {
        search_query: raw.search_query.map(|q| q.trim().to_string()).filter(|q| !q.is_empty()).unwrap_or_else(|| question.to_string()),
        date_from,
        date_to,
        kind: raw.kind.map(|k| k.trim().to_lowercase()).filter(|k| KINDS.contains(&k.as_str())),
    }
}

const ANSWER_SYSTEM: &str = "You answer questions about what an investigation collected (on public figures, or on any subject it follows) using ONLY the numbered excerpts provided: official records first, then news, web pages and video subtitles. Rules:\n\
- Cite every factual claim with the excerpt number in square brackets, e.g. [2] or [1][3].\n\
- Tell apart what a person said themselves (a speech, an interview, their own post) from what others report about them, and say which it is.\n\
- Official records (votes, bills, speeches in Congress, expenses, amendments, contracts, TSE data) are the primary source: facts about what happened. News, videos and posts are secondary: what others report. When they disagree, follow the record and say that the secondary source differs.\n\
- Give the date of each statement when it matters (promises, positions that changed over time).\n\
- Subtitles of videos may have transcription errors: quote them with care.\n\
- If the excerpts do not contain the answer, say clearly that it was not found in the collected sources. Never invent facts, and never state as fact an accusation the excerpts only report.\n\
- Answer in the same language as the question. Be concise; markdown is allowed.";

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "video" => "Video subtitles (secondary source)",
        "page" => "Web page (secondary source)",
        "post" => "Social media post (secondary source)",
        "speech" => "Speech in Congress (official record)",
        "bill" => "Bill (official record)",
        "vote" => "Nominal vote (official record)",
        "expense" => "Parliamentary quota expenses (official record)",
        "profile" => "Official profile",
        "candidacy" => "Candidacy at the electoral court (TSE)",
        "proposal" => "Government proposal filed at the TSE",
        "amendment" => "Parliamentary amendment and who received it (official record)",
        "gazette" => "Municipal official gazette",
        "contract" => "Public contracts of a city hall (official record, PNCP)",
        "procurement" => "Public tenders of a city hall (official record, PNCP)",
        _ => "News article (secondary source)",
    }
}

fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn answer_prompt(question: &str, today: NaiveDate, context: &str, evidence: &[SearchResult], history: &[ConversationTurn]) -> String {
    let excerpts = evidence
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let hit = &r.hit;
            let date = hit.doc_date.get(..10).unwrap_or(&hit.doc_date);
            let time = hit.start_time.map(|t| format!(" · {}", format_time(t))).unwrap_or_default();
            let site = hit.site.as_deref().map(|s| format!(" · {s}")).unwrap_or_default();
            format!("[{}] \"{}\"{site} · {date}{time} · {}\n{}", i + 1, hit.title, kind_label(&hit.kind), hit.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let history = format_history(history);
    let mut prompt = format!("Today is {today}.\n\n<investigation>\n{context}\n</investigation>\n\n");
    if !history.is_empty() {
        prompt.push_str(&format!("<conversation>\n{history}\n</conversation>\n\n"));
    }
    prompt.push_str(&format!("<excerpts>\n{excerpts}\n</excerpts>\n\nQuestion: {question}"));
    prompt
}

/// Returns the excerpt numbers cited as [n] (also [1, 2]), in first-seen order.
pub fn cited_indices(answer: &str, max: usize) -> Vec<usize> {
    let mut cited = Vec::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        for part in after[..close].split(',') {
            if let Ok(n) = part.trim().parse::<usize>() {
                if (1..=max).contains(&n) && !cited.contains(&n) {
                    cited.push(n);
                }
            }
        }
        rest = &after[close + 1..];
    }
    cited
}

/// Removes citation markers like [2] or [1, 3], leaving other brackets intact.
pub fn strip_citations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find(']') {
            Some(close) if !after[..close].trim().is_empty() && after[..close].split(',').all(|p| p.trim().parse::<usize>().is_ok()) => {
                rest = &after[close + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace(" .", ".").trim().to_string()
}

pub fn excerpt(text: &str) -> String {
    const MAX: usize = 400;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(MAX).collect::<String>())
    }
}

/// The investigations asked about as the prompts see them: each one's name, description and who it follows.
pub async fn investigation_context(pool: &SqlitePool, scope: &[String]) -> Result<String> {
    let mut contexts = Vec::new();
    for investigation_id in scope {
        contexts.push(one_context(pool, investigation_id).await?);
    }
    Ok(contexts.join("\n\n"))
}

async fn one_context(pool: &SqlitePool, investigation_id: &str) -> Result<String> {
    let (name, description): (String, String) = sqlx::query_as("SELECT name, description FROM investigations WHERE id = ?")
        .bind(investigation_id)
        .fetch_one(pool)
        .await?;
    let targets: Vec<(String, String, String)> = sqlx::query_as("SELECT kind, name, aliases FROM targets WHERE investigation_id = ? ORDER BY created_at")
        .bind(investigation_id)
        .fetch_all(pool)
        .await?;
    let mut context = format!("Investigation: {name}");
    if !description.trim().is_empty() {
        context.push_str(&format!("\n{}", description.trim()));
    }
    for (kind, name, aliases) in targets {
        let aliases: Vec<String> = serde_json::from_str(&aliases).unwrap_or_default();
        let aka = if aliases.is_empty() { String::new() } else { format!(" (also: {})", aliases.join(", ")) };
        context.push_str(&format!("\nFollows {kind}: {name}{aka}"));
    }
    Ok(context)
}

pub async fn ask(
    pool: &SqlitePool,
    embedder: &dyn EmbeddingProvider,
    llm: &dyn ChatModel,
    scope: &[String],
    question: &str,
    history: &[ConversationTurn],
    today: NaiveDate,
) -> Result<Answer> {
    let question = question.trim();
    let context = investigation_context(pool, scope).await?;

    // 1. Plan
    let plan = match llm.complete(PLANNER_SYSTEM, &planner_prompt(question, today, &context, history)).await {
        Ok(reply) => parse_plan(&reply, question),
        Err(e) => {
            log::warn!("Ask: planner failed, searching with the raw question: {e}");
            QueryPlan { search_query: question.to_string(), ..Default::default() }
        }
    };
    log::info!("Ask: plan {plan:?}");

    // 2. Retrieve (relaxing filters if they leave nothing)
    let filters = SearchFilters { date_from: plan.date_from.clone(), date_to: plan.date_to.clone(), kind: plan.kind.clone() };
    let has_filters = filters.date_from.is_some() || filters.date_to.is_some() || filters.kind.is_some();
    let mut response = hybrid_search(pool, embedder, scope, &plan.search_query, &filters, PASSAGES_FOR_ANSWER).await?;
    let mut filters_relaxed = false;
    if response.results.is_empty() && has_filters {
        filters_relaxed = true;
        response = hybrid_search(pool, embedder, scope, &plan.search_query, &SearchFilters::default(), PASSAGES_FOR_ANSWER).await?;
    }
    let notice = response.vector_error.clone();

    // 3. Evidence gate
    if response.results.is_empty() {
        return Ok(Answer { answer: String::new(), found: false, citations: Vec::new(), plan, filters_relaxed, notice, history_id: None });
    }

    // 4. Answer
    // Official records first (the sort keeps relevance order within each group)
    let mut evidence = response.results;
    evidence.sort_by_key(|r| !crate::collect::official::is_record(&r.hit.kind));
    let answer = llm.complete(ANSWER_SYSTEM, &answer_prompt(question, today, &context, &evidence, history)).await?.trim().to_string();
    let cited = cited_indices(&answer, evidence.len());
    let citations = evidence
        .iter()
        .enumerate()
        .filter(|(i, _)| cited.is_empty() || cited.contains(&(i + 1)))
        .map(|(i, r)| Citation {
            index: i + 1,
            chunk_id: r.hit.chunk_id.clone(),
            document_id: r.hit.document_id.clone(),
            title: r.hit.title.clone(),
            url: r.hit.url.clone(),
            site: r.hit.site.clone(),
            kind: r.hit.kind.clone(),
            doc_date: r.hit.doc_date.clone(),
            start_time: r.hit.start_time,
            excerpt: excerpt(&r.hit.text),
        })
        .collect();
    Ok(Answer { answer, found: true, citations, plan, filters_relaxed, notice, history_id: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_keep_valid_filters_and_fall_back_on_garbage() {
        let plan = parse_plan(
            r#"Plan: {"search_query": "reforma tributária", "date_from": "2022-01-01", "date_to": "2023-01-01", "kind": "Video"}"#,
            "q",
        );
        assert_eq!(plan.search_query, "reforma tributária");
        assert_eq!(plan.date_from.as_deref(), Some("2022-01-01"));
        assert_eq!(plan.kind.as_deref(), Some("video"));

        let inverted = parse_plan(r#"{"search_query": "x", "date_from": "2023-01-01", "date_to": "2022-01-01", "kind": "tweet"}"#, "q");
        assert_eq!((inverted.date_from, inverted.date_to, inverted.kind), (None, None, None));

        assert_eq!(parse_plan("no json here", "what did she say?").search_query, "what did she say?");
    }

    #[test]
    fn citations_are_read_and_stripped() {
        assert_eq!(cited_indices("Ela prometeu [2] e depois votou contra [1, 3][9].", 3), vec![2, 1, 3]);
        assert_eq!(strip_citations("Ela prometeu [2]. Lei [anexo] [1, 3]."), "Ela prometeu. Lei [anexo].");
    }
}
