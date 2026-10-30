//! Fact checks while a session goes on. The lines of each person followed are read in a window
//! of the last minute, every 20 s of new speech (Assunta's insights Window); the AI lists the
//! checkable claims in the new part (its quote must be in what was said, as in
//! `analyze::claims`), each claim is searched in the records (`analyze::evidence`: the acts
//! first), and the AI says whether the records confirm it, contradict it, confirm it in part or
//! do not settle it, citing them. When they do not settle it and a web search service is set
//! up, the web is searched, the first pages are collected into the investigation and the claim
//! is judged again. The records searched are those of the investigation following the person
//! who said it (in a debate, each person's own).
//!
//! A verdict needs records: without a record cited it is UNVERIFIED. And it is a suggestion:
//! only a person confirms or rejects it.

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::analyze::claims::{self, Person};
use crate::analyze::{self, actor::Actor, judge};
use crate::collect::{self, Hints};
use crate::discover::websearch;
use crate::llm::ChatModel;
use crate::rag::embeddings::EmbeddingProvider;
use crate::rag::indexer;
use crate::rag::retriever::SearchResult;

pub const EVENT: &str = "live-check";
/// What is read each time: the last minute of a person's lines…
const WINDOW_SECONDS: f64 = 60.0;
/// …once this much new speech came in.
const EVERY_SECONDS: f64 = 20.0;
const MIN_CONFIDENCE: f64 = 0.6;
/// Pages collected from one web search.
const WEB_PAGES: usize = 3;
/// Web searches per session (free plans of search services have small quotas).
const MAX_WEB_SEARCHES: usize = 30;
const EXCERPT_CHARS: usize = 600;

pub const EXTRACT_SYSTEM: &str = "You read a live transcript of a Brazilian politician speaking (a debate, an interview, a rally), written down automatically: punctuation may be missing and some words misheard. List the factual claims the politician makes in the <new> part that public records could confirm or refute: numbers and statistics, dates, votes and bills, laws, public spending, contracts and budget amendments, results of a government or a term, what someone did or said. The <before> part is only context.\n\
Skip opinions, promises and plans for the future, slogans, questions, jokes and attacks without a checkable fact. Skip what is in the list of claims already checked.\n\
Reply with ONLY a JSON object, no prose: {\"claims\": [{\"kind\": \"affirmation\" | \"denial\", \"claim\": string, \"quote\": string, \"topic\": string, \"query\": string}]}.\n\
- kind: denial when the politician denies a fact or an accusation (\"nunca votei a favor\", \"não é verdade que eu…\", \"isso é mentira\"); affirmation otherwise.\n\
- claim: one short, specific sentence IN BRAZILIAN PORTUGUESE (never English), in the third person about the politician (\"<name> afirma que votou…\", never \"<name> votei…\"), keeping the numbers, dates and names said.\n\
- quote: the words of the transcript that say it, copied exactly as written (at most 300 characters).\n\
- topic: 1 to 4 words in Portuguese naming the subject.\n\
- query: words to find records about it, in Portuguese (names, bill numbers, places, the subject), at most 12 words.\n\
- Return {\"claims\": []} when there are none.";

pub const JUDGE_SYSTEM: &str = "You fact-check a claim a Brazilian politician made live against numbered records (nominal votes, bills, speeches, contracts, budget amendments, expenses, news, web pages), each with its date. Decide what the records show:\n\
- TRUE: they confirm it: the same fact, act or number (rounding aside).\n\
- FALSE: they show it is wrong: the act did not happen or went the other way, or the number is clearly different.\n\
- PARTLY: the core is right, but a number, a date or the context is wrong, exaggerated or left out.\n\
- UNVERIFIED: they do not settle it: no record is about this exact fact (records on the same theme only are not enough).\n\
Use only the records, never what you know: the records are the evidence a person will read. Each record says whose act it is (\"Act of\"): another person's vote or bill says nothing about what the politician did, unless the claim is about that person. Read what a vote was about before using its Sim/Não: voting \"Não\" on a motion to reject a bill is supporting it. A news report or a web page counts when it states the fact itself.\n\
Reply with ONLY a JSON object: {\"verdict\": \"TRUE\" | \"FALSE\" | \"PARTLY\" | \"UNVERIFIED\", \"records\": [the numbers of the records the verdict rests on], \"confidence\": number from 0 to 1, \"rationale\": one or two sentences in Portuguese saying what the records show}.";

/// A line said by a person followed, in seconds from the session start.
#[derive(Debug, Clone)]
pub struct Line {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub target_id: String,
}

/// The last `seconds` of a person's lines; those not read yet are the new part. A line is
/// marked read itself, not by its time: a voice answered later brings earlier lines, unread.
pub struct Window {
    seconds: f64,
    every: f64,
    lines: VecDeque<(Line, bool)>,
}

impl Window {
    pub fn new(seconds: f64, every: f64) -> Self {
        Self { seconds, every, lines: VecDeque::new() }
    }

    /// Adds a line; true when `every` seconds of speech wait to be read.
    pub fn push(&mut self, line: Line) -> bool {
        if line.text.trim().is_empty() {
            return false;
        }
        let at = self.lines.iter().position(|(l, _)| l.start > line.start).unwrap_or(self.lines.len());
        self.lines.insert(at, (line, false));
        let latest = self.lines.iter().map(|(l, _)| l.end).fold(0.0, f64::max);
        while self.lines.front().is_some_and(|(l, read)| *read && l.end < latest - self.seconds) {
            self.lines.pop_front();
        }
        let unread: f64 = self.lines.iter().filter(|(_, read)| !read).map(|(l, _)| l.end - l.start).sum();
        unread >= self.every
    }

    pub fn has_new(&self) -> bool {
        self.lines.iter().any(|(_, read)| !read)
    }

    /// (lines already read, lines not read yet)
    fn parts(&self) -> (Vec<&Line>, Vec<&Line>) {
        let (read, new): (Vec<_>, Vec<_>) = self.lines.iter().partition(|(_, read)| *read);
        (read.into_iter().map(|(l, _)| l).collect(), new.into_iter().map(|(l, _)| l).collect())
    }

    fn mark_read(&mut self) {
        self.lines.iter_mut().for_each(|(_, read)| *read = true);
    }
}

fn joined(lines: &[&Line]) -> String {
    lines.iter().map(|l| l.text.trim()).collect::<Vec<_>>().join(" ")
}

/// A claim the AI found, with the words it rests on as they are in the transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    /// affirmation | denial
    pub kind: &'static str,
    pub claim: String,
    pub quote: String,
    pub topic: String,
    pub query: String,
}

pub fn extract_prompt(person: &Person, before: &str, new: &str, checked: &[String]) -> String {
    let name = if person.aliases.is_empty() { person.name.clone() } else { format!("{} (also: {})", person.name, person.aliases.join(", ")) };
    let checked = if checked.is_empty() { "(none)".to_string() } else { checked.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n") };
    format!(
        "Politician: {name}\n\nClaims already checked:\n{checked}\n\n<before>\n{before}\n</before>\n\n<new>\n{new}\n</new>\n\nWrite \"claim\", \"topic\" and \"query\" in Brazilian Portuguese."
    )
}

/// The claims of the reply whose quote is really in `transcript` (an invented one never shows).
pub fn parse_claims(reply: &str, transcript: &str) -> Vec<Claim> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        claims: Vec<RawClaim>,
    }
    #[derive(Deserialize)]
    struct RawClaim {
        #[serde(default)]
        kind: String,
        #[serde(default)]
        claim: String,
        #[serde(default)]
        quote: String,
        #[serde(default)]
        topic: String,
        #[serde(default)]
        query: String,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return Vec::new() };
    let Ok(raw) = serde_json::from_str::<Raw>(reply.get(start..=end).unwrap_or_default()) else { return Vec::new() };
    raw.claims
        .into_iter()
        .filter_map(|c| {
            let claim = c.claim.trim().to_string();
            if claim.is_empty() {
                return None;
            }
            let Some(quote) = claims::locate_quote(transcript, &c.quote) else {
                log::info!("Live check: dropped a claim whose quote is not in the transcript: {:?}", c.quote);
                return None;
            };
            let topic = c.topic.trim().to_lowercase();
            let query = if c.query.trim().is_empty() { format!("{topic} {claim}") } else { c.query.trim().to_string() };
            let kind = if c.kind.trim().eq_ignore_ascii_case("denial") { "denial" } else { "affirmation" };
            Some(Claim { kind, claim, quote, topic, query })
        })
        .collect()
}

/// The words of a text that say what it is about.
fn content_words(text: &str) -> HashSet<String> {
    claims::normalize(text).split_whitespace().filter(|w| w.chars().count() >= 4).map(str::to_string).collect()
}

/// The same claim again: its quote overlaps an earlier one's, or most of its words are an earlier claim's.
fn repeats(claim: &Claim, checked: &[Claim]) -> bool {
    let quote = claims::normalize(&claim.quote);
    let words = content_words(&claim.claim);
    checked.iter().any(|earlier| {
        let earlier_quote = claims::normalize(&earlier.quote);
        let earlier_words = content_words(&earlier.claim);
        let shared = words.intersection(&earlier_words).count();
        earlier_quote.contains(&quote)
            || quote.contains(&earlier_quote)
            || (shared >= 3 && shared as f64 >= 0.7 * words.len().min(earlier_words.len()) as f64)
    })
}

/// When a claim was said: the line its quote is in (or two lines it spans), else the new part.
fn moment(lines: &[&Line], quote: &str) -> (f64, f64) {
    if let Some(line) = lines.iter().find(|l| claims::locate_quote(&l.text, quote).is_some()) {
        return (line.start, line.end);
    }
    for pair in lines.windows(2) {
        if claims::locate_quote(&format!("{} {}", pair[0].text, pair[1].text), quote).is_some() {
            return (pair[0].start, pair[1].end);
        }
    }
    (lines.first().map_or(0.0, |l| l.start), lines.last().map_or(0.0, |l| l.end))
}

/// What the AI concluded about a claim, from the records shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// true | false | partly | unverified
    pub verdict: &'static str,
    /// Indexes of the records it rests on.
    pub records: Vec<usize>,
    pub confidence: f64,
    pub rationale: String,
}

impl Verdict {
    fn unverified(rationale: &str) -> Self {
        Self { verdict: "unverified", records: Vec::new(), confidence: 0.0, rationale: rationale.to_string() }
    }
}

pub fn judge_prompt(person: &str, date: &str, claim: &Claim, records: &[judge::Record]) -> String {
    format!(
        "Politician: {person}\nSaid live on {date}: {}\nTheir words: \"{}\"\n\n<records>\n{}\n</records>",
        claim.claim,
        claim.quote,
        judge::numbered(records)
    )
}

#[derive(Deserialize, Default)]
struct RawVerdict {
    #[serde(default)]
    verdict: String,
    #[serde(default)]
    records: Vec<serde_json::Value>,
    confidence: Option<f64>,
    #[serde(default)]
    rationale: String,
}

/// The fields read one by one, for JSON a small model broke (seen with Qwen 3.5 4B: the quote
/// that opens the rationale dropped, and quotes left unescaped inside it). The rationale is the
/// last field: it runs to the closing brace.
fn lenient(json: &str) -> RawVerdict {
    let after = |key: &str| {
        let marker = format!("\"{key}\"");
        json.find(&marker).map(|at| json[at + marker.len()..].trim_start().trim_start_matches(':').trim_start())
    };
    RawVerdict {
        verdict: after("verdict").map(|v| v.trim_start_matches('"').chars().take_while(char::is_ascii_alphabetic).collect()).unwrap_or_default(),
        records: after("records")
            .and_then(|v| v.strip_prefix('['))
            .and_then(|v| v.split(']').next())
            .map(|list| list.split(',').filter_map(|n| n.trim().trim_matches('"').parse::<u64>().ok()).map(serde_json::Value::from).collect())
            .unwrap_or_default(),
        confidence: after("confidence").and_then(|v| v.split(|c: char| !(c.is_ascii_digit() || c == '.')).next()).and_then(|n| n.parse().ok()),
        rationale: after("rationale").map(|v| v.rfind('}').map_or(v, |end| &v[..end]).trim().trim_matches('"').replace("\\\"", "\"")).unwrap_or_default(),
    }
}

/// The verdict of the reply. Confirming, refuting or correcting needs records cited and some
/// confidence: otherwise the claim is not settled.
pub fn parse_verdict(reply: &str, records: usize) -> Verdict {
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return Verdict::unverified("") };
    let json = reply.get(start..=end).unwrap_or_default();
    let raw = serde_json::from_str::<RawVerdict>(json).unwrap_or_else(|_| lenient(json));
    let mut cited: Vec<usize> = Vec::new();
    for number in raw.records.iter().filter_map(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))) {
        let number = number as usize;
        if number >= 1 && number <= records && !cited.contains(&(number - 1)) {
            cited.push(number - 1);
        }
    }
    let rationale = raw.rationale.trim().to_string();
    let confidence = raw.confidence.unwrap_or(0.0).clamp(0.0, 1.0);
    let verdict = match raw.verdict.trim().to_uppercase().as_str() {
        "TRUE" => "true",
        "FALSE" => "false",
        "PARTLY" => "partly",
        _ => "unverified",
    };
    if verdict == "unverified" || cited.is_empty() || confidence < MIN_CONFIDENCE || rationale.is_empty() {
        return Verdict::unverified(if verdict == "unverified" { &rationale } else { "" });
    }
    Verdict { verdict, records: cited, confidence, rationale }
}

/// A record a verdict rests on, as the UI shows it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub document_id: String,
    pub chunk_id: String,
    pub title: String,
    pub kind: String,
    pub date: String,
    pub url: String,
    pub site: Option<String>,
    pub excerpt: String,
    pub start_time: Option<f64>,
    /// Whose act it is; None for news, pages and videos.
    pub actor: Option<Actor>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckView {
    pub id: String,
    pub session_id: String,
    pub target_id: Option<String>,
    pub start: f64,
    pub end: f64,
    pub quote: String,
    pub claim: String,
    pub topic: String,
    /// affirmation | denial
    pub kind: String,
    /// detected (not checked yet) | checking | true | false | partly | unverified
    pub verdict: String,
    pub confidence: Option<f64>,
    pub rationale: Option<String>,
    pub evidence: Vec<Evidence>,
    pub web_searched: bool,
    /// suggested | confirmed | rejected
    pub status: String,
    pub error: Option<String>,
}

#[derive(sqlx::FromRow)]
struct CheckRow {
    id: String,
    session_id: String,
    target_id: Option<String>,
    start_time: f64,
    end_time: f64,
    quote: String,
    claim: String,
    topic: String,
    kind: String,
    verdict: String,
    confidence: Option<f64>,
    rationale: Option<String>,
    evidence: String,
    web_searched: bool,
    status: String,
    error: Option<String>,
}

impl From<CheckRow> for CheckView {
    fn from(r: CheckRow) -> Self {
        Self {
            id: r.id,
            session_id: r.session_id,
            target_id: r.target_id,
            start: r.start_time,
            end: r.end_time,
            quote: r.quote,
            claim: r.claim,
            topic: r.topic,
            kind: r.kind,
            verdict: r.verdict,
            confidence: r.confidence,
            rationale: r.rationale,
            evidence: serde_json::from_str(&r.evidence).unwrap_or_default(),
            web_searched: r.web_searched,
            status: r.status,
            error: r.error,
        }
    }
}

const COLUMNS: &str =
    "id, session_id, target_id, start_time, end_time, quote, claim, topic, kind, verdict, confidence, rationale, evidence, web_searched, status, error";

pub async fn list(pool: &SqlitePool, session_id: &str) -> Result<Vec<CheckView>> {
    let rows: Vec<CheckRow> = sqlx::query_as(&format!("SELECT {COLUMNS} FROM live_checks WHERE session_id = ? ORDER BY start_time, created_at"))
        .bind(session_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(CheckView::from).collect())
}

/// One check, as the UI shows it.
pub async fn load(pool: &SqlitePool, id: &str) -> Result<CheckView> {
    let row: CheckRow = sqlx::query_as(&format!("SELECT {COLUMNS} FROM live_checks WHERE id = ?")).bind(id).fetch_one(pool).await?;
    Ok(row.into())
}

/// The person's review of a check.
pub async fn set_status(pool: &SqlitePool, id: &str, status: &str) -> Result<()> {
    if !matches!(status, "suggested" | "confirmed" | "rejected") {
        return Err(anyhow!("Unknown status: {status}"));
    }
    sqlx::query("UPDATE live_checks SET status = ? WHERE id = ?").bind(status).bind(id).execute(pool).await?;
    Ok(())
}

/// Reads windows, checks claims, keeps them and tells the UI.
pub struct Checker {
    pub pool: SqlitePool,
    pub llm: Arc<dyn ChatModel>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub session_id: String,
    /// The people followed (claims are checked for them).
    pub people: Vec<Person>,
    /// When the session happens (YYYY-MM-DD), for the judge.
    pub date: String,
    pub on_check: Box<dyn Fn(&CheckView) + Send + Sync>,
    /// Check each claim found right away; off, claims are only detected until a person asks.
    pub auto: Arc<AtomicBool>,
    windows: HashMap<String, Window>,
    checked: Vec<Claim>,
    web_searches: usize,
}

impl Checker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: SqlitePool,
        llm: Arc<dyn ChatModel>,
        embedder: Arc<dyn EmbeddingProvider>,
        session_id: String,
        people: Vec<Person>,
        date: String,
        on_check: Box<dyn Fn(&CheckView) + Send + Sync>,
    ) -> Self {
        Self {
            pool,
            llm,
            embedder,
            session_id,
            people,
            date,
            on_check,
            auto: Arc::new(AtomicBool::new(true)),
            windows: HashMap::new(),
            checked: Vec::new(),
            web_searches: 0,
        }
    }

    /// A line of a person followed: read their window once enough new speech came in.
    pub async fn heard(&mut self, line: Line) {
        let target = line.target_id.clone();
        let window = self.windows.entry(target.clone()).or_insert_with(|| Window::new(WINDOW_SECONDS, EVERY_SECONDS));
        if window.push(line) {
            self.read(&target).await;
        }
    }

    /// The session ended: what was said since the last read is read now.
    pub async fn flush(&mut self) {
        let waiting: Vec<String> = self.windows.iter().filter(|(_, w)| w.has_new()).map(|(t, _)| t.clone()).collect();
        for target in waiting {
            self.read(&target).await;
        }
    }

    async fn read(&mut self, target_id: &str) {
        // Someone followed after the session started
        if !self.people.iter().any(|p| p.target_id == target_id) {
            if let Ok(people) = super::session::people(&self.pool, &self.session_id).await {
                self.people = people;
            }
        }
        let Some(person) = self.people.iter().find(|p| p.target_id == target_id).cloned() else { return };
        let Some(window) = self.windows.get_mut(target_id) else { return };
        let (before, new) = window.parts();
        let (before_text, new_text) = (joined(&before), joined(&new));
        let new_lines: Vec<Line> = new.into_iter().cloned().collect();
        let all_text = format!("{before_text} {new_text}");
        window.mark_read();
        let checked: Vec<String> = self.checked.iter().rev().take(10).map(|c| c.claim.clone()).collect();
        let reply = match self.llm.complete(EXTRACT_SYSTEM, &extract_prompt(&person, &before_text, &new_text, &checked)).await {
            Ok(reply) => reply,
            Err(e) => return log::warn!("Live check: reading the transcript failed: {e:#}"),
        };
        for claim in parse_claims(&reply, &all_text) {
            if repeats(&claim, &self.checked) {
                continue;
            }
            self.checked.push(claim.clone());
            let lines: Vec<&Line> = new_lines.iter().collect();
            let (start, end) = moment(&lines, &claim.quote);
            if let Err(e) = self.check(&person, &claim, start, end).await {
                log::warn!("Live check: checking {:?} failed: {e:#}", claim.claim);
            }
        }
    }

    async fn check(&mut self, person: &Person, claim: &Claim, start: f64, end: f64) -> Result<()> {
        let id = format!("check-{}", uuid::Uuid::new_v4());
        let auto = self.auto.load(Ordering::SeqCst);
        sqlx::query(
            "INSERT INTO live_checks (id, session_id, target_id, start_time, end_time, quote, claim, topic, kind, query, verdict, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&self.session_id)
        .bind(&person.target_id)
        .bind(start)
        .bind(end)
        .bind(&claim.quote)
        .bind(&claim.claim)
        .bind(&claim.topic)
        .bind(claim.kind)
        .bind(&claim.query)
        .bind(if auto { "checking" } else { "detected" })
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        (self.on_check)(&load(&self.pool, &id).await?);
        if auto {
            self.settle_and_keep(&id, person, claim).await?;
        }
        Ok(())
    }

    /// Checks a claim kept as `id` and keeps the verdict.
    async fn settle_and_keep(&mut self, id: &str, person: &Person, claim: &Claim) -> Result<()> {
        let result = self.settle(person, claim).await;
        let (verdict, evidence, web_searched, error) = match result {
            Ok((verdict, evidence, web)) => (verdict, evidence, web, None),
            Err(e) => (Verdict::unverified(""), Vec::new(), false, Some(e.to_string())),
        };
        sqlx::query("UPDATE live_checks SET verdict = ?, confidence = ?, rationale = ?, evidence = ?, web_searched = ?, error = ? WHERE id = ?")
            .bind(verdict.verdict)
            .bind((verdict.verdict != "unverified").then_some(verdict.confidence))
            .bind(Some(verdict.rationale).filter(|r| !r.is_empty()))
            .bind(serde_json::to_string(&evidence)?)
            .bind(web_searched)
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        (self.on_check)(&load(&self.pool, id).await?);
        Ok(())
    }

    /// The verdict, the records it rests on, and whether the web was searched.
    async fn settle(&mut self, person: &Person, claim: &Claim) -> Result<(Verdict, Vec<Evidence>, bool)> {
        let (verdict, evidence) = self.judge(person, claim).await?;
        if verdict.verdict != "unverified" || self.web_searches >= MAX_WEB_SEARCHES || !websearch::configured(&self.pool).await {
            return Ok((verdict, evidence, false));
        }
        self.web_searches += 1;
        let found = self.collect_web(&home(&self.pool, &person.target_id).await?, &claim.query).await;
        if found == 0 {
            return Ok((verdict, evidence, true));
        }
        let (verdict, evidence) = self.judge(person, claim).await?;
        Ok((verdict, evidence, true))
    }

    async fn judge(&self, person: &Person, claim: &Claim) -> Result<(Verdict, Vec<Evidence>)> {
        let investigation_id = home(&self.pool, &person.target_id).await?;
        let records = analyze::evidence(&self.pool, self.embedder.as_ref(), &investigation_id, &claim.query, None).await?;
        if records.is_empty() {
            return Ok((Verdict::unverified(""), Vec::new()));
        }
        let actors = analyze::actors(&self.pool, &investigation_id, &person.target_id, &records).await?;
        let shown: Vec<judge::Record> = records
            .iter()
            .zip(&actors)
            .map(|(r, a)| judge::Record {
                actor: a.as_ref().map(|a| analyze::act_of(a, &person.name)),
                kind: &r.hit.kind,
                title: &r.hit.title,
                date: &r.hit.doc_date,
                text: &r.hit.text,
            })
            .collect();
        let reply = self.llm.complete(JUDGE_SYSTEM, &judge_prompt(&person.name, &self.date, claim, &shown)).await?;
        log::debug!("Live check: {} records for {:?}, the judge said {reply}", records.len(), claim.claim);
        let verdict = parse_verdict(&reply, records.len());
        let evidence = verdict.records.iter().map(|&i| evidence_of(&records[i], actors[i].clone())).collect();
        Ok((verdict, evidence))
    }

    /// Searches the web and collects the first pages not stored yet into the investigation; returns how many.
    async fn collect_web(&self, investigation_id: &str, query: &str) -> usize {
        let entries = match websearch::search(&self.pool, query).await {
            Ok(entries) => entries,
            Err(e) => {
                log::warn!("Live check: web search failed: {e:#}");
                return 0;
            }
        };
        let mut collected = 0;
        for entry in entries {
            if collected >= WEB_PAGES {
                break;
            }
            let stored: Option<(String,)> = sqlx::query_as("SELECT id FROM documents WHERE investigation_id = ? AND url = ?")
                .bind(investigation_id)
                .bind(collect::canonical_url(&entry.url))
                .fetch_optional(&self.pool)
                .await
                .ok()
                .flatten();
            if stored.is_some() {
                continue;
            }
            let hints = Hints { title: entry.title.as_deref(), published: entry.published.as_deref(), must_mention: &[] };
            match collect::ingest_page(&self.pool, investigation_id, None, &entry.url, "article", hints).await {
                Ok(page) => {
                    if let Some(id) = page.saved {
                        if let Err(e) = indexer::index_document(&self.pool, self.embedder.as_ref(), &id).await {
                            log::warn!("Live check: indexing {} failed: {e:#}", entry.url);
                        }
                        collected += 1;
                    }
                }
                Err(e) => log::info!("Live check: {} not collected: {e:#}", entry.url),
            }
        }
        collected
    }
}

/// The investigation following a person: where what they say is checked.
async fn home(pool: &SqlitePool, target_id: &str) -> Result<String> {
    Ok(sqlx::query_scalar("SELECT investigation_id FROM targets WHERE id = ?").bind(target_id).fetch_one(pool).await?)
}

/// Checks one claim now (detected, or checked before), during or after its session.
pub async fn check_now(pool: &SqlitePool, id: &str, llm: Arc<dyn ChatModel>, embedder: Arc<dyn EmbeddingProvider>, on_check: Box<dyn Fn(&CheckView) + Send + Sync>) -> Result<()> {
    PENDING.fetch_add(1, Ordering::SeqCst);
    let result = check_claim(pool, id, llm, embedder, on_check).await;
    PENDING.fetch_sub(1, Ordering::SeqCst);
    result
}

async fn check_claim(pool: &SqlitePool, id: &str, llm: Arc<dyn ChatModel>, embedder: Arc<dyn EmbeddingProvider>, on_check: Box<dyn Fn(&CheckView) + Send + Sync>) -> Result<()> {
    let (session_id, target_id, quote, text, topic, kind, query, started_at): (String, Option<String>, String, String, String, String, String, String) =
        sqlx::query_as(
            "SELECT c.session_id, c.target_id, c.quote, c.claim, c.topic, c.kind, c.query, s.started_at
             FROM live_checks c JOIN live_sessions s ON s.id = c.session_id WHERE c.id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await?;
    let people = super::session::people(pool, &session_id).await?;
    let person = people.iter().find(|p| Some(&p.target_id) == target_id.as_ref()).cloned().ok_or_else(|| anyhow!("The person who said it is no longer followed"))?;
    let (web_searches,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM live_checks WHERE session_id = ? AND web_searched = 1").bind(&session_id).fetch_one(pool).await?;
    let query = if query.is_empty() { format!("{topic} {text}") } else { query };
    let claim = Claim { kind: if kind == "denial" { "denial" } else { "affirmation" }, claim: text, quote, topic, query };
    sqlx::query("UPDATE live_checks SET verdict = 'checking', error = NULL WHERE id = ?").bind(id).execute(pool).await?;
    on_check(&load(pool, id).await?);
    let mut checker = Checker::new(pool.clone(), llm, embedder, session_id, people, started_at.get(..10).unwrap_or(&started_at).to_string(), on_check);
    checker.web_searches = web_searches as usize;
    checker.settle_and_keep(id, &person, &claim).await
}

fn evidence_of(record: &SearchResult, actor: Option<Actor>) -> Evidence {
    let hit = &record.hit;
    Evidence {
        document_id: hit.document_id.clone(),
        chunk_id: hit.chunk_id.clone(),
        title: hit.title.clone(),
        kind: hit.kind.clone(),
        date: hit.doc_date.get(..10).unwrap_or(&hit.doc_date).to_string(),
        url: hit.url.clone(),
        site: hit.site.clone(),
        excerpt: hit.text.chars().take(EXCERPT_CHARS).collect(),
        start_time: hit.start_time,
        actor,
    }
}

// ---- The checker of the session in progress ----

enum Message {
    Line(Line),
    Flush,
}

static SENDER: Mutex<Option<mpsc::UnboundedSender<Message>>> = Mutex::new(None);
/// Lines heard and not handled yet: while there are any, the AI is busy with the session.
static PENDING: AtomicUsize = AtomicUsize::new(0);

/// Lines still to read and claims still to check.
pub fn pending() -> usize {
    PENDING.load(Ordering::SeqCst)
}

/// Starts checking a session's lines with `checker` (None: no AI model, lines are only counted off).
pub fn start(mut checker: Option<Checker>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    if let Ok(mut sender) = SENDER.lock() {
        *sender = Some(tx);
    }
    tauri::async_runtime::spawn(async move {
        while let Some(message) = rx.recv().await {
            if let Some(checker) = checker.as_mut() {
                match message {
                    Message::Line(line) => checker.heard(line).await,
                    Message::Flush => checker.flush().await,
                }
            }
            PENDING.fetch_sub(1, Ordering::SeqCst);
        }
        log::info!("Live check: done");
    });
}

/// A line of a person followed, to check.
pub fn heard(line: Line) {
    send(Message::Line(line));
}

/// No more lines: the last ones are read, then the checker ends.
pub fn finish() {
    send(Message::Flush);
    if let Ok(mut sender) = SENDER.lock() {
        *sender = None;
    }
}

fn send(message: Message) {
    let Ok(sender) = SENDER.lock() else { return };
    if let Some(sender) = sender.as_ref() {
        PENDING.fetch_add(1, Ordering::SeqCst);
        if sender.send(message).is_err() {
            PENDING.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{save_document, NewDocument};
    use async_trait::async_trait;

    fn line(start: f64, end: f64, text: &str) -> Line {
        Line { start, end, text: text.into(), target_id: "t1".into() }
    }

    fn marina() -> Person {
        Person { target_id: "t1".into(), name: "Marina Exemplo".into(), aliases: vec!["Marina do Bairro".into()] }
    }

    #[test]
    fn the_window_keeps_a_minute_and_reads_every_twenty_seconds_of_new_speech() {
        let mut w = Window::new(WINDOW_SECONDS, EVERY_SECONDS);
        assert!(!w.push(line(0.0, 5.0, "boa noite")));
        assert!(w.push(line(5.0, 21.0, "eu votei contra a reforma")));
        w.mark_read();
        assert!(!w.push(line(21.0, 30.0, "e construí creches")));
        let (before, new) = w.parts();
        assert_eq!(joined(&before), "boa noite eu votei contra a reforma");
        assert_eq!(joined(&new), "e construí creches");
        assert!(w.push(line(30.0, 95.0, "fim")));
        w.mark_read();
        // What ended more than a minute before the latest line, and was read, goes
        w.push(line(95.0, 100.0, "obrigada"));
        assert_eq!(joined(&w.parts().0), "fim");
        // A line of a voice answered later goes back in its place, unread
        let mut w = Window::new(WINDOW_SECONDS, EVERY_SECONDS);
        w.push(line(10.0, 12.0, "depois"));
        w.push(line(2.0, 4.0, "antes"));
        assert_eq!(joined(&w.parts().1), "antes depois");
    }

    #[test]
    fn claims_need_their_words_in_the_transcript() {
        let transcript = "boa noite eu votei contra a reforma da previdência em 2019 e no meu governo construímos 120 creches";
        let reply = r#"Aqui: {"claims": [
            {"claim": "Marina Exemplo afirma ter votado contra a reforma da previdência em 2019.", "quote": "eu votei contra a reforma da Previdência em 2019", "topic": "Reforma da previdência", "query": "votação reforma da previdência 2019"},
            {"claim": "Marina Exemplo diz que construiu 120 creches.", "quote": "construímos 120 creches", "topic": "creches", "query": ""},
            {"claim": "Inventada.", "quote": "fechamos todas as escolas da cidade", "topic": "x", "query": "x"},
            {"claim": "", "quote": "boa noite eu votei", "topic": "x", "query": "x"}
        ]}"#;
        let found = parse_claims(reply, transcript);
        assert_eq!(found.len(), 2, "{found:?}");
        // No kind given: an affirmation
        assert_eq!(found[0].kind, "affirmation");
        let denial = r#"{"claims": [{"kind": "denial", "claim": "Marina Exemplo nega ter votado contra a reforma.", "quote": "eu votei contra a reforma da previdência", "topic": "x", "query": "x"}]}"#;
        assert_eq!(parse_claims(denial, transcript)[0].kind, "denial");
        assert_eq!(found[0].quote, "eu votei contra a reforma da previdência em 2019");
        assert_eq!(found[0].topic, "reforma da previdência");
        // No query: the topic and the claim are searched
        assert_eq!(found[1].query, "creches Marina Exemplo diz que construiu 120 creches.");
        assert!(parse_claims("nada", transcript).is_empty());
    }

    #[test]
    fn a_claim_said_again_is_not_checked_twice() {
        let claim = |claim: &str, quote: &str| Claim { kind: "affirmation", claim: claim.into(), quote: quote.into(), topic: String::new(), query: String::new() };
        let checked = vec![claim("Marina Exemplo afirma ter votado contra a reforma da previdência.", "eu votei contra a reforma da previdência")];
        // The same words seen again in the next window
        assert!(repeats(&claim("Outra frase.", "votei contra a reforma"), &checked));
        // The same claim in other words
        assert!(repeats(&claim("Marina Exemplo diz que votou contra a reforma da previdência.", "como eu disse fui contra essa reforma"), &checked));
        assert!(!repeats(&claim("Marina Exemplo diz que construiu 120 creches.", "construímos 120 creches"), &checked));
    }

    #[test]
    fn a_claim_is_placed_at_the_lines_its_words_are_in() {
        let lines = [line(10.0, 14.0, "boa noite a todos"), line(14.0, 19.0, "eu votei contra a reforma"), line(19.0, 25.0, "da previdência em 2019")];
        let refs: Vec<&Line> = lines.iter().collect();
        assert_eq!(moment(&refs, "eu votei contra a reforma"), (14.0, 19.0));
        assert_eq!(moment(&refs, "votei contra a reforma da previdência em 2019"), (14.0, 25.0));
        assert_eq!(moment(&refs, "nada disso aparece aqui"), (10.0, 25.0));
    }

    #[test]
    fn a_verdict_needs_records_cited_and_confidence() {
        let reply = |verdict: &str, records: &str, confidence: f64| {
            format!(r#"```json {{"verdict": "{verdict}", "records": {records}, "confidence": {confidence}, "rationale": "Votou Sim em 2019."}} ```"#)
        };
        let false_claim = parse_verdict(&reply("FALSE", "[2, 9, 2]", 0.9), 3);
        assert_eq!(false_claim, Verdict { verdict: "false", records: vec![1], confidence: 0.9, rationale: "Votou Sim em 2019.".into() });
        assert_eq!(parse_verdict(&reply("TRUE", r#"["1"]"#, 0.8), 3).records, vec![0]);
        // No record cited, records that do not exist, or little confidence: not settled
        assert_eq!(parse_verdict(&reply("TRUE", "[]", 0.9), 3).verdict, "unverified");
        assert_eq!(parse_verdict(&reply("FALSE", "[7]", 0.9), 3).verdict, "unverified");
        assert_eq!(parse_verdict(&reply("PARTLY", "[1]", 0.3), 3).verdict, "unverified");
        assert!(parse_verdict(&reply("FALSE", "[7]", 0.9), 3).records.is_empty());
        // UNVERIFIED keeps its reason; garbage settles nothing
        assert_eq!(parse_verdict(&reply("UNVERIFIED", "[]", 0.9), 3).rationale, "Votou Sim em 2019.");
        assert_eq!(parse_verdict("sem json", 3).verdict, "unverified");
        // The rationale's opening quote dropped, and quotes inside it (seen with Qwen 3.5 4B)
        let slip = r#"{"verdict": "TRUE", "records": [2], "confidence": 1.0, "rationale": O PL 1234/2023, "Creche no Bairro", é de autoria dela."}"#;
        assert_eq!(parse_verdict(slip, 3), Verdict { verdict: "true", records: vec![1], confidence: 1.0, rationale: r#"O PL 1234/2023, "Creche no Bairro", é de autoria dela."#.into() });
    }

    /// One dimension per keyword.
    struct Keywords;

    #[async_trait]
    impl EmbeddingProvider for Keywords {
        fn model_id(&self) -> &str {
            "kw"
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            const VOCAB: &[&str] = &["previdencia", "previdência", "creche", "votou"];
            Ok(texts
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    let mut v: Vec<f32> = VOCAB.iter().map(|w| t.matches(w).count() as f32).collect();
                    v.push(if v.iter().all(|x| *x == 0.0) { 1.0 } else { 0.0 });
                    v
                })
                .collect())
        }
    }

    /// Answers the extraction and judge prompts with scripted replies.
    struct Scripted {
        replies: std::sync::Mutex<Vec<String>>,
        prompts: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ChatModel for Scripted {
        async fn complete(&self, _system: &str, user: &str) -> Result<String> {
            self.prompts.lock().unwrap().push(user.to_string());
            Ok(self.replies.lock().unwrap().remove(0))
        }
    }

    /// A session of Marina Exemplo, and her vote for the 2019 pension reform, indexed.
    async fn pool_with_vote() -> (SqlitePool, String) {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for sql in [
            "INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Teste', '2026')",
            "INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', 'Marina Exemplo', '2026')",
            "INSERT INTO live_sessions (id, investigation_id, target_id, title, started_at) VALUES ('s1', 'inv', 't1', 'Debate', '2026-10-22T20:00:00Z')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let vote = save_document(
            &pool,
            NewDocument {
                investigation_id: "inv",
                source_id: None,
                url: "https://camara.leg.br/v1",
                kind: "vote",
                title: "Votação da reforma da previdência",
                author: None,
                site: Some("Câmara dos Deputados"),
                published_at: Some("2019-07-10"),
                lang: None,
                text: "Marina Exemplo votou \"Sim\" na votação: PEC 6/2019, reforma da previdência.",
                segments: None,
                note: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
        indexer::index_document(&pool, &Keywords, &vote).await.unwrap();
        (pool, vote)
    }

    #[tokio::test]
    async fn a_claim_and_the_vote_that_refutes_it_become_a_suggested_fake() {
        let (pool, vote) = pool_with_vote().await;

        let llm = Arc::new(Scripted {
            replies: std::sync::Mutex::new(vec![
                r#"{"claims": [{"claim": "Marina Exemplo afirma ter votado contra a reforma da previdência em 2019.", "quote": "eu votei contra a reforma da previdência", "topic": "reforma da previdência", "query": "votação reforma da previdência"}]}"#.into(),
                r#"{"verdict": "FALSE", "records": [1], "confidence": 0.9, "rationale": "Ela votou Sim na PEC 6/2019."}"#.into(),
            ]),
            prompts: std::sync::Mutex::new(Vec::new()),
        });
        let seen = Arc::new(std::sync::Mutex::new(Vec::<CheckView>::new()));
        let seen_by_ui = seen.clone();
        let mut checker = Checker::new(
            pool.clone(),
            llm.clone(),
            Arc::new(Keywords),
            "s1".into(),
            vec![marina()],
            "2026-10-22".into(),
            Box::new(move |check| seen_by_ui.lock().unwrap().push(check.clone())),
        );
        checker.heard(line(3.0, 9.0, "boa noite eu votei contra a reforma da previdência")).await;
        assert!(seen.lock().unwrap().is_empty(), "not 20 s of speech yet");
        checker.flush().await;

        // The UI saw it being checked, then the verdict
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].verdict, "checking");
        let check = &seen[1];
        assert_eq!((check.verdict.as_str(), check.status.as_str()), ("false", "suggested"));
        assert_eq!((check.start, check.end), (3.0, 9.0));
        assert_eq!(check.evidence.len(), 1);
        assert_eq!(check.evidence[0].document_id, vote);
        assert_eq!(check.evidence[0].date, "2019-07-10");
        assert_eq!(check.rationale.as_deref(), Some("Ela votou Sim na PEC 6/2019."));
        // The judge read the vote, its date, and the day the claim was said
        let prompts = llm.prompts.lock().unwrap();
        assert!(prompts[1].contains("vote · 2019-07-10") && prompts[1].contains("Said live on 2026-10-22"), "{}", prompts[1]);
        assert_eq!(list(&pool, "s1").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn without_auto_check_a_claim_is_detected_and_checked_on_demand() {
        let (pool, vote) = pool_with_vote().await;
        let extraction = r#"{"claims": [{"kind": "denial", "claim": "Marina Exemplo nega ter votado pela reforma da previdência.", "quote": "eu nunca votei pela reforma da previdência", "topic": "reforma da previdência", "query": "votação reforma da previdência"}]}"#;
        let llm = Arc::new(Scripted { replies: std::sync::Mutex::new(vec![extraction.into()]), prompts: std::sync::Mutex::new(Vec::new()) });
        let seen = Arc::new(std::sync::Mutex::new(Vec::<CheckView>::new()));
        let seen_by_ui = seen.clone();
        let mut checker = Checker::new(pool.clone(), llm.clone(), Arc::new(Keywords), "s1".into(), vec![marina()], "2026-10-22".into(),
            Box::new(move |check| seen_by_ui.lock().unwrap().push(check.clone())));
        checker.auto.store(false, Ordering::SeqCst);
        checker.heard(line(3.0, 9.0, "boa noite eu nunca votei pela reforma da previdência")).await;
        checker.flush().await;
        // Only found: the model read the transcript once, and judged nothing
        let detected = list(&pool, "s1").await.unwrap();
        assert_eq!(detected.len(), 1);
        assert_eq!((detected[0].verdict.as_str(), detected[0].kind.as_str()), ("detected", "denial"));
        assert_eq!(llm.prompts.lock().unwrap().len(), 1);

        // Checar: the records are searched with the query found, and the verdict kept
        let judge = Arc::new(Scripted {
            replies: std::sync::Mutex::new(vec![r#"{"verdict": "FALSE", "records": [1], "confidence": 0.9, "rationale": "Ela votou Sim na PEC 6/2019."}"#.into()]),
            prompts: std::sync::Mutex::new(Vec::new()),
        });
        let verdicts = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let heard = verdicts.clone();
        check_now(&pool, &detected[0].id, judge.clone(), Arc::new(Keywords), Box::new(move |c| heard.lock().unwrap().push(c.verdict.clone()))).await.unwrap();
        assert_eq!(*verdicts.lock().unwrap(), vec!["checking", "false"]);
        let checked = &list(&pool, "s1").await.unwrap()[0];
        assert_eq!((checked.verdict.as_str(), checked.evidence[0].document_id.as_str()), ("false", vote.as_str()));
        assert!(judge.prompts.lock().unwrap()[0].contains("nunca votei pela reforma"));
        assert_eq!(pending(), 0);
    }

    #[tokio::test]
    async fn in_a_debate_a_claim_is_checked_in_the_investigation_of_who_said_it() {
        let (pool, vote) = pool_with_vote().await;
        // A debate outside every investigation, Marina (followed in 'inv') among its people
        for sql in [
            "INSERT INTO live_sessions (id, title, started_at) VALUES ('d1', 'Debate', '2026-10-22T20:00:00Z')",
            "INSERT INTO live_participants (session_id, position, name, target_id) VALUES ('d1', 1, 'Marina Exemplo', 't1')",
            "INSERT INTO live_checks (id, session_id, target_id, start_time, end_time, quote, claim, topic, kind, query, verdict, created_at)
                VALUES ('c1', 'd1', 't1', 3, 9, 'eu votei contra a reforma', 'Marina Exemplo afirma ter votado contra a reforma da previdência.', 'previdência', 'affirmation', 'votação reforma da previdência', 'detected', 'x')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let judge = Arc::new(Scripted {
            replies: std::sync::Mutex::new(vec![r#"{"verdict": "FALSE", "records": [1], "confidence": 0.9, "rationale": "Ela votou Sim na PEC 6/2019."}"#.into()]),
            prompts: std::sync::Mutex::new(Vec::new()),
        });
        // check_claim, not check_now: the pending count is the other test's to watch
        check_claim(&pool, "c1", judge.clone(), Arc::new(Keywords), Box::new(|_| {})).await.unwrap();
        let checked = &list(&pool, "d1").await.unwrap()[0];
        assert_eq!((checked.verdict.as_str(), checked.evidence[0].document_id.as_str()), ("false", vote.as_str()));
        assert!(judge.prompts.lock().unwrap()[0].starts_with("Politician: Marina Exemplo"));
    }
}
