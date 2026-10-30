//! Analysis: the AI reads what was collected about the people followed, extracts their
//! promises and statements (with their words), and looks in the other records (votes, bills,
//! speeches, news) for what contradicts or fulfills them. Everything it finds is a
//! suggestion: it counts only after a person confirms it.

pub mod actor;
pub mod claims;
pub mod judge;

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::collect::fold;
use crate::graph;
use crate::llm::ChatModel;
use crate::rag::embeddings::EmbeddingProvider;
use crate::rag::retriever::{hybrid_search, SearchResult};
use crate::rag::store::SearchFilters;
use crate::state::AppState;
use claims::{Claim, Person};

/// Documents where people say things (`live`: a live session's transcript, its lines
/// prefixed with who said them). Votes, bills, expenses and TSE records are facts already:
/// they are what claims are checked against.
const ANALYZED_KINDS: &str = "'speech', 'video', 'article', 'proposal', 'page', 'post', 'live'";
/// Characters of a document read per request.
const BATCH_CHARS: usize = 6000;
/// A long document (a government plan) is read up to this many requests.
const MAX_BATCHES: usize = 30;
const MIN_CONFIDENCE: f64 = 0.6;

/// The people followed in an investigation (claims are only attributed to them).
pub async fn people(pool: &SqlitePool, investigation_id: &str) -> Result<Vec<Person>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT id, name, aliases FROM targets WHERE investigation_id = ? AND kind = 'person'")
        .bind(investigation_id)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(target_id, name, aliases)| Person { target_id, name, aliases: serde_json::from_str(&aliases).unwrap_or_default() })
        .collect())
}

#[derive(sqlx::FromRow)]
struct DocumentRow {
    id: String,
    investigation_id: String,
    title: String,
    kind: String,
    author: Option<String>,
    site: Option<String>,
    date: String,
}

#[derive(sqlx::FromRow)]
struct ChunkRow {
    id: String,
    text: String,
    start_time: Option<f64>,
}

/// Only political investigations have claims to analyze; interests are only collected.
const POLITICS: &str = "investigation_id IN (SELECT id FROM investigations WHERE kind = 'politics')";

/// Documents waiting to be analyzed, in every investigation.
pub async fn pending(pool: &SqlitePool) -> Result<i64> {
    let (count,): (i64,) = sqlx::query_as(&format!(
        "SELECT COUNT(*) FROM documents WHERE analyzed_at IS NULL AND indexed_at IS NOT NULL AND kind IN ({ANALYZED_KINDS}) AND {POLITICS}"
    ))
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Analyzes up to `limit` documents; `on_document` hears each title. Returns how many.
pub async fn run_batch(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, llm: &dyn ChatModel, limit: i64, on_document: impl Fn(&str)) -> Result<usize> {
    let docs: Vec<DocumentRow> = sqlx::query_as(&format!(
        "SELECT id, investigation_id, title, kind, author, site, COALESCE(published_at, fetched_at) AS date FROM documents
         WHERE analyzed_at IS NULL AND indexed_at IS NOT NULL AND kind IN ({ANALYZED_KINDS}) AND {POLITICS} ORDER BY fetched_at LIMIT ?"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    for doc in &docs {
        on_document(&doc.title);
        let result = analyze_document(pool, embedder, llm, doc).await;
        if let Err(e) = &result {
            log::warn!("Analyze: {} failed: {e}", doc.id);
        }
        sqlx::query("UPDATE documents SET analyzed_at = ?, analysis_error = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(result.err().map(|e| e.to_string()))
            .bind(&doc.id)
            .execute(pool)
            .await?;
    }
    Ok(docs.len())
}

fn claim_key(claim: &Claim) -> String {
    let digest = Sha256::digest(format!("{}|{}", claim.target_id, claims::normalize(&claim.text)).as_bytes());
    format!("claim:{:x}", digest)[..22].to_string()
}

async fn analyze_document(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, llm: &dyn ChatModel, doc: &DocumentRow) -> Result<usize> {
    let people = people(pool, &doc.investigation_id).await?;
    if people.is_empty() {
        return Ok(0);
    }
    // A speech or a government plan is its author's own words
    let author = matches!(doc.kind.as_str(), "speech" | "proposal")
        .then(|| doc.author.as_deref().and_then(|a| people.iter().find(|p| p.is(a))))
        .flatten();
    let chunks: Vec<ChunkRow> = sqlx::query_as("SELECT id, text, start_time FROM rag_chunks WHERE document_id = ? ORDER BY chunk_index")
        .bind(&doc.id)
        .fetch_all(pool)
        .await?;
    let header = format!(
        "\"{}\" ({}{}, {})",
        doc.title,
        doc.kind,
        doc.site.as_deref().map(|s| format!(", {s}")).unwrap_or_default(),
        doc.date.get(..10).unwrap_or(&doc.date)
    );

    let mut found = 0;
    for batch in batches(&chunks).into_iter().take(MAX_BATCHES) {
        let excerpt = batch.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n\n");
        let reply = llm.complete(claims::SYSTEM, &claims::prompt(&people, &header, &excerpt)).await?;
        for claim in claims::parse(&reply, &people, &excerpt, author) {
            // The passage the words are in, for the citation (and the moment of a video)
            let chunk = batch.iter().find(|c| claims::locate_quote(&c.text, &claim.quote).is_some()).unwrap_or(&batch[0]);
            let person = people.iter().find(|p| p.target_id == claim.target_id).expect("parse keeps known people");
            record_claim(pool, embedder, llm, doc, person, &claim, chunk).await?;
            found += 1;
        }
    }
    // News that reports a case against someone followed: one more request, on the passage that says so
    if crate::scandals::NEWS_KINDS.contains(&doc.kind.as_str()) {
        let batches = batches(&chunks);
        let texts = batches.iter().map(|batch| batch.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n\n"));
        if let Some(excerpt) = texts.into_iter().find(|text| crate::scandals::worth_asking(&people, text)) {
            if let Err(e) = crate::scandals::suggest(pool, llm, &doc.investigation_id, &doc.id, &people, &header, &excerpt).await {
                log::warn!("Analyze: scandals in {} failed: {e}", doc.id);
            }
        }
    }
    Ok(found)
}

/// Consecutive chunks of at most BATCH_CHARS (a longer chunk is a batch of its own).
fn batches(chunks: &[ChunkRow]) -> Vec<Vec<&ChunkRow>> {
    let mut batches: Vec<Vec<&ChunkRow>> = Vec::new();
    let mut size = 0;
    for chunk in chunks {
        if batches.is_empty() || size + chunk.text.len() > BATCH_CHARS {
            batches.push(Vec::new());
            size = 0;
        }
        size += chunk.text.len();
        batches.last_mut().expect("pushed above").push(chunk);
    }
    batches
}

async fn record_claim(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, llm: &dyn ChatModel, doc: &DocumentRow, person: &Person, claim: &Claim, chunk: &ChunkRow) -> Result<()> {
    let inv = &doc.investigation_id;
    let date = doc.date.get(..10).unwrap_or(&doc.date).to_string();
    let person_id = graph::entity(pool, inv, "person", &format!("target:{}", person.target_id), &person.name, json!({}), None).await?;
    let claim_id = graph::entity(
        pool,
        inv,
        &claim.kind,
        &claim_key(claim),
        &claim.text,
        json!({"quote": claim.quote, "topic": claim.topic, "stated_at": date, "target_id": person.target_id, "chunk_id": chunk.id, "start_time": chunk.start_time}),
        Some(&doc.id),
    )
    .await?;
    graph::relate(pool, inv, &person_id, &claim_id, "MADE", &date, json!({}), Some(&doc.id)).await?;

    let records = evidence(pool, embedder, inv, &format!("{} {}", claim.topic, claim.text), Some(&doc.id)).await?;
    if records.is_empty() {
        return Ok(());
    }
    let actors = actors(pool, inv, &person.target_id, &records).await?;
    let shown: Vec<judge::Record> = records
        .iter()
        .zip(&actors)
        .map(|(r, a)| judge::Record {
            actor: a.as_ref().map(|a| act_of(a, &person.name)),
            kind: &r.hit.kind,
            title: &r.hit.title,
            date: &r.hit.doc_date,
            text: &r.hit.text,
        })
        .collect();
    let reply = llm.complete(judge::SYSTEM, &judge::prompt(&person.name, &claim.kind, &claim.text, &claim.quote, &date, &shown)).await?;
    for judgement in judge::parse(&reply, records.len(), claim.kind == "promise", MIN_CONFIDENCE) {
        let hit = &records[judgement.record].hit;
        let acted_by = &actors[judgement.record];
        let evidence = evidence_entity(pool, inv, &hit.document_id, &hit.kind, &hit.title).await?;
        let excerpt: String = hit.text.chars().take(500).collect();
        graph::suggest(
            pool,
            inv,
            &claim_id,
            &evidence,
            &judgement.relation,
            hit.doc_date.get(..10).unwrap_or(&hit.doc_date),
            json!({"chunk_id": hit.chunk_id, "excerpt": excerpt, "start_time": hit.start_time, "actor": acted_by, "scope": acted_by.as_ref().map_or("none", |a| a.scope.as_str())}),
            Some(&hit.document_id),
            &judgement.rationale,
            judgement.confidence,
        )
        .await?;
    }
    Ok(())
}

/// Records about a subject: the acts first (votes, bills, contracts, amendments: the primary
/// sources), then anything, news included, up to 10 (the acts fill 8 at most). Never the
/// document the claim came from, nor procedural votes.
pub async fn evidence(
    pool: &SqlitePool,
    embedder: &dyn EmbeddingProvider,
    investigation_id: &str,
    query: &str,
    exclude_document: Option<&str>,
) -> Result<Vec<SearchResult>> {
    let mut records: Vec<SearchResult> = Vec::new();
    for (kind, take) in [(Some("vote"), 3), (Some("bill"), 2), (Some("contract"), 2), (Some("amendment"), 1), (None, 5)] {
        let filters = SearchFilters { kind: kind.map(String::from), ..Default::default() };
        for hit in hybrid_search(pool, embedder, &[investigation_id.to_string()], query, &filters, take + 2).await?.results {
            let duplicate = records.iter().any(|r| r.hit.chunk_id == hit.hit.chunk_id || r.hit.document_id == hit.hit.document_id);
            let procedural = hit.hit.kind == "vote" && procedural_vote(&hit.hit.text);
            if Some(hit.hit.document_id.as_str()) != exclude_document && !duplicate && !procedural && records.len() < 10 {
                records.push(hit);
            }
        }
    }
    Ok(records)
}

/// Whose act each record is, next to the claimant (None: a report, such as news).
pub async fn actors(pool: &SqlitePool, investigation_id: &str, claimant: &str, records: &[SearchResult]) -> Result<Vec<Option<actor::Actor>>> {
    let mut actors = Vec::with_capacity(records.len());
    for r in records {
        actors.push(actor::of(pool, investigation_id, claimant, &r.hit.document_id).await?);
    }
    Ok(actors)
}

/// The "Act of" line the model reads for a record: the claimant themselves, or another person.
pub fn act_of(actor: &actor::Actor, claimant_name: &str) -> String {
    match actor.scope.as_str() {
        "self" => format!("{claimant_name} (the politician themselves)"),
        _ => format!("{}{}, another person, not {claimant_name}", actor.name, actor.party.as_deref().map(|p| format!(" ({p})")).unwrap_or_default()),
    }
}

/// A vote whose Sim/Não does not say where the person stands on the subject: a motion about
/// the procedure (urgency, postponing, a separate vote on one part…). Its line is "… votou
/// \"X\" na votação: <what was voted>".
pub fn procedural_vote(text: &str) -> bool {
    const PROCEDURAL: [&str; 11] = [
        "requerimento",
        "destaq",
        "destacad",
        "dtq",
        "retirada de pauta",
        "adiamento",
        "urgencia",
        "preferencia",
        "intersticio",
        "encerramento da discussao",
        "verificacao",
    ];
    let line = text.lines().next().unwrap_or_default();
    let voted = fold(line.split_once("votação:").map_or(line, |(_, what)| what));
    PROCEDURAL.iter().any(|p| voted.contains(p))
}

/// The node a record is in the graph: its vote session, bill or candidacy when official data
/// made one, else the document itself.
async fn evidence_entity(pool: &SqlitePool, inv: &str, document_id: &str, kind: &str, title: &str) -> Result<String> {
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM entities WHERE investigation_id = ? AND document_id = ? AND type IN ('vote_session', 'bill', 'candidacy') LIMIT 1",
    )
    .bind(inv)
    .bind(document_id)
    .fetch_optional(pool)
    .await?;
    match existing {
        Some((id,)) => Ok(id),
        None => graph::entity(pool, inv, kind, &format!("doc:{document_id}"), title, json!({}), Some(document_id)).await,
    }
}

// ===== Review =====

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct DocRef {
    pub document_id: Option<String>,
    pub title: Option<String>,
    pub url: Option<String>,
    #[sqlx(rename = "kind_of_document")]
    pub kind: Option<String>,
    pub site: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    /// CONTRADICTS | FULFILLS
    pub label: String,
    /// suggested | confirmed | rejected
    pub status: String,
    pub confidence: f64,
    pub rationale: Option<String>,
    pub date: String,
    pub excerpt: Option<String>,
    pub start_time: Option<f64>,
    pub evidence: DocRef,
    /// Whose act the evidence is; None for news, pages, videos.
    pub actor: Option<actor::Actor>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimView {
    pub id: String,
    /// promise | statement
    pub kind: String,
    pub text: String,
    pub quote: String,
    pub topic: String,
    pub stated_at: String,
    pub target_id: String,
    pub start_time: Option<f64>,
    pub source: DocRef,
    pub findings: Vec<Finding>,
}

/// contradicted | fulfilled | review | none: where a claim stands, as the Promises page says
/// (claimsService.claimState). Confirmed findings about the claimant's own acts (or reports)
/// decide; suggestions wait for a person.
pub fn claim_state(claim: &ClaimView) -> &'static str {
    let live: Vec<&Finding> = claim.findings.iter().filter(|f| f.status != "rejected").collect();
    let own = |f: &&&Finding| f.actor.as_ref().map_or(true, |a| a.scope == "self");
    if live.iter().filter(own).any(|f| f.label == "CONTRADICTS" && f.status == "confirmed") {
        "contradicted"
    } else if live.iter().filter(own).any(|f| f.label == "FULFILLS" && f.status == "confirmed") {
        "fulfilled"
    } else if live.iter().any(|f| f.status == "suggested") {
        "review"
    } else {
        "none"
    }
}

#[derive(sqlx::FromRow)]
struct ClaimRow {
    id: String,
    kind: String,
    text: String,
    attrs: String,
    #[sqlx(flatten)]
    source: DocRef,
}

#[derive(sqlx::FromRow)]
struct FindingRow {
    claim_id: String,
    id: String,
    label: String,
    status: String,
    confidence: f64,
    rationale: Option<String>,
    date: String,
    props: String,
    #[sqlx(flatten)]
    evidence: DocRef,
}

#[tauri::command]
pub async fn list_claims(state: tauri::State<'_, AppState>, investigation_id: String) -> Result<Vec<ClaimView>, String> {
    claims(state.db_manager.pool(), &investigation_id).await.map_err(|e| e.to_string())
}

/// The claims of an investigation with what the AI found about each, those to review first.
pub async fn claims(pool: &SqlitePool, investigation_id: &str) -> Result<Vec<ClaimView>> {
    let rows: Vec<ClaimRow> = sqlx::query_as(
        "SELECT e.id, e.type AS kind, e.label AS text, e.attrs, d.id AS document_id, d.title, d.url, d.kind AS kind_of_document, d.site
         FROM entities e LEFT JOIN documents d ON d.id = e.document_id
         WHERE e.investigation_id = ? AND e.type IN ('promise', 'statement')",
    )
    .bind(investigation_id)
    .fetch_all(pool)
    .await?;
    let findings: Vec<FindingRow> = sqlx::query_as(
        "SELECT r.from_id AS claim_id, r.id, r.label, r.status, r.confidence, r.rationale, r.date, r.props,
                d.id AS document_id, d.title, d.url, d.kind AS kind_of_document, d.site
         FROM relations r LEFT JOIN documents d ON d.id = r.document_id
         WHERE r.investigation_id = ? AND r.label IN ('CONTRADICTS', 'FULFILLS') ORDER BY r.confidence DESC",
    )
    .bind(investigation_id)
    .fetch_all(pool)
    .await?;
    let claimants: std::collections::HashMap<String, String> = rows
        .iter()
        .map(|r| (r.id.clone(), serde_json::from_str::<serde_json::Value>(&r.attrs).ok().and_then(|a| a["target_id"].as_str().map(String::from)).unwrap_or_default()))
        .collect();
    let mut by_claim: std::collections::HashMap<String, Vec<Finding>> = std::collections::HashMap::new();
    for f in findings {
        let props: serde_json::Value = serde_json::from_str(&f.props).unwrap_or_default();
        // Findings from before actors were known: worked out now, and kept (the Map reads them too)
        let actor = if props.get("scope").is_some() {
            serde_json::from_value(props["actor"].clone()).ok()
        } else {
            let claimant = claimants.get(&f.claim_id).cloned().unwrap_or_default();
            let actor = match &f.evidence.document_id {
                Some(doc) => actor::of(pool, investigation_id, &claimant, doc).await?,
                None => None,
            };
            let patch = json!({"actor": actor, "scope": actor.as_ref().map_or("none", |a| a.scope.as_str())});
            sqlx::query("UPDATE relations SET props = json_patch(props, ?) WHERE id = ?").bind(patch.to_string()).bind(&f.id).execute(pool).await?;
            actor
        };
        by_claim.entry(f.claim_id).or_default().push(Finding {
            actor,
            id: f.id,
            label: f.label,
            status: f.status,
            confidence: f.confidence,
            rationale: f.rationale,
            date: f.date,
            excerpt: props["excerpt"].as_str().map(String::from),
            start_time: props["start_time"].as_f64(),
            evidence: f.evidence,
        });
    }
    let mut claims: Vec<ClaimView> = rows
        .into_iter()
        .map(|row| {
            let attrs: serde_json::Value = serde_json::from_str(&row.attrs).unwrap_or_default();
            let field = |name: &str| attrs[name].as_str().unwrap_or_default().to_string();
            ClaimView {
                findings: by_claim.remove(&row.id).unwrap_or_default(),
                quote: field("quote"),
                topic: field("topic"),
                stated_at: field("stated_at"),
                target_id: field("target_id"),
                start_time: attrs["start_time"].as_f64(),
                id: row.id,
                kind: row.kind,
                text: row.text,
                source: row.source,
            }
        })
        .collect();
    // What needs a look first: suggestions, then contradictions, then the newest
    let rank = |c: &ClaimView| (!c.findings.iter().any(|f| f.status == "suggested"), !c.findings.iter().any(|f| f.label == "CONTRADICTS" && f.status != "rejected"));
    claims.sort_by(|a, b| rank(a).cmp(&rank(b)).then(b.stated_at.cmp(&a.stated_at)));
    Ok(claims)
}

/// Confirms or rejects what the AI suggested (or puts it back to suggested).
#[tauri::command]
pub async fn set_finding_status(state: tauri::State<'_, AppState>, id: String, status: String) -> Result<(), String> {
    if !matches!(status.as_str(), "suggested" | "confirmed" | "rejected") {
        return Err(format!("Unknown status: {status}"));
    }
    sqlx::query("UPDATE relations SET status = ? WHERE id = ? AND origin = 'llm'")
        .bind(status)
        .bind(id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Removes a claim the AI read wrong, with what it found about it.
#[tauri::command]
pub async fn delete_claim(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM entities WHERE id = ? AND type IN ('promise', 'statement')")
        .bind(id)
        .execute(state.db_manager.pool())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisStatus {
    /// Documents of this investigation waiting to be read.
    pub pending: i64,
    pub analyzed: i64,
    pub failed: i64,
}

#[tauri::command]
pub async fn analysis_status(state: tauri::State<'_, AppState>, investigation_id: String) -> Result<AnalysisStatus, String> {
    let (pending, analyzed, failed): (i64, i64, i64) = sqlx::query_as(&format!(
        "SELECT COALESCE(SUM(analyzed_at IS NULL), 0), COALESCE(SUM(analyzed_at IS NOT NULL AND analysis_error IS NULL), 0),
                COALESCE(SUM(analysis_error IS NOT NULL), 0)
         FROM documents WHERE investigation_id = ? AND kind IN ({ANALYZED_KINDS})"
    ))
    .bind(investigation_id)
    .fetch_one(state.db_manager.pool())
    .await
    .map_err(|e| e.to_string())?;
    Ok(AnalysisStatus { pending, analyzed, failed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::{save_document, NewDocument};
    use crate::rag::indexer::index_document;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// One dimension per keyword.
    struct Keywords;

    #[async_trait]
    impl EmbeddingProvider for Keywords {
        fn model_id(&self) -> &str {
            "kw"
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            const VOCAB: &[&str] = &["imposto", "creche", "votou"];
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
        replies: Mutex<Vec<String>>,
        prompts: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ChatModel for Scripted {
        async fn complete(&self, _system: &str, user: &str) -> Result<String> {
            self.prompts.lock().unwrap().push(user.to_string());
            Ok(self.replies.lock().unwrap().remove(0))
        }
    }

    #[tokio::test]
    async fn a_promise_and_the_vote_against_it_become_a_suggested_contradiction() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Teste', '2026')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', 'Marina Exemplo', '2026')")
            .execute(&pool)
            .await
            .unwrap();
        let doc = |url: &'static str, kind: &'static str, title: &'static str, text: &'static str, date: &'static str| NewDocument {
            investigation_id: "inv",
            source_id: None,
            url,
            kind,
            title,
            author: Some("Marina Exemplo"),
            site: None,
            published_at: Some(date),
            lang: None,
            text,
            segments: None,
            note: None,
        };
        let speech = save_document(&pool, doc("u1", "speech", "Discurso", "Eu vou votar pela redução do imposto sobre alimentos.", "2022-09-01"))
            .await
            .unwrap()
            .unwrap();
        let vote = save_document(&pool, doc("u2", "vote", "Votação imposto", "Marina Exemplo votou Não na redução do imposto sobre alimentos.", "2023-07-06"))
            .await
            .unwrap()
            .unwrap();
        index_document(&pool, &Keywords, &speech).await.unwrap();
        index_document(&pool, &Keywords, &vote).await.unwrap();
        sqlx::query("UPDATE documents SET analyzed_at = 'x' WHERE id = ?").bind(&vote).execute(&pool).await.unwrap();

        let llm = Scripted {
            replies: Mutex::new(vec![
                r#"{"claims": [{"person": "Marina Exemplo", "kind": "promise", "claim": "Marina promete votar pela redução do imposto sobre alimentos.", "quote": "vou votar pela redução do imposto sobre alimentos", "topic": "imposto sobre alimentos"}]}"#.into(),
                r#"{"judgements": [{"record": 1, "same_subject": true, "relation": "CONTRADICTS", "confidence": 0.9, "rationale": "Prometeu reduzir e votou contra."}]}"#.into(),
            ]),
            prompts: Mutex::new(Vec::new()),
        };
        assert_eq!(pending(&pool).await.unwrap(), 1);
        assert_eq!(run_batch(&pool, &Keywords, &llm, 5, |_| {}).await.unwrap(), 1);
        assert_eq!(pending(&pool).await.unwrap(), 0);
        // The judge saw the vote, with its date
        assert!(llm.prompts.lock().unwrap()[1].contains("vote · 2023-07-06"));

        let (label, status, rationale, evidence): (String, String, String, String) =
            sqlx::query_as("SELECT label, status, rationale, document_id FROM relations WHERE label = 'CONTRADICTS'").fetch_one(&pool).await.unwrap();
        assert_eq!((label.as_str(), status.as_str(), evidence.as_str()), ("CONTRADICTS", "suggested", vote.as_str()));
        assert_eq!(rationale, "Prometeu reduzir e votou contra.");
        let (made,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM relations WHERE label = 'MADE'").fetch_one(&pool).await.unwrap();
        assert_eq!(made, 1);

        // As the review page reads it
        let listed = claims(&pool, "inv").await.unwrap();
        assert_eq!(listed.len(), 1);
        let claim = &listed[0];
        assert_eq!((claim.kind.as_str(), claim.target_id.as_str(), claim.quote.as_str()), ("promise", "t1", "vou votar pela redução do imposto sobre alimentos."));
        assert_eq!(claim.source.kind.as_deref(), Some("speech"));
        assert_eq!(claim.findings.len(), 1);
        assert_eq!(claim.findings[0].evidence.title.as_deref(), Some("Votação imposto"));
        assert_eq!(claim.findings[0].evidence.kind.as_deref(), Some("vote"));
        assert_eq!(claim.findings[0].date, "2023-07-06");
    }

    #[test]
    fn procedural_votes_are_not_evidence() {
        let vote = |what: &str| format!("Em 30/05/2023, na Câmara dos Deputados (PLEN), Fulana votou \"Sim\" na votação: {what}\nProposição: PL 490/2007 – Requerimento antigo na ementa.");
        assert!(procedural_vote(&vote("Rejeitado o Requerimento de retirada de pauta.")));
        assert!(procedural_vote(&vote("Aprovado o Requerimento de Urgência (Art. 155 do RICD).")));
        assert!(procedural_vote(&vote("Mantido o texto destacado (DTQ 3).")));
        assert!(!procedural_vote(&vote("Aprovado o Substitutivo adotado pelo relator.")));
        assert!(!procedural_vote(&vote("Aprovada a Redação Final.")));
    }

    #[test]
    fn long_documents_are_read_in_batches() {
        let chunk = |n: usize| ChunkRow { id: n.to_string(), text: "x".repeat(2500), start_time: None };
        let chunks: Vec<ChunkRow> = (0..5).map(chunk).collect();
        let sizes: Vec<usize> = batches(&chunks).iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![2, 2, 1]);
    }
}
