//! Several investigations side by side (two people, two subjects, or one of each): the documents
//! they both collected, what has the same key in their graphs (parties, bills, companies), how
//! their people voted on the same votes, the topics of their claims, the passages that sit
//! between their interests and the ones only one of them has, and how much each collected per
//! week. compare_summary has the AI write it up from those facts and passages, citing them.

use std::collections::HashMap;

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::analyze;
use crate::collect::fold;
use crate::llm::{ChatModel, ConfiguredChatModel};
use crate::rag::answer::{cited_indices, excerpt, investigation_context, Citation};
use crate::rag::embeddings::EmbeddingProvider;
use crate::rag::store::{cosine_similarity, decode_vector, RagStore};
use crate::state::AppState;

/// What people say and read (records are compared through the graph instead).
const KINDS: &str = "'article', 'page', 'video', 'post', 'speech', 'live', 'proposal'";
/// Documents of each investigation scored against the others' interests.
const SCORED: i64 = 400;
const BRIDGES: usize = 8;
const DIVERGENT: usize = 3;
const SHARED: usize = 60;
const WEEKS: i64 = 26;
/// A passage at least this close to every other side's interest, relative to its own, connects them.
const BRIDGE: f64 = 0.9;
/// Two claim topics this close (cosine) are the same subject.
const SAME_TOPIC: f64 = 0.8;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Side {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub documents: i64,
    pub targets: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SameDocument {
    pub url: String,
    pub title: String,
    pub document_id: String,
    pub sides: Vec<String>,
}

/// How an investigation's entity is tied to what is shared ("Marina Exemplo · MEMBER_OF").
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedTie {
    pub investigation_id: String,
    pub who: String,
    pub label: String,
    pub date: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shared {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub ties: Vec<SharedTie>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideVote {
    pub investigation_id: String,
    pub who: String,
    pub vote: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteDiff {
    pub label: String,
    pub date: String,
    pub document_id: Option<String>,
    pub votes: Vec<SideVote>,
}

/// The votes every side's people voted in.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Votes {
    pub both: i64,
    pub agreed: i64,
    /// The latest votes where they voted differently.
    pub disagreements: Vec<VoteDiff>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicSide {
    pub investigation_id: String,
    /// The topic as this side's claims name it.
    pub topic: String,
    pub claims: i64,
    pub contradicted: i64,
    pub fulfilled: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    pub sides: Vec<TopicSide>,
}

/// A passage of one investigation and how close it is to each side's interest (in `sides` order).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Passage {
    pub investigation_id: String,
    pub document_id: String,
    pub chunk_id: String,
    pub title: String,
    pub site: Option<String>,
    pub date: String,
    pub scores: Vec<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Week {
    /// Its Monday.
    pub week: String,
    /// Documents per side, in `sides` order.
    pub counts: Vec<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub sides: Vec<Side>,
    pub same_documents: Vec<SameDocument>,
    pub shared: Vec<Shared>,
    pub votes: Votes,
    pub topics: Vec<Topic>,
    /// Passages close to every side's interest: where they connect.
    pub bridges: Vec<Passage>,
    /// Each side's passages furthest from the others: where they diverge.
    pub divergent: Vec<Passage>,
    pub timeline: Vec<Week>,
    /// Why the passages are missing (the embedding model is unavailable).
    pub notice: Option<String>,
}

fn json_ids(ids: &[String]) -> String {
    serde_json::to_string(ids).unwrap_or_default()
}

async fn sides(pool: &SqlitePool, ids: &[String]) -> Result<Vec<Side>> {
    let mut sides = Vec::new();
    for id in ids {
        let (name, kind, documents): (String, String, i64) =
            sqlx::query_as("SELECT name, kind, (SELECT COUNT(*) FROM documents d WHERE d.investigation_id = i.id) FROM investigations i WHERE id = ?").bind(id).fetch_one(pool).await?;
        let targets: Vec<(String,)> = sqlx::query_as("SELECT name FROM targets WHERE investigation_id = ? ORDER BY created_at").bind(id).fetch_all(pool).await?;
        sides.push(Side { id: id.clone(), name, kind, documents, targets: targets.into_iter().map(|(n,)| n).collect() });
    }
    Ok(sides)
}

async fn same_documents(pool: &SqlitePool, ids: &[String]) -> Result<Vec<SameDocument>> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT url, MIN(title), MIN(id), json_group_array(DISTINCT investigation_id) FROM documents
         WHERE investigation_id IN (SELECT value FROM json_each(?1)) GROUP BY url HAVING COUNT(DISTINCT investigation_id) >= 2
         ORDER BY MAX(COALESCE(published_at, fetched_at)) DESC LIMIT 30",
    )
    .bind(json_ids(ids))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(url, title, document_id, sides)| SameDocument { url, title, document_id, sides: serde_json::from_str(&sides).unwrap_or_default() })
        .collect())
}

#[derive(sqlx::FromRow)]
struct TieRow {
    key: String,
    kind: String,
    label: String,
    investigation_id: String,
    who: String,
    relation: String,
    date: String,
    vote: Option<String>,
    document_id: Option<String>,
}

/// What has the same key in more than one investigation (people's own entities and claims apart),
/// with how each is tied to it; and the votes all sides' people voted in.
async fn shared(pool: &SqlitePool, ids: &[String]) -> Result<(Vec<Shared>, Votes)> {
    let rows: Vec<TieRow> = sqlx::query_as(
        "WITH scope AS (SELECT value AS id FROM json_each(?1)),
              keys AS (SELECT key FROM entities WHERE investigation_id IN (SELECT id FROM scope) AND key NOT LIKE 'target:%'
                       AND type NOT IN ('promise', 'statement', 'scandal') GROUP BY key HAVING COUNT(DISTINCT investigation_id) >= 2)
         SELECT e.key, e.type AS kind, e.label, e.investigation_id, f.label AS who, r.label AS relation, r.date,
                json_extract(r.props, '$.vote') AS vote, r.document_id
         FROM entities e JOIN relations r ON r.to_id = e.id AND r.status = 'confirmed' JOIN entities f ON f.id = r.from_id
         WHERE e.investigation_id IN (SELECT id FROM scope) AND e.key IN (SELECT key FROM keys)
         ORDER BY e.key, r.date DESC",
    )
    .bind(json_ids(ids))
    .fetch_all(pool)
    .await?;

    let mut shared: Vec<Shared> = Vec::new();
    let mut sessions: Vec<(String, String, Option<String>, Vec<SideVote>)> = Vec::new();
    for row in rows {
        if row.kind == "vote_session" {
            if let Some(vote) = row.vote {
                match sessions.iter_mut().find(|s| s.0 == row.key) {
                    Some(session) => session.3.push(SideVote { investigation_id: row.investigation_id, who: row.who, vote }),
                    None => sessions.push((row.key, row.label, row.document_id, vec![SideVote { investigation_id: row.investigation_id, who: row.who, vote }])),
                }
            }
            continue;
        }
        let tie = SharedTie { investigation_id: row.investigation_id, who: row.who, label: row.relation, date: row.date };
        match shared.iter_mut().find(|s| s.key == row.key) {
            Some(entity) => entity.ties.push(tie),
            None => shared.push(Shared { key: row.key, kind: row.kind, label: row.label, ties: vec![tie] }),
        }
    }
    // Parties and companies first: they say the most at a glance
    let rank = |kind: &str| match kind {
        "party" => 0,
        "organization" => 1,
        "candidacy" => 2,
        "bill" => 3,
        _ => 4,
    };
    shared.sort_by_key(|s| rank(&s.kind));
    shared.truncate(SHARED);

    let mut votes = Votes::default();
    for (_, label, document_id, cast) in sessions {
        // One vote per side (its first person's), and only where every side voted
        let mut by_side: Vec<SideVote> = Vec::new();
        for vote in cast {
            if !by_side.iter().any(|v| v.investigation_id == vote.investigation_id) {
                by_side.push(vote);
            }
        }
        if by_side.len() < ids.len() {
            continue;
        }
        votes.both += 1;
        if by_side.iter().all(|v| fold(&v.vote) == fold(&by_side[0].vote)) {
            votes.agreed += 1;
        } else if votes.disagreements.len() < 10 {
            let date = label.rsplit('(').next().unwrap_or_default().trim_end_matches(')').to_string();
            votes.disagreements.push(VoteDiff { label, date, document_id, votes: by_side });
        }
    }
    Ok((shared, votes))
}

/// The claims' topics of each political side, the same subject on one row (by meaning when the
/// embedding model answers, else by the same words).
async fn topics(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, sides: &[Side]) -> Result<Vec<Topic>> {
    let mut found: Vec<TopicSide> = Vec::new();
    for side in sides.iter().filter(|s| s.kind == "politics") {
        let mut by_topic: HashMap<String, TopicSide> = HashMap::new();
        for claim in analyze::claims(pool, &side.id).await? {
            if claim.topic.trim().is_empty() {
                continue;
            }
            let entry = by_topic.entry(claim.topic.clone()).or_insert_with(|| TopicSide { investigation_id: side.id.clone(), topic: claim.topic.clone(), ..Default::default() });
            entry.claims += 1;
            match analyze::claim_state(&claim) {
                "contradicted" => entry.contradicted += 1,
                "fulfilled" => entry.fulfilled += 1,
                _ => {}
            }
        }
        found.extend(by_topic.into_values());
    }
    let vectors = match embedder.embed(&found.iter().map(|t| t.topic.clone()).collect::<Vec<_>>()).await {
        Ok(vectors) if vectors.len() == found.len() => Some(vectors),
        _ => None,
    };
    let same = |a: usize, b: usize| match &vectors {
        Some(v) => cosine_similarity(&v[a], &v[b]) >= SAME_TOPIC,
        None => fold(&found[a].topic) == fold(&found[b].topic),
    };
    // Each topic joins the first row holding the same subject and nothing of its side yet
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for i in 0..found.len() {
        match rows.iter_mut().find(|row| row.iter().all(|&j| found[j].investigation_id != found[i].investigation_id) && row.iter().any(|&j| same(i, j))) {
            Some(row) => row.push(i),
            None => rows.push(vec![i]),
        }
    }
    let mut topics: Vec<Topic> = rows.into_iter().map(|row| Topic { sides: row.into_iter().map(|i| found[i].clone()).collect() }).collect();
    // Shared subjects first, then the most talked about
    topics.sort_by_key(|t| (std::cmp::Reverse(t.sides.len()), std::cmp::Reverse(t.sides.iter().map(|s| s.claims).sum::<i64>())));
    Ok(topics)
}

#[derive(sqlx::FromRow)]
struct ChunkRow {
    chunk_id: String,
    document_id: String,
    embedding: Vec<u8>,
    title: String,
    site: Option<String>,
    date: String,
}

/// Each side's latest documents with every passage scored against every side's interest.
async fn passages(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, sides: &[Side]) -> Result<Vec<Passage>> {
    let mut profiles = Vec::new();
    for side in sides {
        profiles.push(crate::feed::interest(pool, embedder, &side.id).await?);
    }
    let mut passages = Vec::new();
    for side in sides {
        let rows: Vec<ChunkRow> = sqlx::query_as(&format!(
            "SELECT c.id AS chunk_id, d.id AS document_id, c.embedding, d.title, d.site, COALESCE(d.published_at, d.fetched_at) AS date FROM rag_chunks c
             JOIN (SELECT * FROM documents WHERE investigation_id = ?1 AND kind IN ({KINDS}) ORDER BY COALESCE(published_at, fetched_at) DESC LIMIT ?3) d ON d.id = c.document_id
             WHERE c.embedding IS NOT NULL AND c.embedding_model = ?2"
        ))
        .bind(&side.id)
        .bind(embedder.model_id())
        .bind(SCORED)
        .fetch_all(pool)
        .await?;
        // A document is as close as its closest passage; the passage shown is the one closest to all sides
        let mut best: HashMap<String, Passage> = HashMap::new();
        for ChunkRow { chunk_id, document_id, embedding, title, site, date } in rows {
            let vector = decode_vector(&embedding);
            let scores: Vec<f64> = profiles.iter().map(|p| cosine_similarity(&vector, p)).collect();
            let worst = scores.iter().cloned().fold(f64::MAX, f64::min);
            let passage = Passage { investigation_id: side.id.clone(), document_id: document_id.clone(), chunk_id, title, site, date, scores };
            match best.get(&document_id) {
                Some(kept) if kept.scores.iter().cloned().fold(f64::MAX, f64::min) >= worst => {}
                _ => {
                    best.insert(document_id, passage);
                }
            }
        }
        passages.extend(best.into_values());
    }
    Ok(passages)
}

/// Passages nearly as close to the other sides' interests as to their own (bridges, closest to
/// every side first), and each side's passages furthest from the others (divergent).
pub fn bridges_and_divergent(sides: &[Side], passages: &[Passage]) -> (Vec<Passage>, Vec<Passage>) {
    let own = |p: &Passage| sides.iter().position(|s| s.id == p.investigation_id).map_or(0.0, |i| p.scores[i]);
    let worst = |p: &Passage| p.scores.iter().cloned().fold(f64::MAX, f64::min);
    let between = |p: &Passage| own(p) > 0.0 && worst(p) >= BRIDGE * own(p);
    let mut bridges: Vec<Passage> = passages.iter().filter(|p| between(p)).cloned().collect();
    bridges.sort_by(|a, b| worst(b).total_cmp(&worst(a)));
    bridges.truncate(BRIDGES);
    let mut divergent: Vec<Passage> = Vec::new();
    for (i, side) in sides.iter().enumerate() {
        let gap = |p: &Passage| p.scores[i] - p.scores.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, s)| *s).fold(f64::MIN, f64::max);
        let mut apart: Vec<&Passage> = passages.iter().filter(|p| p.investigation_id == side.id && !between(p)).collect();
        apart.sort_by(|a, b| gap(b).total_cmp(&gap(a)));
        divergent.extend(apart.into_iter().take(DIVERGENT).cloned());
    }
    (bridges, divergent)
}

async fn timeline(pool: &SqlitePool, sides: &[Side]) -> Result<Vec<Week>> {
    let ids: Vec<String> = sides.iter().map(|s| s.id.clone()).collect();
    let rows: Vec<(String, String, i64)> = sqlx::query_as(&format!(
        "SELECT investigation_id, date(COALESCE(published_at, fetched_at), '-6 days', 'weekday 1') AS week, COUNT(*) FROM documents
         WHERE investigation_id IN (SELECT value FROM json_each(?1)) AND kind IN ({KINDS}) AND date(COALESCE(published_at, fetched_at)) >= date('now', ?2)
         GROUP BY 1, 2"
    ))
    .bind(json_ids(&ids))
    .bind(format!("-{} days", WEEKS * 7))
    .fetch_all(pool)
    .await?;
    let today = chrono::Utc::now().date_naive();
    let monday = today - chrono::Duration::days(chrono::Datelike::weekday(&today).num_days_from_monday() as i64);
    Ok((0..WEEKS)
        .rev()
        .map(|back| {
            let week = (monday - chrono::Duration::weeks(back)).to_string();
            let counts = sides.iter().map(|s| rows.iter().find(|(id, w, _)| id == &s.id && w == &week).map_or(0, |r| r.2)).collect();
            Week { week, counts }
        })
        .collect())
}

pub async fn compare(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, ids: &[String]) -> Result<Comparison> {
    let sides = sides(pool, ids).await?;
    let (shared, votes) = shared(pool, ids).await?;
    let (bridges, divergent, notice) = match passages(pool, embedder, &sides).await {
        Ok(passages) => {
            let (bridges, divergent) = bridges_and_divergent(&sides, &passages);
            (bridges, divergent, None)
        }
        Err(e) => {
            log::warn!("Compare: passages unavailable: {e}");
            (Vec::new(), Vec::new(), Some(e.to_string()))
        }
    };
    Ok(Comparison {
        same_documents: same_documents(pool, ids).await?,
        shared,
        votes,
        topics: topics(pool, embedder, &sides).await?,
        bridges,
        divergent,
        timeline: timeline(pool, &sides).await?,
        notice,
        sides,
    })
}

const SUMMARY_SYSTEM: &str = "You compare investigations a Brazilian user follows (people, or subjects) using ONLY the facts and the numbered excerpts given. \
Write in Brazilian Portuguese, in Markdown, with exactly three sections: \"## Onde se conectam\", \"## Onde divergem\" and \"## O que ainda falta saber\". \
Cite excerpts as [n] after each sentence that uses them. Facts from public records need no citation. Do not invent facts, numbers or motives; \
say so when the material is thin. Keep it under 300 words.";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub text: String,
    pub citations: Vec<Citation>,
}

fn facts(comparison: &Comparison) -> String {
    let name = |id: &str| comparison.sides.iter().find(|s| s.id == id).map_or(id.to_string(), |s| s.name.clone());
    let mut lines = Vec::new();
    for shared in comparison.shared.iter().take(15) {
        let ties: Vec<String> = shared.ties.iter().take(4).map(|t| format!("{} ({}: {})", t.who, name(&t.investigation_id), t.label)).collect();
        lines.push(format!("- {} [{}]: {}", shared.label, shared.kind, ties.join("; ")));
    }
    if comparison.votes.both > 0 {
        lines.push(format!("- They voted in {} of the same votes and agreed in {}.", comparison.votes.both, comparison.votes.agreed));
        for diff in comparison.votes.disagreements.iter().take(5) {
            let cast: Vec<String> = diff.votes.iter().map(|v| format!("{} voted {}", v.who, v.vote)).collect();
            lines.push(format!("  - {}: {}", diff.label, cast.join(", ")));
        }
    }
    for topic in comparison.topics.iter().take(10) {
        let per_side: Vec<String> = topic.sides.iter().map(|s| format!("{}: {} claims about \"{}\"", name(&s.investigation_id), s.claims, s.topic)).collect();
        lines.push(format!("- Topic: {}", per_side.join(" | ")));
    }
    if !comparison.same_documents.is_empty() {
        lines.push(format!("- Both collected {} of the same documents.", comparison.same_documents.len()));
    }
    lines.join("\n")
}

/// The AI's write-up of a comparison: where they connect, where they diverge, what is missing.
pub async fn summarize(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, llm: &dyn ChatModel, ids: &[String]) -> Result<Summary> {
    let comparison = compare(pool, embedder, ids).await?;
    let chosen: Vec<&Passage> = comparison.bridges.iter().take(6).chain(comparison.divergent.iter().take(6)).collect();
    let hits = RagStore::hydrate(pool, chosen.iter().map(|p| (p.chunk_id.clone(), 0.0)).collect()).await?;
    let side = |id: &str| comparison.sides.iter().find(|s| s.id == id).map_or(String::new(), |s| s.name.clone());
    let whose = |hit: &crate::rag::store::ChunkHit| chosen.iter().find(|p| p.chunk_id == hit.chunk_id).map_or(String::new(), |p| side(&p.investigation_id));
    let excerpts: Vec<String> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| format!("[{}] ({}; {}, {}) {}\n{}", i + 1, whose(hit), hit.title, hit.doc_date.get(..10).unwrap_or(&hit.doc_date), hit.site.clone().unwrap_or_default(), hit.text))
        .collect();
    let prompt = format!(
        "{}\n\nFacts from public records and from what a person reviewed:\n{}\n\n<excerpts>\n{}\n</excerpts>\n\nCompare them.",
        investigation_context(pool, ids).await?,
        facts(&comparison),
        excerpts.join("\n\n")
    );
    let text = llm.complete(SUMMARY_SYSTEM, &prompt).await?.trim().to_string();
    let cited = cited_indices(&text, hits.len());
    let citations = hits
        .iter()
        .enumerate()
        .filter(|(i, _)| cited.contains(&(i + 1)))
        .map(|(i, hit)| Citation {
            index: i + 1,
            chunk_id: hit.chunk_id.clone(),
            document_id: hit.document_id.clone(),
            title: hit.title.clone(),
            url: hit.url.clone(),
            site: hit.site.clone(),
            kind: hit.kind.clone(),
            doc_date: hit.doc_date.clone(),
            start_time: hit.start_time,
            excerpt: excerpt(&hit.text),
        })
        .collect();
    Ok(Summary { text, citations })
}

// ===== Commands =====

#[tauri::command]
pub async fn compare_investigations(state: tauri::State<'_, AppState>, investigation_ids: Vec<String>) -> Result<Comparison, String> {
    let pool = state.db_manager.pool();
    let embedder = crate::rag::indexer::embedder(pool).await.map_err(|e| e.to_string())?;
    compare(pool, &embedder, &investigation_ids).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn compare_summary(state: tauri::State<'_, AppState>, investigation_ids: Vec<String>) -> Result<Summary, String> {
    let pool = state.db_manager.pool();
    let embedder = crate::rag::indexer::embedder(pool).await.map_err(|e| e.to_string())?;
    let llm = ConfiguredChatModel::from_settings(pool).await.map_err(|e| e.to_string())?.deterministic();
    summarize(pool, &embedder, &llm, &investigation_ids).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph;
    use serde_json::json;

    /// One dimension per subject: AI, industry, anything else.
    struct Subjects;

    #[async_trait::async_trait]
    impl EmbeddingProvider for Subjects {
        fn model_id(&self) -> &str {
            "subjects"
        }
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| {
                    let words: Vec<String> = fold(text).split(|c: char| !c.is_alphanumeric()).map(String::from).collect();
                    let has = |options: &[&str]| f32::from(u8::from(words.iter().any(|w| options.contains(&w.as_str()))));
                    vec![has(&["ia", "inteligencia"]), has(&["industria", "fabrica", "fabricas"]), 0.1]
                })
                .collect())
        }
    }

    async fn document(pool: &SqlitePool, investigation: &str, id: &str, url: &str, title: &str) {
        sqlx::query("INSERT INTO documents (id, investigation_id, url, kind, title, text, fetched_at, published_at, content_hash) VALUES (?, ?, ?, 'article', ?, ?, '2026-10-20', '2026-10-20', ?)")
            .bind(id)
            .bind(investigation)
            .bind(url)
            .bind(title)
            .bind(title)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        let vector = Subjects.embed(&[title.to_string()]).await.unwrap().remove(0);
        sqlx::query("INSERT INTO rag_chunks (id, investigation_id, document_id, chunk_index, text, doc_date, embedding, embedding_model, dims, created_at) VALUES (?, ?, ?, 0, ?, '2026-10-20', ?, 'subjects', 3, '2026')")
            .bind(format!("c-{id}"))
            .bind(investigation)
            .bind(id)
            .bind(title)
            .bind(crate::rag::store::encode_vector(&vector))
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn two_subjects_side_by_side() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for (id, name) in [("ia", "Inteligência artificial"), ("ind", "Indústria")] {
            sqlx::query("INSERT INTO investigations (id, name, kind, created_at) VALUES (?, ?, 'interest', '2026')").bind(id).bind(name).execute(&pool).await.unwrap();
        }
        document(&pool, "ia", "a1", "https://n.example/modelos", "Novos modelos de IA").await;
        document(&pool, "ia", "a2", "https://n.example/ia-fabricas", "IA chega às fábricas").await;
        document(&pool, "ind", "b1", "https://n.example/aco", "Nova fábrica de aço").await;
        document(&pool, "ind", "b2", "https://n.example/ia-fabricas", "IA chega às fábricas").await;
        // Both have the same party, and a vote they cast differently
        for (id, vote) in [("ia", "Sim"), ("ind", "Não")] {
            let person = graph::entity(&pool, id, "person", &format!("target:{id}"), &format!("Pessoa {id}"), json!({}), None).await.unwrap();
            let party = graph::entity(&pool, id, "party", "party:PDE", "PDE", json!({}), None).await.unwrap();
            let session = graph::entity(&pool, id, "vote_session", "camara:vot:1", "PL 1/2026 (02/10/2026)", json!({}), None).await.unwrap();
            graph::relate(&pool, id, &person, &party, "MEMBER_OF", "", json!({}), None).await.unwrap();
            graph::relate(&pool, id, &person, &session, "VOTED", "2026-10-02", json!({"vote": vote}), None).await.unwrap();
        }

        let ids = ["ia".to_string(), "ind".to_string()];
        let comparison = compare(&pool, &Subjects, &ids).await.unwrap();
        assert_eq!(comparison.same_documents.len(), 1, "the same news in both");
        assert_eq!(comparison.same_documents[0].sides, vec!["ia", "ind"]);
        assert_eq!(comparison.shared.len(), 1);
        assert_eq!((comparison.shared[0].label.as_str(), comparison.shared[0].ties.len()), ("PDE", 2));
        assert_eq!((comparison.votes.both, comparison.votes.agreed, comparison.votes.disagreements.len()), (1, 0, 1));
        // AI in the factories sits between both interests; each side's own subject is where they diverge
        assert!(["a2", "b2"].contains(&comparison.bridges[0].document_id.as_str()));
        let divergent: Vec<&str> = comparison.divergent.iter().map(|p| p.document_id.as_str()).collect();
        assert!(divergent.contains(&"a1") && divergent.contains(&"b1"), "{divergent:?}");
        assert_eq!(comparison.timeline.len(), WEEKS as usize);
        assert!(comparison.notice.is_none());
    }
}
