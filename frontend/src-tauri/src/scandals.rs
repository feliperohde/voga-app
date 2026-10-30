//! Scandals: a news item a person marked as one, with who is involved (INVOLVED_IN, confirmed),
//! and the AI's suggestions of the same from the news it reads (suggested until a person decides).
//! Apart from them, from public records: the sanctioned companies a person is tied to (what they
//! spent with, what their amendments paid or funded, what their city hall contracted): two facts
//! side by side, never an accusation.

use std::collections::HashMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::analyze::claims::{locate_quote, Person};
use crate::collect::{contains_phrase, fold};
use crate::graph;
use crate::llm::ChatModel;
use crate::state::AppState;

/// Folded words that make a news item worth asking the AI about.
const SIGNS: &[&str] = &[
    "operacao", "denuncia", "denunciad", "investigad", "inquerito", "indiciad", "reu ", "condenad", "improbidade", "corrupcao", "propina",
    "lavagem", "desvio", "fraude", "peculato", "busca e apreensao", "preso", "prisao", "cassacao", "cassad", "ministerio publico", "policia federal",
];
/// What the AI is asked about: what people read, not records or speeches.
pub const NEWS_KINDS: &[&str] = &["article", "page", "post", "video"];
const CONFIDENCE: f64 = 0.7;

pub const SYSTEM: &str = "You read an excerpt of Brazilian news and say whether it reports an accusation, a police or prosecutors' investigation, \
an indictment, a conviction or a police operation against one of the people listed. \
Reply with ONLY a JSON object, no prose: {\"reports\": [{\"person\": string, \"case\": string, \"quote\": string}]}.\n\
- person: one of the people listed, written as listed.\n\
- case: a short name for the case in Brazilian Portuguese (\"Operação Exemplo\", \"Compra de respiradores\"); when it is one of the known cases, write that name exactly.\n\
- quote: the exact words of the excerpt that report it, copied.\n\
Leave out opinions, rumors without a source, accusations the person makes against others, and people not listed. \
No such report: {\"reports\": []}.";

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub target_id: String,
    pub case: String,
    pub quote: String,
}

/// Whether a news text is worth asking about: it names someone followed and a sign of a case.
pub fn worth_asking(people: &[Person], text: &str) -> bool {
    let folded = fold(text);
    SIGNS.iter().any(|sign| folded.contains(sign))
        && people.iter().any(|p| std::iter::once(&p.name).chain(&p.aliases).any(|name| contains_phrase(&folded, &fold(name))))
}

pub fn prompt(people: &[Person], cases: &[String], document: &str, excerpt: &str) -> String {
    let people: Vec<&str> = people.iter().map(|p| p.name.as_str()).collect();
    let cases = if cases.is_empty() { "none".to_string() } else { cases.join("; ") };
    format!("People followed: {}\nKnown cases: {cases}\n\nDocument: {document}\n\n<excerpt>\n{excerpt}\n</excerpt>", people.join("; "))
}

/// The reports in the AI's reply whose person is followed and whose words are in the excerpt.
pub fn parse(reply: &str, people: &[Person], excerpt: &str) -> Vec<Report> {
    #[derive(Deserialize)]
    struct Raw {
        #[serde(default)]
        reports: Vec<RawReport>,
    }
    #[derive(Deserialize)]
    struct RawReport {
        #[serde(default)]
        person: String,
        #[serde(default)]
        case: String,
        #[serde(default)]
        quote: String,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else { return Vec::new() };
    let Ok(raw) = serde_json::from_str::<Raw>(reply.get(start..=end).unwrap_or_default()) else { return Vec::new() };
    raw.reports
        .into_iter()
        .filter_map(|r| {
            let person = people.iter().find(|p| p.is(&r.person))?;
            let case = r.case.trim();
            let quote = locate_quote(excerpt, &r.quote)?;
            (!case.is_empty()).then(|| Report { target_id: person.target_id.clone(), case: case.to_string(), quote })
        })
        .collect()
}

/// The person entity of a target (made when missing).
async fn person(pool: &SqlitePool, investigation_id: &str, target_id: &str) -> Result<String> {
    let (name,): (String,) = sqlx::query_as("SELECT name FROM targets WHERE id = ? AND investigation_id = ?").bind(target_id).bind(investigation_id).fetch_one(pool).await?;
    graph::entity(pool, investigation_id, "person", &format!("target:{target_id}"), &name, json!({}), None).await
}

/// Asks the AI whether a news excerpt reports a case against someone followed, and suggests each
/// one it reports (a case it names as a known one joins it). Returns how many.
pub async fn suggest(pool: &SqlitePool, llm: &dyn ChatModel, investigation_id: &str, document_id: &str, people: &[Person], header: &str, excerpt: &str) -> Result<usize> {
    let known: Vec<(String, String)> = sqlx::query_as("SELECT key, label FROM entities WHERE investigation_id = ? AND type = 'scandal'").bind(investigation_id).fetch_all(pool).await?;
    let cases: Vec<String> = known.iter().map(|(_, label)| label.clone()).collect();
    let reply = llm.complete(SYSTEM, &prompt(people, &cases, header, excerpt)).await?;
    let reports = parse(&reply, people, excerpt);
    for report in &reports {
        let key = known.iter().find(|(_, label)| fold(label) == fold(&report.case)).map(|(key, _)| key.clone()).unwrap_or_else(|| {
            let digest = Sha256::digest(fold(&report.case).as_bytes());
            format!("scandal:{:x}", digest)[..24].to_string()
        });
        let scandal = graph::entity(pool, investigation_id, "scandal", &key, &report.case, json!({}), Some(document_id)).await?;
        let from = person(pool, investigation_id, &report.target_id).await?;
        graph::suggest(pool, investigation_id, &from, &scandal, "INVOLVED_IN", "", json!({"quote": report.quote}), Some(document_id), &report.quote, CONFIDENCE).await?;
    }
    Ok(reports.len())
}

/// Marks a document as a scandal involving `target_ids` (a person's decision: confirmed).
pub async fn mark(pool: &SqlitePool, document_id: &str, title: &str, date: &str, target_ids: &[String]) -> Result<String> {
    if title.trim().is_empty() || target_ids.is_empty() {
        bail!("A scandal needs a name and someone involved");
    }
    let (investigation_id,): (String,) = sqlx::query_as("SELECT investigation_id FROM documents WHERE id = ?").bind(document_id).fetch_one(pool).await?;
    let key = format!("scandal:{}", uuid::Uuid::new_v4());
    let scandal = graph::entity(pool, &investigation_id, "scandal", &key, title.trim(), json!({"date": date}), Some(document_id)).await?;
    sqlx::query("UPDATE entities SET origin = 'user' WHERE id = ?").bind(&scandal).execute(pool).await?;
    for target_id in target_ids {
        let from = person(pool, &investigation_id, target_id).await?;
        graph::relate(pool, &investigation_id, &from, &scandal, "INVOLVED_IN", "", json!({}), Some(document_id)).await?;
        sqlx::query("UPDATE relations SET origin = 'user' WHERE from_id = ? AND to_id = ? AND label = 'INVOLVED_IN'").bind(&from).bind(&scandal).execute(pool).await?;
    }
    Ok(scandal)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Involved {
    /// The INVOLVED_IN relation (reviewed with set_finding_status when the AI suggested it).
    pub relation_id: String,
    #[serde(skip)]
    pub scandal_id: String,
    pub target_id: Option<String>,
    pub name: String,
    /// suggested | confirmed | rejected
    pub status: String,
    /// user | llm
    pub origin: String,
    /// The words of the news (an AI suggestion).
    pub rationale: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scandal {
    pub id: String,
    pub investigation_id: String,
    pub title: String,
    /// When it happened, as the person who marked it said; else when the news came out.
    pub date: Option<String>,
    pub document_id: Option<String>,
    pub document_title: Option<String>,
    pub people: Vec<Involved>,
}

/// One way a person is tied to a sanctioned company.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tie {
    /// SPENT_WITH, or PAID_TO / FUNDS / CONTRACTED through `via` (an amendment, a city hall).
    pub label: String,
    pub via: Option<String>,
    pub date: String,
    pub props: Value,
    pub document_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SanctionedTie {
    pub investigation_id: String,
    pub target_id: String,
    pub person: String,
    pub company_id: String,
    pub company: String,
    pub sanctions: Value,
    pub ties: Vec<Tie>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scandals {
    pub scandals: Vec<Scandal>,
    pub sanctioned: Vec<SanctionedTie>,
}

#[derive(sqlx::FromRow)]
struct TieRow {
    target_id: String,
    person: String,
    company_id: String,
    company: String,
    sanctions: String,
    label: String,
    via: Option<String>,
    date: String,
    props: String,
    document_id: Option<String>,
}

/// The sanctioned companies each person of an investigation is tied to, one or two steps away.
pub async fn sanctioned(pool: &SqlitePool, investigation_id: &str) -> Result<Vec<SanctionedTie>> {
    let rows: Vec<TieRow> = sqlx::query_as(
        "WITH people AS (
            SELECT e.id, t.id AS target_id, t.name FROM entities e JOIN targets t ON e.key = 'target:' || t.id
            WHERE e.investigation_id = ?1 AND t.kind = 'person'),
         first AS (
            SELECT p.target_id, p.name, r.to_id, r.label, NULL AS via, r.date, r.props, r.document_id FROM relations r JOIN people p ON p.id = r.from_id
            WHERE r.investigation_id = ?1 AND r.status = 'confirmed'),
         second AS (
            SELECT f.target_id, f.name, r.to_id, r.label, v.label AS via, r.date, r.props, r.document_id FROM relations r
            JOIN first f ON f.to_id = r.from_id JOIN entities v ON v.id = r.from_id
            WHERE r.investigation_id = ?1 AND r.status = 'confirmed' AND r.label IN ('PAID_TO', 'FUNDS', 'CONTRACTED'))
         SELECT t.target_id, t.name AS person, c.id AS company_id, c.label AS company, json_extract(c.attrs, '$.sanctions') AS sanctions,
                t.label, t.via, t.date, t.props, t.document_id
         FROM (SELECT * FROM first WHERE label = 'SPENT_WITH' UNION ALL SELECT * FROM second) t JOIN entities c ON c.id = t.to_id
         WHERE c.key LIKE 'cnpj:%' AND json_array_length(json_extract(c.attrs, '$.sanctions')) > 0
         ORDER BY t.name, c.label, t.date DESC",
    )
    .bind(investigation_id)
    .fetch_all(pool)
    .await?;
    let mut ties: Vec<SanctionedTie> = Vec::new();
    for row in rows {
        let tie = Tie { label: row.label, via: row.via, date: row.date, props: serde_json::from_str(&row.props).unwrap_or_default(), document_id: row.document_id };
        match ties.iter_mut().find(|t| t.target_id == row.target_id && t.company_id == row.company_id) {
            Some(existing) => existing.ties.push(tie),
            None => ties.push(SanctionedTie {
                investigation_id: investigation_id.to_string(),
                target_id: row.target_id,
                person: row.person,
                company_id: row.company_id,
                company: row.company,
                sanctions: serde_json::from_str(&row.sanctions).unwrap_or_default(),
                ties: vec![tie],
            }),
        }
    }
    Ok(ties)
}

#[derive(sqlx::FromRow)]
struct ScandalRow {
    id: String,
    title: String,
    date: Option<String>,
    document_id: Option<String>,
    document_title: Option<String>,
    published: Option<String>,
}

pub async fn list(pool: &SqlitePool, investigation_id: &str) -> Result<Scandals> {
    let rows: Vec<ScandalRow> = sqlx::query_as(
        "SELECT s.id, s.label AS title, json_extract(s.attrs, '$.date') AS date, s.document_id, d.title AS document_title,
                COALESCE(d.published_at, d.fetched_at) AS published
         FROM entities s LEFT JOIN documents d ON d.id = s.document_id WHERE s.investigation_id = ? AND s.type = 'scandal'",
    )
    .bind(investigation_id)
    .fetch_all(pool)
    .await?;
    let involved: Vec<Involved> = sqlx::query_as(
        "SELECT r.id AS relation_id, r.to_id AS scandal_id, t.id AS target_id, p.label AS name, r.status, r.origin, r.rationale
         FROM relations r JOIN entities p ON p.id = r.from_id LEFT JOIN targets t ON p.key = 'target:' || t.id
         WHERE r.investigation_id = ? AND r.label = 'INVOLVED_IN' AND r.status != 'rejected'",
    )
    .bind(investigation_id)
    .fetch_all(pool)
    .await?;
    let mut by_scandal: HashMap<String, Vec<Involved>> = HashMap::new();
    for person in involved {
        by_scandal.entry(person.scandal_id.clone()).or_default().push(person);
    }
    let mut scandals: Vec<Scandal> = rows
        .into_iter()
        .filter_map(|ScandalRow { id, title, date, document_id, document_title, published }| {
            // A suggestion everyone rejected is gone
            let people = by_scandal.remove(&id)?;
            Some(Scandal { investigation_id: investigation_id.to_string(), date: date.filter(|d| !d.is_empty()).or(published), id, title, document_id, document_title, people })
        })
        .collect();
    scandals.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(Scandals { scandals, sanctioned: sanctioned(pool, investigation_id).await? })
}

// ===== Commands =====

#[tauri::command]
pub async fn mark_scandal(state: tauri::State<'_, AppState>, document_id: String, title: String, date: String, target_ids: Vec<String>) -> Result<String, String> {
    mark(state.db_manager.pool(), &document_id, &title, &date, &target_ids).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_scandals(state: tauri::State<'_, AppState>, investigation_id: String) -> Result<Scandals, String> {
    list(state.db_manager.pool(), &investigation_id).await.map_err(|e| e.to_string())
}

/// Removes a scandal marked by mistake, with who it involved.
#[tauri::command]
pub async fn delete_scandal(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    sqlx::query("DELETE FROM entities WHERE id = ? AND type = 'scandal'").bind(id).execute(state.db_manager.pool()).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people() -> Vec<Person> {
        vec![Person { target_id: "t1".into(), name: "Helena Prado".into(), aliases: vec![] }]
    }

    const NEWS: &str = "A Polícia Federal deflagrou nesta quarta a Operação Ponte Quebrada. Segundo o Ministério Público, a prefeita Helena Prado é investigada por fraude na compra de merenda escolar.";

    #[test]
    fn only_news_naming_someone_and_a_case_are_asked_about() {
        assert!(worth_asking(&people(), NEWS));
        assert!(!worth_asking(&people(), "Helena Prado inaugura creche no bairro."));
        assert!(!worth_asking(&people(), "A Polícia Federal deflagrou uma operação contra fraudes no INSS."));
    }

    #[test]
    fn reports_need_a_followed_person_and_words_from_the_text() {
        let reply = r#"Here: {"reports": [
            {"person": "Helena Prado", "case": "Operação Ponte Quebrada", "quote": "a prefeita Helena Prado é investigada por fraude na compra de merenda escolar"},
            {"person": "Helena Prado", "case": "Outro caso", "quote": "palavras que não estão no texto de jeito nenhum"},
            {"person": "Fulano", "case": "Operação Ponte Quebrada", "quote": "deflagrou nesta quarta a Operação Ponte Quebrada"}]}"#;
        let reports = parse(reply, &people(), NEWS);
        assert_eq!(reports.len(), 1);
        assert_eq!((reports[0].target_id.as_str(), reports[0].case.as_str()), ("t1", "Operação Ponte Quebrada"));
    }

    #[tokio::test]
    async fn marked_scandals_and_sanctioned_ties() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for sql in [
            "INSERT INTO investigations (id, name, kind, created_at) VALUES ('inv', 'Prefeitura', 'politics', '2026')",
            "INSERT INTO targets (id, investigation_id, name, created_at) VALUES ('t1', 'inv', 'Helena Prado', '2026')",
            "INSERT INTO documents (id, investigation_id, url, kind, title, text, fetched_at, content_hash) VALUES ('d1', 'inv', 'https://n.example/1', 'article', 'Operação', 'texto', '2026-03-01', 'h')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        mark(&pool, "d1", "Merenda superfaturada", "2026-02-10", &["t1".to_string()]).await.unwrap();

        // The city hall she heads contracted a company the CGU sanctioned
        let person = person(&pool, "inv", "t1").await.unwrap();
        let hall = graph::entity(&pool, "inv", "organization", "prefeitura:1", "Prefeitura de Cidade Exemplo", json!({}), None).await.unwrap();
        let company = graph::entity(&pool, "inv", "organization", "cnpj:11222333000181", "Gráfica Exemplo", json!({"sanctions": [{"cadastro": "CEIS", "inicio": "2025-01-01"}]}), None).await.unwrap();
        let clean = graph::entity(&pool, "inv", "organization", "cnpj:99000111000122", "Papelaria Exemplo", json!({"sanctions": []}), None).await.unwrap();
        graph::relate(&pool, "inv", &person, &hall, "HEADS", "", json!({}), None).await.unwrap();
        graph::relate(&pool, "inv", &hall, &company, "CONTRACTED", "2026-01-05", json!({"valor": 1000}), None).await.unwrap();
        graph::relate(&pool, "inv", &hall, &clean, "CONTRACTED", "2026-01-06", json!({}), None).await.unwrap();

        let found = list(&pool, "inv").await.unwrap();
        assert_eq!(found.scandals.len(), 1);
        let scandal = &found.scandals[0];
        assert_eq!((scandal.title.as_str(), scandal.date.as_deref()), ("Merenda superfaturada", Some("2026-02-10")));
        assert_eq!((scandal.people[0].name.as_str(), scandal.people[0].status.as_str(), scandal.people[0].origin.as_str()), ("Helena Prado", "confirmed", "user"));
        assert_eq!(found.sanctioned.len(), 1, "only the sanctioned company");
        let tie = &found.sanctioned[0];
        assert_eq!((tie.company.as_str(), tie.ties[0].label.as_str(), tie.ties[0].via.as_deref()), ("Gráfica Exemplo", "CONTRACTED", Some("Prefeitura de Cidade Exemplo")));
    }
}
