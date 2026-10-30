//! State deputies' amendments, from the states' own transparency portals (open data, no key):
//! São Paulo (transparencia.sp.gov.br), Minas Gerais (dados.mg.gov.br, the Segov's Portal de
//! Emendas), Espírito Santo (dados.es.gov.br) and Bahia (dados.ba.gov.br). A target is linked to
//! an author as the state writes the name ("UF:name"); each amendment becomes a document and an
//! entity the person AUTHORED, PAID_TO the recipients known by CNPJ (MG and ES; SP and BA publish
//! names only, kept in the text).
//!
//! Each portal repeats rows its own way:
//! - SP: an amendment's row comes once per pair of its instrument's commitments and settlements,
//!   and the values are the instrument's (a convênio or a contrato de gestão may carry more money
//!   than the amendment). Distinct values are summed, and the text says whose they are.
//! - ES: a row comes once per wording of its PPA action.
//! - MG has one row per amendment (indicação); BA one per budget line, with the payments apart.
//!
//! Goiás publishes too (dadosabertos.go.gov.br), but its CNPJs went through a spreadsheet
//! (7,23812E+13) and its files stopped in July 2025.

use anyhow::{anyhow, bail, Result};
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use super::{brl, bulk_file_named, name_matches, number, scan_csv, utf8, Ctx, Doc, OfficialMatch};

pub const KIND: &str = "emendas_estaduais";
const SP_ZIP: &str = "https://www.transparencia.sp.gov.br/Dados%20Abertos/Emendas/emendas.zip";
const MG_CSV: &str = "https://dados.mg.gov.br/dataset/715a1617-078b-4c07-8657-fe16894b62c2/resource/d85d9a92-dd8e-473d-bcb4-a4f3d13309b1/download/vw_sg_v2_ep_indic_recursos_tw.csv";
/// The dataset page: it lists one file per year, and the CKAN API is robots-disallowed.
const ES_PAGE: &str = "https://dados.es.gov.br/dataset/portal-da-transparencia-emendas-parlamentares-do-estado";
const BA_ZIP: &str = "https://dados.ba.gov.br/dataset/1436b3e7-6594-4683-bfa5-b2e3a6c69e07/resource/2d284f2e-79cc-4e3c-a45b-6fc903a6e2d0/download/emendasparlamentares.zip";
/// Recipients of one amendment linked in the graph (the document lists them all), as for the Union's.
const GRAPH_RECIPIENTS: usize = 30;
/// Payments listed in a Bahia amendment's text.
const LISTED_PAYMENTS: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Sp,
    Mg,
    Es,
    Ba,
}

const STATES: [State; 4] = [State::Sp, State::Mg, State::Es, State::Ba];

impl State {
    fn uf(self) -> &'static str {
        match self {
            State::Sp => "SP",
            State::Mg => "MG",
            State::Es => "ES",
            State::Ba => "BA",
        }
    }

    fn from_uf(uf: &str) -> Option<State> {
        STATES.into_iter().find(|s| s.uf() == uf)
    }

    fn site(self) -> &'static str {
        match self {
            State::Sp => "Portal da Transparência do Estado de São Paulo",
            State::Mg => "Portal de Emendas de Minas Gerais",
            State::Es => "Portal da Transparência do Espírito Santo",
            State::Ba => "Dados Abertos da Bahia",
        }
    }

    /// Where a person finds the amendments (none has a page per amendment).
    fn page(self) -> &'static str {
        match self {
            State::Sp => "https://www.transparencia.sp.gov.br/EmendasParlamentares/Concedidas",
            State::Mg => "https://dados.mg.gov.br/dataset/portal_emendas_estaduais",
            State::Es => ES_PAGE,
            State::Ba => "https://dados.ba.gov.br/dataset/emendas-parlamentares",
        }
    }
}

/// Whose amendments to read: names like the one searched, or one author exactly.
enum Author {
    Like(String),
    Exactly(String),
}

impl Author {
    fn accepts(&self, author: &str) -> bool {
        match self {
            Author::Like(name) => name_matches(name, author),
            Author::Exactly(wanted) => author == wanted,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Amendment {
    pub code: String,
    pub year: String,
    pub author: String,
    pub party: String,
    pub kind: String,
    pub status: String,
    pub body: String,
    pub object: String,
    pub area: String,
    pub places: Vec<String>,
    pub planned: Option<f64>,
    pub committed: f64,
    pub settled: f64,
    pub paid: f64,
    /// The values are their instruments' (SP), not the amendment's alone.
    pub instrument_values: bool,
    pub instruments: Vec<String>,
    pub recipients: Vec<Recipient>,
    /// Paid to people (by CPF or NIS): only how many and how much, never who.
    pub people_paid: (usize, f64),
    /// Bahia's payments: date, payee, amount, purpose.
    pub payments: Vec<String>,
}

#[derive(Debug, Default, Clone)]
pub struct Recipient {
    pub name: String,
    pub cnpj: Option<String>,
    pub amount: Option<f64>,
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !list.iter().any(|v| v == value) {
        list.push(value.to_string());
    }
}

/// A CNPJ as the states write it: "18.278.051/0001-45", "18278051000145", or a spreadsheet's
/// number that lost its leading zeros ("1614334000118,0"). CPFs and NIS are not CNPJs.
fn cnpj(text: &str) -> Option<String> {
    let text = text.trim();
    let number = text.strip_suffix(",0").or_else(|| text.strip_suffix(".0")).unwrap_or(text);
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    match digits.len() {
        14 => Some(digits),
        12 | 13 if number.chars().all(|c| c.is_ascii_digit()) => Some(format!("{digits:0>14}")),
        _ => None,
    }
}

/// "Dep. Raquel Lessa" → "Raquel Lessa": the name as the person is known.
fn display(author: &str) -> String {
    let author = author.trim();
    ["Dep. ", "Deputado ", "Deputada "].iter().find_map(|p| author.strip_prefix(p)).unwrap_or(author).trim().to_string()
}

// ===== São Paulo =====

/// emenda.csv: orgao_executor;origem_recurso;cod_emenda;nome_parlamentar;partido;
/// num_instrumento_juridico;nome_beneficiario;objeto;tipo_emenda;nome_funcao;municipio_gasto;
/// vl_empenhado;vl_liquidado;vl_pago. A ';' in the object is not quoted: the first seven and the
/// last six fields are certain, the object is what lies between.
fn read_sp(archive: &Path, who: &Author) -> Result<Vec<Amendment>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let file = zip.by_index(0)?;
    let mut reader = csv::ReaderBuilder::new().delimiter(b';').flexible(true).quoting(false).from_reader(BufReader::new(file));
    let mut by_code: BTreeMap<String, Amendment> = BTreeMap::new();
    // (code, instrument) → the distinct commitments, settlements and payments seen
    let mut values: BTreeMap<(String, String), [BTreeSet<String>; 3]> = BTreeMap::new();
    let mut record = csv::ByteRecord::new();
    while reader.read_byte_record(&mut record)? {
        let n = record.len();
        // A line break inside an object cuts its row short: the pieces are dropped
        if n < 14 {
            continue;
        }
        let field = |i: usize| utf8(&record[i]).trim().to_string();
        let author = field(3);
        if !who.accepts(&author) {
            continue;
        }
        let code = field(2);
        let a = by_code.entry(code.clone()).or_insert_with(|| Amendment {
            year: code.get(..4).unwrap_or("").to_string(),
            code: code.clone(),
            author,
            party: field(4),
            kind: field(1),
            body: field(0),
            object: (7..n - 6).map(|i| utf8(&record[i])).collect::<Vec<_>>().join(";").trim().to_string(),
            instrument_values: true,
            ..Default::default()
        });
        push_unique(&mut a.places, &field(n - 4));
        let beneficiary = field(6);
        if !beneficiary.is_empty() && !a.recipients.iter().any(|r| r.name == beneficiary) {
            a.recipients.push(Recipient { name: beneficiary, ..Default::default() });
        }
        let instrument = field(5);
        if instrument.is_empty() {
            continue;
        }
        push_unique(&mut a.instruments, &instrument);
        let seen = values.entry((code, instrument)).or_default();
        for (set, i) in seen.iter_mut().zip([n - 3, n - 2, n - 1]) {
            let value = field(i);
            if !value.is_empty() {
                set.insert(value);
            }
        }
    }
    // ponytail: two genuine payments of the same amount count once; the repeated pairs are far more common
    for ((code, _), [committed, settled, paid]) in values {
        let a = by_code.get_mut(&code).expect("seen above");
        a.committed += committed.iter().map(|v| number(v)).sum::<f64>();
        a.settled += settled.iter().map(|v| number(v)).sum::<f64>();
        a.paid += paid.iter().map(|v| number(v)).sum::<f64>();
    }
    Ok(by_code.into_values().collect())
}

// ===== Minas Gerais =====

fn read_mg(path: &Path, who: &Author) -> Result<Vec<Amendment>> {
    let mut found = Vec::new();
    scan_csv(BufReader::new(std::fs::File::open(path)?), utf8, |row| {
        let author = row.get("responsavel");
        if !who.accepts(author) {
            return true;
        }
        // The Segov's own figure, or the accounting warehouse's when the Segov has none yet (this
        // year's payments). When both are there they mostly agree; the warehouse's larger ones
        // are commitments shared by several amendments.
        let value = |stage: &str| Some(number(row.get(&format!("valor_{stage}")))).filter(|v| *v != 0.0).unwrap_or_else(|| number(row.get(&format!("valor_{stage}_armazem"))));
        let paid = value("pago");
        let impositive = if row.get("indicador_impositividade") == "S" { "Impositiva" } else { "Não impositiva" };
        let mut a = Amendment {
            code: row.get("numero_indicacao").into(),
            year: row.get("ano_exercicio").into(),
            author: author.into(),
            kind: [impositive, row.get("tipo_indicacao")].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · "),
            status: [row.get("status_indicacao"), row.get("status_transparencia_descricao")].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · "),
            body: row.get("uo_descricao").into(),
            object: row.get("descricao_indicacao").into(),
            area: [row.get("funcao_descricao"), row.get("acao_nome")].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" / "),
            planned: Some(number(row.get("valor_indicacao"))),
            committed: value("empenhado"),
            settled: value("liquidado"),
            paid,
            ..Default::default()
        };
        push_unique(&mut a.places, row.get("municipio"));
        let instrument = [row.get("instrumento_numero"), row.get("instrumento_titulo")].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" – ");
        push_unique(&mut a.instruments, &instrument);
        let name = row.get("beneficiario_nome");
        if !name.is_empty() {
            a.recipients.push(Recipient { name: name.into(), cnpj: cnpj(row.get("beneficiario_cnpj")), amount: Some(paid) });
        }
        found.push(a);
        true
    })?;
    Ok(found)
}

// ===== Espírito Santo =====

/// The yearly files linked from the dataset page (the execution, contract and agreement files
/// repeat what they sum).
fn es_years(html: &str) -> Vec<(String, String)> {
    let page = scraper::Html::parse_document(html);
    let links = scraper::Selector::parse("a[href]").expect("selector");
    let mut years = BTreeMap::new();
    for href in page.select(&links).filter_map(|a| a.value().attr("href")) {
        let Some(year) = href.rsplit_once("/download/emendas-estaduais-").and_then(|(_, rest)| rest.strip_suffix(".csv")) else { continue };
        if year.len() == 4 && year.chars().all(|c| c.is_ascii_digit()) {
            years.insert(year.to_string(), href.to_string());
        }
    }
    years.into_iter().collect()
}

/// What tells two rows apart besides the PPA action's wording (and the row id).
const ES_ROW: [&str; 16] = [
    "NumeroEmenda", "CpfCnpjNis", "NumeroInstrumento", "CodigoConvenio", "CodigoContrato", "ValorEmpenho", "ValorLiquidado", "ValorPago", "ValorRap", "AcaoPPA",
    "CodigoModalidade", "IdtElemento", "CodigoGrupoDespesa", "CodigoFuncao", "CodigoSubFuncao", "CodigoMunicipio",
];

fn read_es(paths: &[PathBuf], who: &Author) -> Result<Vec<Amendment>> {
    let mut by_code: BTreeMap<String, Amendment> = BTreeMap::new();
    let mut seen = HashSet::new();
    // (code, CNPJ) → recipient; code → CPFs/NIS, only to count them
    let mut recipients: BTreeMap<(String, String), Recipient> = BTreeMap::new();
    let mut people: HashMap<String, HashSet<String>> = HashMap::new();
    for path in paths {
        scan_csv(BufReader::new(std::fs::File::open(path)?), utf8, |row| {
            let author = row.get("NomeAutor");
            if !who.accepts(author) || !seen.insert(ES_ROW.map(|c| row.get(c).to_string())) {
                return true;
            }
            let code = row.get("NumeroEmenda").to_string();
            let a = by_code.entry(code.clone()).or_insert_with(|| Amendment {
                code: code.clone(),
                year: row.get("AnoEmenda").into(),
                author: author.into(),
                kind: row.get("TipoEmenda").into(),
                status: row.get("NomeElemento").into(),
                body: row.get("DescricaoOrgaoExecutor").into(),
                object: row.get("ObjetoFinalidade").into(),
                area: [row.get("Funcao"), row.get("SubFuncao")].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" / "),
                planned: Some(number(row.get("ValorPrevisto"))),
                ..Default::default()
            });
            push_unique(&mut a.places, row.get("DescricaoRegiaoBeneficiada"));
            push_unique(&mut a.instruments, row.get("NumeroInstrumento"));
            let paid = number(row.get("ValorPago"));
            a.committed += number(row.get("ValorEmpenho"));
            a.settled += number(row.get("ValorLiquidado"));
            a.paid += paid;
            let payee = row.get("CpfCnpjNis");
            match cnpj(payee) {
                Some(id) => {
                    let r = recipients.entry((code, id.clone())).or_insert_with(|| Recipient { name: row.get("Favorecido").into(), cnpj: Some(id), amount: Some(0.0) });
                    *r.amount.get_or_insert(0.0) += paid;
                }
                None if !payee.is_empty() => {
                    a.people_paid.1 += paid;
                    people.entry(code).or_default().insert(payee.to_string());
                }
                None => {}
            }
            true
        })?;
    }
    for ((code, _), r) in recipients {
        by_code.get_mut(&code).expect("seen above").recipients.push(r);
    }
    for (code, set) in people {
        by_code.get_mut(&code).expect("seen above").people_paid.0 = set.len();
    }
    for a in by_code.values_mut() {
        a.recipients.sort_by(|x, y| y.amount.unwrap_or(0.0).total_cmp(&x.amount.unwrap_or(0.0)));
    }
    Ok(by_code.into_values().collect())
}

// ===== Bahia =====

const BA_LINES: &str = "VW_PAINEL_EMENDAS_PARLAMENTARES_DESPESAS.csv";
const BA_EXECUTION: &str = "VW_PAINEL_EMENDAS_PARLAMENTARES_CENTRALIZACAO_DESCENTRALIZACAO.csv";
const BA_PAYMENTS: &str = "VW_PAINEL_EMENDAS_PARLAMENTARES_PAGAMENTOS.csv";

fn scan_zip_utf8(archive: &Path, name: &str, f: impl FnMut(&super::Row) -> bool) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let file = zip.by_name(name).map_err(|e| anyhow!("{name} is not in {}: {e}", archive.display()))?;
    scan_csv(BufReader::new(file), utf8, f)
}

/// Each budget line a deputy's amendment opened (by action), and the payments made from it, which
/// may run under another code (a budget moved to another body).
fn read_ba(archive: &Path, who: &Author) -> Result<Vec<Amendment>> {
    let mut by_code: BTreeMap<String, Amendment> = BTreeMap::new();
    scan_zip_utf8(archive, BA_LINES, |row| {
        let author = row.get("Nome do Deputado");
        if who.accepts(author) {
            let code = row.get("num_codigo").to_string();
            by_code.insert(
                code.clone(),
                Amendment {
                    code,
                    year: row.get("Ano Exercício").into(),
                    author: author.into(),
                    kind: "Impositiva".into(),
                    body: row.get("Órgão").into(),
                    area: row.get("Ação do Programa de Governo").into(),
                    planned: Some(number(row.get("Valor Orçado Atual."))),
                    committed: number(row.get("Valor Empenhado.")),
                    settled: number(row.get("Valor Liquidado.")),
                    paid: number(row.get("Valor Pago.")),
                    ..Default::default()
                },
            );
        }
        true
    })?;
    if by_code.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines: HashMap<String, String> = by_code.keys().map(|c| (c.clone(), c.clone())).collect();
    scan_zip_utf8(archive, BA_EXECUTION, |row| {
        let code = row.get("num_codigo");
        if by_code.contains_key(code) {
            lines.insert(row.get("num_codigo_exec").to_string(), code.to_string());
        }
        true
    })?;
    let mut paid_to: BTreeMap<(String, String), f64> = BTreeMap::new();
    scan_zip_utf8(archive, BA_PAYMENTS, |row| {
        let Some(code) = lines.get(row.get("num_codigo_exec")) else { return true };
        if row.get("Pagamento_Efetivado") != "Sim" {
            return true;
        }
        let (payee, amount) = (row.get("RazaoSocialCredorPagamento"), number(row.get("val_pagto_nob")));
        *paid_to.entry((code.clone(), payee.to_string())).or_default() += amount;
        let a = by_code.get_mut(code).expect("lines point at amendments");
        let date = row.get("Data do Pagamento").get(..10).unwrap_or("");
        a.payments.push(format!("{date} · {payee}: {} · {}", brl(amount), row.get("Objeto")));
        true
    })?;
    for ((code, name), amount) in paid_to {
        by_code.get_mut(&code).expect("seen above").recipients.push(Recipient { name, cnpj: None, amount: Some(amount) });
    }
    for a in by_code.values_mut() {
        a.recipients.sort_by(|x, y| y.amount.unwrap_or(0.0).total_cmp(&x.amount.unwrap_or(0.0)));
    }
    Ok(by_code.into_values().collect())
}

// ===== Reading, finding, collecting =====

async fn files(pool: &SqlitePool, app_data_dir: &Path, state: State) -> Result<Vec<PathBuf>> {
    Ok(match state {
        State::Sp => vec![bulk_file_named(pool, app_data_dir, SP_ZIP, "sp-emendas.zip", true).await?],
        State::Mg => vec![bulk_file_named(pool, app_data_dir, MG_CSV, "mg-emendas-indicacoes.csv", true).await?],
        State::Ba => vec![bulk_file_named(pool, app_data_dir, BA_ZIP, "ba-emendas.zip", true).await?],
        State::Es => {
            let page = bulk_file_named(pool, app_data_dir, ES_PAGE, "es-emendas.html", true).await?;
            let mut files = Vec::new();
            for (year, url) in es_years(&std::fs::read_to_string(page)?) {
                files.push(bulk_file_named(pool, app_data_dir, &url, &format!("es-emendas-{year}.csv"), true).await?);
            }
            if files.is_empty() {
                bail!("No yearly amendment files on {ES_PAGE}");
            }
            files
        }
    })
}

async fn amendments(pool: &SqlitePool, app_data_dir: &Path, state: State, who: Author) -> Result<Vec<Amendment>> {
    let files = files(pool, app_data_dir, state).await?;
    tokio::task::spawn_blocking(move || match state {
        State::Sp => read_sp(&files[0], &who),
        State::Mg => read_mg(&files[0], &who),
        State::Es => read_es(&files, &who),
        State::Ba => read_ba(&files[0], &who),
    })
    .await?
}

/// Authors whose name matches, in every state, with how many amendments and which years. The
/// states are different hosts: they are read at the same time.
pub async fn find(pool: &SqlitePool, app_data_dir: &Path, name: &str) -> Vec<OfficialMatch> {
    let lists = futures_util::future::join_all(STATES.map(|state| amendments(pool, app_data_dir, state, Author::Like(name.to_string())))).await;
    let mut found = Vec::new();
    for (state, list) in STATES.into_iter().zip(lists) {
        let list = match list {
            Ok(list) => list,
            Err(e) => {
                log::warn!("Official: {} amendments search failed: {e}", state.uf());
                continue;
            }
        };
        // author → (count, first year, last year, party of the latest)
        let mut authors: BTreeMap<String, (usize, String, String, String)> = BTreeMap::new();
        for a in list {
            let entry = authors.entry(a.author.clone()).or_insert_with(|| (0, a.year.clone(), a.year.clone(), String::new()));
            entry.0 += 1;
            entry.1 = entry.1.clone().min(a.year.clone());
            if a.year >= entry.2 {
                entry.2 = a.year.clone();
                if !a.party.is_empty() {
                    entry.3 = a.party.clone();
                }
            }
        }
        found.extend(authors.into_iter().map(|(author, (count, first, last, party))| OfficialMatch {
            kind: KIND.into(),
            id: format!("{}:{author}", state.uf()),
            label: display(&author),
            detail: format!(
                "{} · {count} emendas estaduais ({first}–{last}){}",
                state.site(),
                if party.is_empty() { String::new() } else { format!(" · {party}") }
            ),
            similar: false,
        }));
    }
    found
}

pub fn text(uf: &str, a: &Amendment) -> String {
    let author = display(&a.author);
    let mut lines = vec![format!("Emenda parlamentar estadual {} de {author} ({uf}, {}).", a.code, a.year)];
    for (label, value) in [("Partido", &a.party), ("Tipo", &a.kind), ("Situação", &a.status), ("Órgão executor", &a.body), ("Objeto", &a.object), ("Área", &a.area)] {
        if !value.is_empty() {
            lines.push(format!("{label}: {value}."));
        }
    }
    if !a.places.is_empty() {
        lines.push(format!("Onde: {}.", a.places.join("; ")));
    }
    if let Some(planned) = a.planned {
        lines.push(format!("Valor indicado: {}.", brl(planned)));
    }
    let values = format!("empenhado {}, liquidado {}, pago {}", brl(a.committed), brl(a.settled), brl(a.paid));
    match (a.instrument_values, a.instruments.is_empty()) {
        (true, true) => lines.push("Ainda sem instrumento jurídico nem valores registrados.".into()),
        (true, false) => lines.push(format!(
            "Valores registrados no instrumento jurídico nº {}, que pode incluir outros recursos além desta emenda: {values}.",
            a.instruments.join(", nº ")
        )),
        (false, _) => {
            lines.push(format!("Valores: {values}."));
            if !a.instruments.is_empty() {
                lines.push(format!("Instrumentos: {}.", a.instruments.join("; ")));
            }
        }
    }
    let mut text = lines.join("\n");
    if !a.recipients.is_empty() || a.people_paid.0 > 0 {
        text.push_str(if a.instrument_values { "\n\nBeneficiário:" } else { "\n\nQuem recebeu:" });
        for r in &a.recipients {
            let id = r.cnpj.as_ref().map(|c| format!(" (CNPJ {c})")).unwrap_or_default();
            let amount = r.amount.map(|v| format!(": {}", brl(v))).unwrap_or_default();
            text.push_str(&format!("\n- {}{id}{amount}", r.name));
        }
        if a.people_paid.0 > 0 {
            text.push_str(&format!("\n- Pessoas físicas ({}): {} no total", a.people_paid.0, brl(a.people_paid.1)));
        }
    }
    if !a.payments.is_empty() {
        text.push_str("\n\nPagamentos:");
        for payment in a.payments.iter().take(LISTED_PAYMENTS) {
            text.push_str(&format!("\n- {payment}"));
        }
        if a.payments.len() > LISTED_PAYMENTS {
            text.push_str(&format!("\n- e mais {} pagamentos", a.payments.len() - LISTED_PAYMENTS));
        }
    }
    text
}

fn title(uf: &str, a: &Amendment) -> String {
    let what = if a.object.is_empty() { &a.area } else { &a.object };
    let what: String = what.chars().take(100).collect();
    let place = a.places.first().map(|p| format!(" – {p}")).unwrap_or_default();
    // SP's values are the instrument's: the title would credit the amendment with them
    let paid = if a.instrument_values { String::new() } else { format!(": {} pagos", brl(a.paid)) };
    format!("Emenda estadual {} ({uf}, {}) – {what}{place}{paid}", a.code, a.year)
}

pub async fn run(ctx: &mut Ctx<'_>) -> Result<()> {
    let (uf, author) = ctx.source.url.split_once(':').ok_or_else(|| anyhow!("Not a state amendments source: {}", ctx.source.url))?;
    let state = State::from_uf(uf).ok_or_else(|| anyhow!("No amendments from state {uf}"))?;
    let list = amendments(ctx.pool, ctx.app_data_dir, state, Author::Exactly(author.to_string())).await?;
    let person = ctx.person.clone();
    for a in list {
        let anchor: String = a.code.chars().filter(|c| !c.is_whitespace()).collect();
        let url = format!("{}#emenda-{anchor}", state.page());
        let date = format!("{}-01-01", a.year);
        let id = ctx
            .document(Doc {
                url: &url,
                kind: "amendment",
                title: &title(uf, &a),
                author: Some(&display(&a.author)),
                site: Some(state.site()),
                published_at: Some(&date),
                text: &text(uf, &a),
                note: None,
            })
            .await?;
        let mut attrs = json!({"uf": uf, "year": a.year, "area": a.area, "places": a.places});
        if !a.instrument_values {
            attrs["committed"] = json!(a.committed);
            attrs["paid"] = json!(a.paid);
        }
        let entity = ctx.entity("amendment", &format!("emenda:{}:{}", uf.to_lowercase(), a.code), &format!("Emenda {} ({uf})", a.code), attrs, Some(&id)).await?;
        ctx.relate(&person, &entity, "AUTHORED", &date, json!({}), Some(&id)).await?;
        for r in a.recipients.iter().filter(|r| r.cnpj.is_some()).take(GRAPH_RECIPIENTS) {
            let cnpj = r.cnpj.as_deref().expect("filtered");
            let org = ctx.entity("organization", &format!("cnpj:{cnpj}"), &r.name, json!({"cnpj": cnpj}), None).await?;
            let amount = (r.amount.unwrap_or(0.0) * 100.0).round() / 100.0;
            ctx.relate(&entity, &org, "PAID_TO", &date, json!({"amount": amount}), Some(&id)).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("voga-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zipped(path: &Path, files: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, text) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn cnpjs_and_names_as_the_states_write_them() {
        assert_eq!(cnpj("18278051000145,0").as_deref(), Some("18278051000145"));
        assert_eq!(cnpj("1614334000118,0").as_deref(), Some("01614334000118"));
        assert_eq!(cnpj("18.278.051/0001-45").as_deref(), Some("18278051000145"));
        assert_eq!(cnpj("12345678901"), None, "a CPF");
        assert_eq!(cnpj(""), None);
        assert_eq!(display("Dep. Marina Exemplo"), "Marina Exemplo");
        assert_eq!(display("  Bruno Ficticio"), "Bruno Ficticio");
    }

    #[test]
    fn sao_paulo_counts_repeated_pairs_once_and_says_whose_values_they_are() {
        let dir = dir("sp");
        let archive = dir.join("sp.zip");
        let header = "orgao_executor;origem_recurso;cod_emenda;nome_parlamentar;partido;num_instrumento_juridico;nome_beneficiario;objeto;tipo_emenda;nome_funcao;municipio_gasto;vl_empenhado;vl_liquidado;vl_pago";
        let row = |emp: &str, liq: &str| format!("SECRETARIA DE ESPORTES;Emenda LOA;2024.084.1;Marina Exemplo;PSB;20240001;PREFEITURA MUNICIPAL DE CIDADE EXEMPLO;REFORMA; QUADRA E VESTIÁRIO;Emenda LOA;;CIDADE EXEMPLO;{emp};{liq};{emp}");
        let csv = [
            header.to_string(),
            row("100.00", "100.00"),
            row("100.00", "250.00"),
            row("250.00", "100.00"),
            row("250.00", "250.00"),
            row("250.00", "250.00"),
            "SECRETARIA DE SAÚDE;Transferência Voluntária;2025.084.2;Marina Exemplo;PSB;;HOSPITAL EXEMPLO;EQUIPAMENTOS;Transferência Voluntária;;OUTRA CIDADE;;;".into(),
            "SECRETARIA DE SAÚDE;Emenda LOA;2025.099.3;Outro Nome;PL;20250009;X;Y;Emenda LOA;;Z;1.00;1.00;1.00".into(),
            "SECRETARIA;Emenda LOA;2025.084.4;Marina Exemplo;PSB;;CORTADO".into(),
        ]
        .join("\r\n");
        zipped(&archive, &[("emenda.csv", &csv)]);
        let found = read_sp(&archive, &Author::Exactly("Marina Exemplo".into())).unwrap();
        assert_eq!(found.len(), 2);
        let a = &found[0];
        assert_eq!(a.object, "REFORMA; QUADRA E VESTIÁRIO");
        assert_eq!((a.committed, a.settled, a.paid), (350.0, 350.0, 350.0));
        assert_eq!(a.places, ["CIDADE EXEMPLO"]);
        let text = text("SP", a);
        assert!(text.contains("instrumento jurídico nº 20240001, que pode incluir outros recursos") && text.contains("Partido: PSB."), "{text}");
        assert!(!title("SP", a).contains("pagos"));
        assert!(super::text("SP", &found[1]).contains("Ainda sem instrumento"));
        let like = read_sp(&archive, &Author::Like("Marina Exemplo".into())).unwrap();
        assert_eq!(like.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn minas_gerais_one_row_per_amendment_with_its_recipient() {
        let dir = dir("mg");
        let path = dir.join("mg.csv");
        std::fs::write(
            &path,
            "\u{feff}numero_indicacao;ano_exercicio;indicador_impositividade;uo_descricao;responsavel;tipo_indicacao;status_indicacao;municipio;beneficiario_nome;beneficiario_cnpj;acao_nome;funcao_descricao;descricao_indicacao;valor_indicacao;valor_empenhado;valor_liquidado;valor_pago;instrumento_numero;status_transparencia_descricao;valor_empenhado_armazem;valor_liquidado_armazem;valor_pago_armazem\n\
             112908;2023;S;FUNDO ESTADUAL DE SAÚDE;MARINA EXEMPLO;RESOLUÇÃO SES;APROVADO;CIDADE EXEMPLO;FUNDO MUNICIPAL DE SAUDE;1614334000118,0;REGULAÇÃO DO ACESSO;SAÚDE;AMBULÂNCIA;243822,0;243822,0;243822,0;243822,0;008719/2023;PAGO TOTAL;468000,0;0,0;0,0\n\
             196929;2026;S;FUNDO ESTADUAL DE SAÚDE;MARINA EXEMPLO;RESOLUÇÃO SES;APROVADO;CIDADE EXEMPLO;FUNDO MUNICIPAL DE SAUDE;1614334000118,0;CUSTEIO;SAÚDE;;180000,0;180000,0;180000,0;0,0;011053/2026;PAGO TOTAL;180000,0;180000,0;180000,0\n\
             112909;2023;S;X;OUTRO NOME;X;APROVADO;X;X;;X;X;X;1,0;0,0;0,0;0,0;;;;;\n",
        )
        .unwrap();
        let found = read_mg(&path, &Author::Like("Marina Exemplo".into())).unwrap();
        assert_eq!(found.len(), 2);
        let a = &found[0];
        assert_eq!(a.recipients[0].cnpj.as_deref(), Some("01614334000118"));
        assert_eq!((a.planned, a.committed, a.paid), (Some(243822.0), 243822.0, 243822.0), "the Segov's own figures first");
        assert_eq!(found[1].paid, 180000.0, "the warehouse's when the Segov has none");
        let text = text("MG", a);
        assert!(text.contains("Situação: APROVADO · PAGO TOTAL.") && text.contains("FUNDO MUNICIPAL DE SAUDE (CNPJ 01614334000118): R$ 243.822,00"), "{text}");
        assert!(title("MG", a).ends_with("R$ 243.822,00 pagos"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn espirito_santo_drops_rows_repeated_for_each_ppa_wording() {
        let dir = dir("es");
        let header = "CodAutor;NomeAutor;AnoEmenda;NumeroEmenda;ObjetoFinalidade;AcaoPPA;DescricaoAcaoPPA;DescricaoRegiaoBeneficiada;DescricaoOrgaoExecutor;ValorPrevisto;NomeElemento;TipoEmenda;NumeroInstrumento;CpfCnpjNis;Favorecido;ValorEmpenho;ValorLiquidado;ValorPago;Funcao;SubFuncao;Id";
        let row = |ppa: &str, payee: &str, name: &str, paid: &str, id: u32| {
            format!("16;Dep. Marina Exemplo;2025;2025 / E0275;MÁQUINAS AGRÍCOLAS;1060;{ppa};CAPARAÓ;\"SECRETARIA DA AGRICULTURA \";50900,00;SANÇÃO DO GOVERNADOR;Individual;TERMO 1/2025;{payee};{name};{paid};{paid};{paid};AGRICULTURA;PROMOÇÃO;{id}")
        };
        let csv = [
            header.to_string(),
            row("APOIO À INFRAESTRUTURA", "49393549000182", "EMPRESA EXEMPLO LTDA", "30000,0000", 1),
            row("PPA DE RAP - APOIO À INFRAESTRUTURA", "49393549000182", "EMPRESA EXEMPLO LTDA", "30000,0000", 2),
            row("APOIO À INFRAESTRUTURA", "12345678901", "FULANO FICTICIO", "500,0000", 3),
        ]
        .join("\r\n");
        let path = dir.join("es-2025.csv");
        std::fs::write(&path, format!("\u{feff}{csv}")).unwrap();
        let found = read_es(&[path], &Author::Like("Marina Exemplo".into())).unwrap();
        assert_eq!(found.len(), 1);
        let a = &found[0];
        assert_eq!(a.paid, 30500.0);
        assert_eq!(a.recipients.len(), 1);
        assert_eq!(a.recipients[0].amount, Some(30000.0));
        assert_eq!(a.people_paid, (1, 500.0));
        let text = text("ES", a);
        assert!(text.contains("de Marina Exemplo (ES, 2025)") && text.contains("Pessoas físicas (1): R$ 500,00"), "{text}");
        assert!(!text.contains("FULANO") && !text.contains("12345678901"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_yearly_files_are_found_on_the_dataset_page() {
        let html = r#"<ul><li><a href="https://dados.es.gov.br/dataset/d1/resource/r1/download/emendas-estaduais-2025.csv">Baixar</a></li>
            <li><a href="https://dados.es.gov.br/dataset/d1/resource/r2/download/emendas-estaduais-execucao-2025.csv">Baixar</a></li>
            <li><a href="https://dados.es.gov.br/dataset/d1/resource/r3/download/emendas-estaduais-convenios.csv">Baixar</a></li>
            <li><a href="https://dados.es.gov.br/dataset/d1/resource/r4/download/emendas-estaduais-2021.csv">Baixar</a></li></ul>"#;
        let years = es_years(html);
        assert_eq!(years.iter().map(|(y, _)| y.as_str()).collect::<Vec<_>>(), ["2021", "2025"]);
        assert!(years[1].1.ends_with("/r1/download/emendas-estaduais-2025.csv"));
    }

    #[test]
    fn bahia_joins_the_payments_made_under_another_code() {
        let dir = dir("ba");
        let archive = dir.join("ba.zip");
        zipped(
            &archive,
            &[
                (
                    BA_LINES,
                    "\u{feff}\"Ano Exercício\";\"Órgão\";\"Ação do Programa de Governo\";\"Deputado\";\"Nome do Deputado\";\"num_codigo\";\"Valor Orçado Atual.\";\"Valor Empenhado.\";\"Valor Liquidado.\";\"Valor Pago.\"\n\
                     \"2025\";\"Secretaria da Educação\";\"Aparelhamento de Unidade\";\"  Marina Exemplo\";\"  Marina Exemplo\";\"2025.3.11.1\";\"67000,00\";\"45343,19\";\"15637,29\";\"15637,29\"\n\
                     \"2025\";\"Secretaria da Saúde\";\"Outra\";\"Outro Nome\";\"Outro Nome\";\"2025.3.19.9\";\"1,00\";\"1,00\";\"1,00\";\"1,00\"\n",
                ),
                (BA_EXECUTION, "\u{feff}\"num_codigo\";\"num_codigo_exec\";\"num_codigo_liqu\"\n\"2025.3.11.1\";\"2025.3.22.5\";\"x\"\n"),
                (
                    BA_PAYMENTS,
                    "\u{feff}\"RazaoSocialCredorPagamento\";\"Data do Pagamento\";\"val_pagto_nob\";\"Pagamento_Efetivado\";\"Objeto\";\"num_codigo_exec\"\n\
                     \"INSTITUTO EXEMPLO\";\"13/06/2025 00:00:00\";\"10000,00\";\"Sim\";\"APOIO AO PROJETO\";\"2025.3.22.5\"\n\
                     \"INSTITUTO EXEMPLO\";\"14/06/2025 00:00:00\";\"5637,29\";\"Sim\";\"SEGUNDA PARCELA\";\"2025.3.11.1\"\n\
                     \"OUTRO CREDOR\";\"15/06/2025 00:00:00\";\"999,00\";\"Não\";\"CANCELADO\";\"2025.3.11.1\"\n",
                ),
            ],
        );
        let found = read_ba(&archive, &Author::Exactly("Marina Exemplo".into())).unwrap();
        assert_eq!(found.len(), 1);
        let a = &found[0];
        assert_eq!(a.recipients.len(), 1);
        assert_eq!((a.recipients[0].name.as_str(), a.recipients[0].amount), ("INSTITUTO EXEMPLO", Some(15637.29)));
        assert_eq!(a.payments.len(), 2);
        let text = text("BA", a);
        assert!(text.contains("13/06/2025 · INSTITUTO EXEMPLO: R$ 10.000,00 · APOIO AO PROJETO") && !text.contains("CANCELADO"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
