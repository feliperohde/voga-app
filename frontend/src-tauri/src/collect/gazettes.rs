//! Municipal official gazettes, searched by name for each target through Querido Diário: its
//! API returns the passages around each mention, saved as they are (a whole gazette is hundreds
//! of pages about everything else). The federal one (DOU) is not collected: in.gov.br's
//! robots.txt disallows everything.

use anyhow::{Context, Result};
use serde_json::Value;

use super::{fetch, NewDocument, SourceRow};
use sqlx::SqlitePool;

const QUERIDO_DIARIO: &str = "https://api.queridodiario.org.br/gazettes";
/// Gazettes taken per run (its date sort is slow on popular names: the default relevance order
/// plus published_since keeps each run small).
const PER_RUN: &str = "10";

/// One gazette mentioning the query: a document per (gazette, query).
pub struct Gazette {
    pub url: String,
    pub title: String,
    pub date: String,
    pub text: String,
}

pub fn parse_querido_diario(body: &Value) -> Vec<Gazette> {
    body["gazettes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let url = g["url"].as_str()?.to_string();
            let city = format!("{} ({})", g["territory_name"].as_str().unwrap_or("?"), g["state_code"].as_str().unwrap_or("?"));
            let date = g["date"].as_str().unwrap_or_default().to_string();
            let edition = g["edition"].as_str().map(|e| format!(", edição {e}")).unwrap_or_default();
            let excerpts: Vec<String> = g["excerpts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                // The PDFs' text keeps their line breaks: one paragraph per excerpt reads better
                .map(|e| e.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect();
            if excerpts.is_empty() {
                return None;
            }
            Some(Gazette {
                title: format!("Diário Oficial de {city} – {date}{edition}"),
                text: format!("Trechos do Diário Oficial de {city} de {date}{edition} que citam a busca:\n\n{}", excerpts.join("\n\n…\n\n")),
                url,
                date,
            })
        })
        .collect()
}

/// Municipal gazettes mentioning `query` published after the last run; saved as documents.
pub async fn run_querido_diario(pool: &SqlitePool, source: &SourceRow) -> Result<Vec<String>> {
    let since: Option<(Option<String>,)> = sqlx::query_as("SELECT last_run_at FROM sources WHERE id = ?").bind(&source.id).fetch_optional(pool).await?;
    let mut url = url::Url::parse(QUERIDO_DIARIO)?;
    url.query_pairs_mut()
        .append_pair("querystring", &source.url)
        .append_pair("size", PER_RUN)
        .append_pair("excerpt_size", "800")
        .append_pair("number_of_excerpts", "5");
    if let Some(date) = since.and_then(|(d,)| d).and_then(|d| d.get(..10).map(str::to_string)) {
        url.query_pairs_mut().append_pair("published_since", &date);
    }
    let body: Value = serde_json::from_slice(&fetch::get_body(url.as_str()).await?.body).context("Unexpected answer from Querido Diário")?;
    let mut saved = Vec::new();
    for gazette in parse_querido_diario(&body) {
        let id = super::save_document(
            pool,
            NewDocument {
                investigation_id: &source.investigation_id,
                source_id: Some(&source.id),
                // The same gazette found by two searches is two documents (each with its passages)
                url: &format!("{}#q={}", gazette.url, urlencode(&source.url)),
                kind: "gazette",
                title: &gazette.title,
                author: None,
                site: Some("Querido Diário"),
                published_at: Some(&gazette.date),
                lang: Some("pt"),
                text: &gazette.text,
                segments: None,
                note: None,
            },
        )
        .await?;
        saved.extend(id);
    }
    Ok(saved)
}

fn urlencode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn querido_diario_gazettes_keep_their_passages() {
        let body = json!({"gazettes": [
            {"url": "https://data.example/a.pdf", "territory_name": "Salto", "state_code": "SP", "date": "2023-05-29", "edition": "1366",
             "excerpts": ["Emenda Nº 41320018 –\n\nDeputada Tabata Amaral – Convênio 937226"]},
            {"url": "https://data.example/b.pdf", "territory_name": "X", "state_code": "PR", "date": "2024-01-01", "excerpts": []}
        ]});
        let found = parse_querido_diario(&body);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Diário Oficial de Salto (SP) – 2023-05-29, edição 1366");
        assert!(found[0].text.contains("Emenda Nº 41320018 – Deputada Tabata Amaral – Convênio 937226"));
    }
}
