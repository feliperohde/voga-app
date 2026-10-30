//! A city hall's purchases (licitações) and contracts from the PNCP (Portal Nacional de
//! Contratações Públicas, open API, no key), linked to the mayor: one document per day of
//! publication for each, from the start of the term, a month at a time. The source's id is the
//! city's IBGE code; the hall is spread over many CNPJs (secretariats, subprefeituras, funds…),
//! learned from the purchases published in the city by municipal executive bodies.
//!
//! The graph stays lean: a contract becomes a fact only when its supplier is already in the
//! investigation (a deputy's supplier, an amendment's recipient, a partner) or the contract says
//! which amendment paid it. Every other contract is in the documents, for search and Ask.

use anyhow::Result;
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet};

use super::{brl, Ctx, Doc, OfficialMatch};
use crate::collect::{fetch, fold};
use crate::graph::{entity, relate};

pub const KIND: &str = "pncp_prefeitura";
const API: &str = "https://pncp.gov.br/api/consulta/v1";
const SITE: &str = "PNCP";
/// ponytail: the current mayoral term (2025–2028); take it from the candidacy when older terms matter.
const TERM_START: &str = "2025-01-01";
/// PNCP's modality codes (pregão, dispensa, concorrência…); the purchases search needs one per call.
const MODALITIES: std::ops::RangeInclusive<u8> = 1..=13;
/// Months fetched per run (hourly): a term's backlog takes hours, without holding the worker.
const MONTHS_PER_RUN: usize = 2;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    data: Vec<T>,
    total_paginas: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Body {
    cnpj: String,
    razao_social: String,
    poder_id: Option<String>,
    esfera_id: Option<String>,
}

impl Body {
    /// The city's own executive (not the state's or the Union's bodies in town, nor the Câmara).
    fn municipal_executive(&self) -> bool {
        self.esfera_id.as_deref() == Some("M") && self.poder_id.as_deref() == Some("E")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Unit {
    nome_unidade: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Purchase {
    orgao_entidade: Body,
    unidade_orgao: Option<Unit>,
    ano_compra: i64,
    sequencial_compra: i64,
    objeto_compra: Option<String>,
    valor_total_estimado: Option<f64>,
    valor_total_homologado: Option<f64>,
    modalidade_nome: Option<String>,
    situacao_compra_nome: Option<String>,
    data_publicacao_pncp: String,
    processo: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contract {
    orgao_entidade: Body,
    unidade_orgao: Option<Unit>,
    ano_contrato: i64,
    sequencial_contrato: i64,
    tipo_pessoa: Option<String>,
    ni_fornecedor: Option<String>,
    nome_razao_social_fornecedor: Option<String>,
    objeto_contrato: Option<String>,
    valor_global: Option<f64>,
    data_assinatura: Option<String>,
    data_vigencia_inicio: Option<String>,
    data_vigencia_fim: Option<String>,
    data_publicacao_pncp: String,
    processo: Option<String>,
    emenda_parlamentar: Option<Value>,
}

impl Contract {
    /// The supplier's CNPJ; None for a person (whose CPF is never kept).
    fn cnpj(&self) -> Option<String> {
        let digits: String = self.ni_fornecedor.as_deref().unwrap_or("").chars().filter(char::is_ascii_digit).collect();
        (self.tipo_pessoa.as_deref() == Some("PJ") && digits.len() == 14).then_some(digits)
    }

    /// The Portal da Transparência code of the amendment that paid it, when the contract says.
    fn amendment(&self) -> Option<String> {
        let text = match self.emenda_parlamentar.as_ref()? {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => return None,
        };
        let digits: String = text.chars().filter(char::is_ascii_digit).collect();
        (digits.len() == 12).then_some(digits)
    }
}

fn opt(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("").trim()
}

fn body_name(body: &Body, unit: &Option<Unit>) -> String {
    match unit.as_ref().and_then(|u| u.nome_unidade.as_deref()).filter(|u| !u.is_empty() && *u != body.razao_social) {
        Some(unit) => format!("{} / {unit}", body.razao_social),
        None => body.razao_social.clone(),
    }
}

/// The next days to collect: after the last one done, to the end of its month, never today.
fn window(last_done: NaiveDate, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let from = last_done + Duration::days(1);
    let yesterday = today - Duration::days(1);
    if from > yesterday {
        return None;
    }
    let next_month = if from.month() == 12 { NaiveDate::from_ymd_opt(from.year() + 1, 1, 1) } else { NaiveDate::from_ymd_opt(from.year(), from.month() + 1, 1) }?;
    Some((from, (next_month - Duration::days(1)).min(yesterday)))
}

/// One page of a PNCP search (204: nothing). The PNCP answers 429 when asked too often, even at
/// Voga's 2 s per host: then it waits, longer each time.
async fn page<T: serde::de::DeserializeOwned>(url: &str) -> Result<Page<T>> {
    let mut attempt = 0;
    let fetched = loop {
        match fetch::get_body(url).await {
            Err(e) if attempt < 4 && e.to_string().contains("HTTP 429") => {
                attempt += 1;
                log::info!("PNCP: too many requests, waiting {}s", 30 * attempt);
                tokio::time::sleep(std::time::Duration::from_secs(30 * attempt)).await;
            }
            result => break result?,
        }
    };
    if fetched.body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Page { data: Vec::new(), total_paginas: 0 });
    }
    Ok(serde_json::from_slice(&fetched.body)?)
}

async fn all<T: serde::de::DeserializeOwned>(url: &str, size: u32) -> Result<Vec<T>> {
    let mut items = Vec::new();
    let mut number = 1;
    loop {
        let found: Page<T> = page(&format!("{url}&pagina={number}&tamanhoPagina={size}")).await?;
        items.extend(found.data);
        if number >= found.total_paginas {
            return Ok(items);
        }
        number += 1;
    }
}

fn purchase_line(p: &Purchase) -> String {
    let value = match p.valor_total_homologado {
        Some(v) if v > 0.0 => format!("homologado {}", brl(v)),
        _ => format!("estimado {}", brl(p.valor_total_estimado.unwrap_or(0.0))),
    };
    format!(
        "- {}: {} ({}; {}; {}; processo {}) https://pncp.gov.br/app/editais/{}/{}/{}",
        body_name(&p.orgao_entidade, &p.unidade_orgao),
        opt(&p.objeto_compra),
        opt(&p.modalidade_nome),
        value,
        opt(&p.situacao_compra_nome),
        opt(&p.processo),
        p.orgao_entidade.cnpj,
        p.ano_compra,
        p.sequencial_compra
    )
}

fn contract_line(c: &Contract) -> String {
    let supplier = match c.cnpj() {
        Some(cnpj) => format!("{} (CNPJ {cnpj})", opt(&c.nome_razao_social_fornecedor)),
        None => format!("{} (pessoa física)", opt(&c.nome_razao_social_fornecedor)),
    };
    let amendment = c.amendment().map(|a| format!("; emenda parlamentar {a}")).unwrap_or_default();
    format!(
        "- {}: {supplier} – {} – {}; assinado em {}, vigência de {} a {}; processo {}{amendment} https://pncp.gov.br/app/contratos/{}/{}/{}",
        body_name(&c.orgao_entidade, &c.unidade_orgao),
        opt(&c.objeto_contrato),
        brl(c.valor_global.unwrap_or(0.0)),
        opt(&c.data_assinatura),
        opt(&c.data_vigencia_inicio),
        opt(&c.data_vigencia_fim),
        opt(&c.processo),
        c.orgao_entidade.cnpj,
        c.ano_contrato,
        c.sequencial_contrato
    )
}

async fn find_entity(pool: &SqlitePool, investigation_id: &str, key: &str) -> Result<Option<String>> {
    Ok(sqlx::query_as::<_, (String,)>("SELECT id FROM entities WHERE investigation_id = ? AND key = ?").bind(investigation_id).bind(key).fetch_optional(pool).await?.map(|(id,)| id))
}

/// A contract as a fact, when the investigation already follows its supplier or its amendment.
async fn link_contract(pool: &SqlitePool, investigation_id: &str, hall: &str, c: &Contract, document_id: &str) -> Result<()> {
    let Some(cnpj) = c.cnpj() else { return Ok(()) };
    let key = format!("cnpj:{cnpj}");
    let amendment = match c.amendment() {
        Some(code) => find_entity(pool, investigation_id, &format!("emenda:{code}")).await?,
        None => None,
    };
    let supplier = match (find_entity(pool, investigation_id, &key).await?, &amendment) {
        (Some(id), _) => id,
        (None, Some(_)) => entity(pool, investigation_id, "organization", &key, opt(&c.nome_razao_social_fornecedor), json!({"cnpj": cnpj}), None).await?,
        (None, None) => return Ok(()),
    };
    let date = opt(&c.data_assinatura).to_string();
    let objeto: String = opt(&c.objeto_contrato).chars().take(200).collect();
    let props = json!({"valor": c.valor_global, "objeto": objeto, "orgao": c.orgao_entidade.razao_social, "pncp": format!("{}/{}/{}", c.orgao_entidade.cnpj, c.ano_contrato, c.sequencial_contrato)});
    relate(pool, investigation_id, hall, &supplier, "CONTRACTED", &date, props.clone(), Some(document_id)).await?;
    if let Some(amendment) = amendment {
        relate(pool, investigation_id, &amendment, &supplier, "FUNDS", &date, props, Some(document_id)).await?;
    }
    Ok(())
}

fn by_day<T>(items: Vec<T>, day: impl Fn(&T) -> String) -> BTreeMap<String, Vec<T>> {
    let mut days: BTreeMap<String, Vec<T>> = BTreeMap::new();
    for item in items {
        days.entry(day(&item)).or_default().push(item);
    }
    days
}

fn br_date(iso: &str) -> String {
    match (iso.get(..4), iso.get(5..7), iso.get(8..10)) {
        (Some(y), Some(m), Some(d)) => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

pub async fn run(ctx: &mut Ctx<'_>) -> Result<()> {
    let ibge = ctx.source.url.clone();
    let hall_name = ctx.source.title.as_deref().and_then(|t| t.split(" · ").next()).unwrap_or("Prefeitura").to_string();
    let hall = ctx.entity("organization", &format!("prefeitura:{ibge}"), &hall_name, json!({"ibge": ibge}), None).await?;
    let person = ctx.person.clone();
    ctx.relate(&person, &hall, "HEADS", TERM_START, json!({}), None).await?;
    let start = NaiveDate::parse_from_str(TERM_START, "%Y-%m-%d")?;
    for _ in 0..MONTHS_PER_RUN {
        let last_done = ctx.cursor("day").await?.and_then(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok()).unwrap_or(start - Duration::days(1));
        let Some((from, to)) = window(last_done, Utc::now().date_naive()) else { break };
        collect_window(ctx, &ibge, &hall_name, &hall, from, to).await?;
    }
    Ok(())
}

/// The purchases and contracts published from `from` to `to`; the cursor moves past them.
async fn collect_window(ctx: &mut Ctx<'_>, ibge: &str, hall_name: &str, hall: &str, from: NaiveDate, to: NaiveDate) -> Result<()> {
    let (a, b) = (from.format("%Y%m%d"), to.format("%Y%m%d"));

    let mut bodies: BTreeSet<String> = ctx.cursor("bodies").await?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    let mut purchases: Vec<Purchase> = Vec::new();
    for modality in MODALITIES {
        let url = format!("{API}/contratacoes/publicacao?dataInicial={a}&dataFinal={b}&codigoModalidadeContratacao={modality}&codigoMunicipioIbge={ibge}");
        for p in all::<Purchase>(&url, 50).await? {
            if p.orgao_entidade.municipal_executive() {
                bodies.insert(p.orgao_entidade.cnpj.clone());
                purchases.push(p);
            }
        }
    }
    // ponytail: a body that signs contracts without ever publishing a purchase here is missed until it does
    let mut contracts: Vec<Contract> = Vec::new();
    for cnpj in &bodies {
        contracts.extend(all::<Contract>(&format!("{API}/contratos?dataInicial={a}&dataFinal={b}&cnpjOrgao={cnpj}"), 500).await?);
    }

    let day = |iso: &String| iso.get(..10).unwrap_or("").to_string();
    for (day, list) in by_day(purchases, |p| day(&p.data_publicacao_pncp)) {
        let total: f64 = list.iter().map(|p| p.valor_total_estimado.unwrap_or(0.0)).sum();
        let title = format!("Licitações publicadas pela {hall_name} em {}: {} ({} estimados)", br_date(&day), list.len(), brl(total));
        let text = format!("{title}.\n\n{}", list.iter().map(purchase_line).collect::<Vec<_>>().join("\n"));
        let url = format!("https://pncp.gov.br/app/editais#prefeitura-{ibge}-{day}");
        ctx.document(Doc { url: &url, kind: "procurement", title: &title, author: None, site: Some(SITE), published_at: Some(&day), text: &text, note: None }).await?;
    }
    for (day, list) in by_day(contracts, |c| day(&c.data_publicacao_pncp)) {
        let total: f64 = list.iter().map(|c| c.valor_global.unwrap_or(0.0)).sum();
        let title = format!("Contratos publicados pela {hall_name} em {}: {} ({})", br_date(&day), list.len(), brl(total));
        let text = format!("{title}.\n\n{}", list.iter().map(contract_line).collect::<Vec<_>>().join("\n"));
        let url = format!("https://pncp.gov.br/app/contratos#prefeitura-{ibge}-{day}");
        let doc = ctx.document(Doc { url: &url, kind: "contract", title: &title, author: None, site: Some(SITE), published_at: Some(&day), text: &text, note: None }).await?;
        for c in &list {
            link_contract(ctx.pool, ctx.investigation(), hall, c, &doc).await?;
        }
    }

    ctx.set_cursor("bodies", &serde_json::to_string(&bodies)?).await?;
    ctx.set_cursor("day", &to.format("%Y-%m-%d").to_string()).await

}

#[derive(Debug, Deserialize)]
struct City {
    id: i64,
    nome: String,
}

/// The city hall of a mayor's candidacy (the TSE gives only the city's name), to link.
pub async fn city_hall(uf: &str, city: &str) -> Result<Option<OfficialMatch>> {
    let cities: Vec<City> = fetch::get_json(&format!("https://servicodados.ibge.gov.br/api/v1/localidades/estados/{uf}/municipios")).await?;
    let wanted = fold(city);
    Ok(cities.into_iter().find(|c| fold(&c.nome) == wanted).map(|c| OfficialMatch {
        kind: KIND.into(),
        id: c.id.to_string(),
        label: format!("Prefeitura de {}", c.nome),
        detail: format!("PNCP · licitações e contratos da prefeitura ({uf}) desde {}", br_date(TERM_START)),
        similar: false,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract(cnpj: &str, kind: &str, amendment: Value) -> Contract {
        serde_json::from_value(json!({
            "orgaoEntidade": {"cnpj": "13864377000130", "razaoSocial": "FUNDO MUNICIPAL DE SAUDE", "poderId": "E", "esferaId": "M"},
            "unidadeOrgao": {"nomeUnidade": "COORDENADORIA LESTE"},
            "anoContrato": 2026, "sequencialContrato": 9, "tipoPessoa": kind, "niFornecedor": cnpj,
            "nomeRazaoSocialFornecedor": "GRAFICA AURORA LTDA", "objetoContrato": "Impressão de cartilhas", "valorGlobal": 1000.5,
            "dataAssinatura": "2026-01-26", "dataVigenciaInicio": "2026-02-06", "dataVigenciaFim": "2026-12-31",
            "dataPublicacaoPncp": "2026-09-01T10:50:50", "processo": "6018.2026/1", "emendaParlamentar": amendment
        }))
        .unwrap()
    }

    #[test]
    fn windows_stay_in_a_month_and_never_reach_today() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(window(d("2024-12-31"), d("2026-10-15")), Some((d("2025-01-01"), d("2025-01-31"))));
        assert_eq!(window(d("2025-12-31"), d("2026-10-15")), Some((d("2026-01-01"), d("2026-01-31"))));
        assert_eq!(window(d("2026-09-30"), d("2026-10-15")), Some((d("2026-10-01"), d("2026-10-14"))));
        assert_eq!(window(d("2026-10-14"), d("2026-10-15")), None);
    }

    #[tokio::test]
    async fn contracts_become_facts_only_for_suppliers_or_amendments_already_followed() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'T', '2026')").execute(&pool).await.unwrap();
        let hall = entity(&pool, "inv", "organization", "prefeitura:3550308", "Prefeitura de São Paulo", json!({}), None).await.unwrap();
        let followed = entity(&pool, "inv", "organization", "cnpj:11222333000181", "Gráfica", json!({"cnpj": "11222333000181"}), None).await.unwrap();
        let amendment = entity(&pool, "inv", "amendment", "emenda:202641320008", "Emenda", json!({}), None).await.unwrap();
        let doc = crate::collect::save_document(
            &pool,
            crate::collect::NewDocument { investigation_id: "inv", source_id: None, url: "u", kind: "contract", title: "t", author: None, site: None, published_at: None, lang: None, text: "x", segments: None, note: None },
        )
        .await
        .unwrap()
        .unwrap();
        let count = |sql: &'static str| {
            let pool = pool.clone();
            async move { sqlx::query_as::<_, (i64,)>(sql).fetch_one(&pool).await.unwrap().0 }
        };

        // Unknown supplier, no amendment: only in the document
        link_contract(&pool, "inv", &hall, &contract("99888777000166", "PJ", Value::Null), &doc).await.unwrap();
        assert_eq!(count("SELECT COUNT(*) FROM entities WHERE key = 'cnpj:99888777000166'").await, 0);
        // A person: never a node
        link_contract(&pool, "inv", &hall, &contract("12345678901", "PF", Value::Null), &doc).await.unwrap();
        assert_eq!(count("SELECT COUNT(*) FROM relations").await, 0);
        // A supplier already followed
        link_contract(&pool, "inv", &hall, &contract("11.222.333/0001-81", "PJ", Value::Null), &doc).await.unwrap();
        let (to,): (String,) = sqlx::query_as("SELECT to_id FROM relations WHERE label = 'CONTRACTED'").fetch_one(&pool).await.unwrap();
        assert_eq!(to, followed);
        // A new supplier paid by a followed amendment
        link_contract(&pool, "inv", &hall, &contract("55666777000188", "PJ", json!("2026.4132.0008")), &doc).await.unwrap();
        let (from,): (String,) = sqlx::query_as("SELECT from_id FROM relations WHERE label = 'FUNDS'").fetch_one(&pool).await.unwrap();
        assert_eq!(from, amendment);
        assert_eq!(count("SELECT COUNT(*) FROM entities WHERE key = 'cnpj:55666777000188'").await, 1);

        let line = contract_line(&contract("12345678901", "PF", Value::Null));
        assert!(line.contains("pessoa física") && !line.contains("12345678901"), "CPF must not be written");
    }
}
