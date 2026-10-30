//! Câmara dos Deputados (dadosabertos.camara.leg.br): a deputy's profile, speeches (with their
//! transcription), authored bills, nominal votes and parliamentary quota expenses (CEAP).
//! Votes and expenses come from the yearly dumps, one download for every deputy followed.

use anyhow::Result;
use chrono::{Datelike, Utc};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};

use super::{brl, bulk_file, name_matches, number, Ctx, Doc, OfficialMatch, Table, HISTORY_START};
use crate::collect::fetch;

const API: &str = "https://dadosabertos.camara.leg.br/api/v2";
const FILES: &str = "https://dadosabertos.camara.leg.br/arquivos";
const SITE: &str = "Câmara dos Deputados";
const BILL_TYPES: [&str; 5] = ["PL", "PLP", "PEC", "PDL", "PRC"];
/// Pages of 100 per stream and run: a long history comes over several runs.
const PAGES_PER_RUN: u32 = 5;

#[derive(Deserialize)]
struct Envelope<T> {
    dados: T,
}

fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// dd/mm/yyyy from an ISO date or date-time.
fn br_date(iso: &str) -> String {
    match (iso.get(..4), iso.get(5..7), iso.get(8..10)) {
        (Some(y), Some(m), Some(d)) => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedDeputy {
    id: i64,
    nome: String,
    sigla_partido: Option<String>,
    sigla_uf: Option<String>,
    id_legislatura: Option<i64>,
}

pub async fn find(name: &str) -> Result<Vec<OfficialMatch>> {
    let mut found: Vec<OfficialMatch> = Vec::new();
    // The API matches part of the parliamentary name: try the full name, then the last word
    let last = name.split_whitespace().last().unwrap_or(name);
    for query in [name, last] {
        let url = format!("{API}/deputados?nome={}&idLegislatura=57&idLegislatura=56&itens=50", encode(query));
        let listed: Envelope<Vec<ListedDeputy>> = fetch::get_json(&url).await?;
        // Listed once per legislature: the latest one tells the current party
        let mut listed = listed.dados;
        listed.sort_by_key(|d| std::cmp::Reverse(d.id_legislatura));
        for deputy in listed {
            let id = deputy.id.to_string();
            if found.iter().any(|m| m.id == id) || (query == last && !name_matches(name, &deputy.nome)) {
                continue;
            }
            found.push(OfficialMatch {
                kind: "camara_deputado".into(),
                id,
                label: deputy.nome,
                detail: format!(
                    "Câmara · {}-{} · legislatura {}",
                    deputy.sigla_partido.unwrap_or_default(),
                    deputy.sigla_uf.unwrap_or_default(),
                    deputy.id_legislatura.unwrap_or_default()
                ),
                similar: false,
            });
        }
        if !found.is_empty() {
            break;
        }
    }
    Ok(found)
}

pub async fn run(ctx: &mut Ctx<'_>) -> Result<()> {
    let id = ctx.source.url.clone();
    profile(ctx, &id).await?;
    speeches(ctx, &id).await?;
    bills(ctx, &id).await?;
    votes(ctx, &id).await?;
    expenses(ctx, &id).await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Deputy {
    nome_civil: Option<String>,
    url_website: Option<String>,
    rede_social: Option<Vec<String>>,
    escolaridade: Option<String>,
    ultimo_status: Status,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    nome: String,
    sigla_partido: Option<String>,
    sigla_uf: Option<String>,
    situacao: Option<String>,
    condicao_eleitoral: Option<String>,
    email: Option<String>,
    id_legislatura: Option<i64>,
}

async fn profile(ctx: &mut Ctx<'_>, id: &str) -> Result<()> {
    let deputy: Envelope<Deputy> = fetch::get_json(&format!("{API}/deputados/{id}")).await?;
    let (deputy, status) = (&deputy.dados, &deputy.dados.ultimo_status);
    let party = status.sigla_partido.clone().unwrap_or_default();
    let uf = status.sigla_uf.clone().unwrap_or_default();
    let mut text = format!(
        "{} ({party}-{uf}), deputado(a) federal na {}ª legislatura. Situação: {}. Condição eleitoral: {}.",
        status.nome,
        status.id_legislatura.unwrap_or_default(),
        status.situacao.as_deref().unwrap_or("-"),
        status.condicao_eleitoral.as_deref().unwrap_or("-"),
    );
    if let Some(civil) = &deputy.nome_civil {
        text.push_str(&format!("\nNome civil: {civil}."));
    }
    if let Some(schooling) = &deputy.escolaridade {
        text.push_str(&format!("\nEscolaridade: {schooling}."));
    }
    if let Some(email) = &status.email {
        text.push_str(&format!("\nE-mail do gabinete: {email}."));
    }
    let links: Vec<&str> = deputy.url_website.iter().chain(deputy.rede_social.iter().flatten()).map(String::as_str).collect();
    if !links.is_empty() {
        text.push_str(&format!("\nSites e redes: {}.", links.join(", ")));
    }
    let url = format!("https://www.camara.leg.br/deputados/{id}");
    let title = format!("Perfil de {} na Câmara", status.nome);
    let doc = ctx
        .document(Doc {
            url: &url,
            kind: "profile",
            title: &title,
            author: None,
            site: Some(SITE),
            published_at: None,
            text: &text,
            note: None,
        })
        .await?;
    ctx.describe_person(json!({"camara_id": id, "uf": uf}), &doc).await?;
    ctx.member_of(&party, Some(&doc)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Speech {
    data_hora_inicio: String,
    tipo_discurso: Option<String>,
    sumario: Option<String>,
    transcricao: Option<String>,
}

async fn speeches(ctx: &mut Ctx<'_>, id: &str) -> Result<()> {
    let from = ctx.cursor("speeches").await?.unwrap_or_else(|| HISTORY_START.to_string());
    let mut latest = from.clone();
    for page in 1..=PAGES_PER_RUN {
        let url = format!("{API}/deputados/{id}/discursos?dataInicio={}&ordenarPor=dataHoraInicio&ordem=ASC&itens=100&pagina={page}", &from[..10.min(from.len())]);
        let listed: Envelope<Vec<Speech>> = fetch::get_json(&url).await?;
        let count = listed.dados.len();
        for speech in listed.dados {
            let text = speech.transcricao.as_deref().or(speech.sumario.as_deref()).unwrap_or("").trim().to_string();
            if text.is_empty() {
                continue;
            }
            let kind = speech.tipo_discurso.as_deref().unwrap_or("Discurso");
            let title = format!("Discurso de {} ({}) – {}", ctx.name, kind.to_lowercase(), br_date(&speech.data_hora_inicio));
            let slug: String = kind.chars().filter(|c| c.is_alphanumeric()).collect();
            let url = format!("https://www.camara.leg.br/deputados/{id}#discurso-{}-{slug}", speech.data_hora_inicio);
            let author = ctx.name.clone();
            ctx.document(Doc {
                url: &url,
                kind: "speech",
                title: &title,
                author: Some(&author),
                site: Some(SITE),
                published_at: Some(&speech.data_hora_inicio),
                text: &text,
                note: None,
            })
            .await?;
            latest = latest.max(speech.data_hora_inicio);
        }
        if count < 100 {
            break;
        }
    }
    ctx.set_cursor("speeches", &latest).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bill {
    id: i64,
    sigla_tipo: String,
    numero: i64,
    ano: i64,
    ementa: String,
    data_apresentacao: Option<String>,
}

pub fn bill_url(id: &str) -> String {
    format!("https://www.camara.leg.br/proposicoesWeb/fichadetramitacao?idProposicao={id}")
}

async fn bills(ctx: &mut Ctx<'_>, id: &str) -> Result<()> {
    let from = ctx.cursor("bills").await?.unwrap_or_else(|| HISTORY_START.to_string());
    let types: String = BILL_TYPES.iter().map(|t| format!("&siglaTipo={t}")).collect();
    let mut latest = from.clone();
    for page in 1..=PAGES_PER_RUN {
        let url = format!(
            "{API}/proposicoes?idDeputadoAutor={id}{types}&dataApresentacaoInicio={}&ordenarPor=id&ordem=ASC&itens=100&pagina={page}",
            &from[..10.min(from.len())]
        );
        let listed: Envelope<Vec<Bill>> = fetch::get_json(&url).await?;
        let count = listed.dados.len();
        for bill in listed.dados {
            let label = format!("{} {}/{}", bill.sigla_tipo, bill.numero, bill.ano);
            let date = bill.data_apresentacao.clone().unwrap_or_default();
            let short: String = bill.ementa.chars().take(110).collect();
            let title = format!("{label} – {short}{}", if bill.ementa.chars().count() > 110 { "…" } else { "" });
            let text = format!("{label}, apresentado em {} com autoria (ou coautoria) de {}.\n\nEmenta: {}", br_date(&date), ctx.name, bill.ementa);
            let url = bill_url(&bill.id.to_string());
            let doc = ctx
                .document(Doc {
                    url: &url,
                    kind: "bill",
                    title: &title,
                    author: None,
                    site: Some(SITE),
                    published_at: bill.data_apresentacao.as_deref(),
                    text: &text,
                    note: None,
                })
                .await?;
            let bill_id = ctx.entity("bill", &format!("camara:prop:{}", bill.id), &label, json!({"ementa": bill.ementa}), Some(&doc)).await?;
            let person = ctx.person.clone();
            ctx.relate(&person, &bill_id, "AUTHORED", date.get(..10).unwrap_or(""), json!({}), Some(&doc)).await?;
            latest = latest.max(date);
        }
        if count < 100 {
            break;
        }
    }
    ctx.set_cursor("bills", &latest).await
}

struct Session {
    date: String,
    body: String,
    description: String,
    approved: bool,
    yes: String,
    no: String,
}

struct VotedBill {
    id: String,
    title: String,
    summary: String,
}

/// Years from the cursor (or the start of the history) to now; the cursor keeps the current
/// year, whose dump keeps growing.
async fn years(ctx: &Ctx<'_>, stream: &str) -> Result<Vec<i32>> {
    let start: i32 = ctx.cursor(stream).await?.and_then(|y| y.parse().ok()).unwrap_or(2023);
    Ok((start..=Utc::now().year()).collect())
}

async fn votes(ctx: &mut Ctx<'_>, id: &str) -> Result<()> {
    let this_year = Utc::now().year();
    for year in years(ctx, "votes_year").await? {
        let refresh = year == this_year;
        let votes_path = bulk_file(ctx.pool, ctx.app_data_dir, &format!("{FILES}/votacoesVotos/csv/votacoesVotos-{year}.csv"), refresh).await?;
        let votes = Table::parse(&std::fs::read_to_string(&votes_path)?)?;
        let mine: Vec<(String, String)> = votes
            .rows()
            .filter(|row| row.get("deputado_id") == id)
            .map(|row| (row.get("idVotacao").to_string(), row.get("voto").to_string()))
            .collect();
        if mine.is_empty() {
            ctx.set_cursor("votes_year", &year.to_string()).await?;
            continue;
        }

        let sessions_path = bulk_file(ctx.pool, ctx.app_data_dir, &format!("{FILES}/votacoes/csv/votacoes-{year}.csv"), refresh).await?;
        let sessions: HashMap<String, Session> = Table::parse(&std::fs::read_to_string(&sessions_path)?)?
            .rows()
            .map(|row| {
                (
                    row.get("id").to_string(),
                    Session {
                        date: row.get("data").to_string(),
                        body: row.get("siglaOrgao").to_string(),
                        description: row.get("descricao").to_string(),
                        approved: row.get("aprovacao") == "1",
                        yes: row.get("votosSim").to_string(),
                        no: row.get("votosNao").to_string(),
                    },
                )
            })
            .collect();
        let bills_path = bulk_file(ctx.pool, ctx.app_data_dir, &format!("{FILES}/votacoesProposicoes/csv/votacoesProposicoes-{year}.csv"), refresh).await?;
        let mut voted_bills: HashMap<String, VotedBill> = HashMap::new();
        for row in Table::parse(&std::fs::read_to_string(&bills_path)?)?.rows() {
            voted_bills.entry(row.get("idVotacao").to_string()).or_insert_with(|| VotedBill {
                id: row.get("proposicao_id").to_string(),
                title: row.get("proposicao_titulo").to_string(),
                summary: row.get("proposicao_ementa").to_string(),
            });
        }

        for (vote_id, vote) in mine {
            let Some(session) = sessions.get(&vote_id) else { continue };
            let bill = voted_bills.get(&vote_id);
            let subject = bill.map_or_else(|| session.description.chars().take(80).collect::<String>(), |b| b.title.clone());
            let title = format!("Votação {} – {subject}: {} votou {vote}", br_date(&session.date), ctx.name);
            let mut text = format!(
                "Em {}, na Câmara dos Deputados ({}), {} votou \"{vote}\" na votação: {}",
                br_date(&session.date),
                session.body,
                ctx.name,
                session.description
            );
            if let Some(bill) = bill {
                text.push_str(&format!("\nProposição: {} – {}", bill.title, bill.summary));
            }
            text.push_str(&format!(
                "\nResultado: {} ({} votos sim, {} não).",
                if session.approved { "aprovada" } else { "rejeitada" },
                session.yes,
                session.no
            ));
            let url = match bill {
                Some(bill) => format!("{}#votacao-{vote_id}-{id}", bill_url(&bill.id)),
                None => format!("{API}/votacoes/{vote_id}#{id}"),
            };
            let doc = ctx
                .document(Doc {
                    url: &url,
                    kind: "vote",
                    title: &title,
                    author: None,
                    site: Some(SITE),
                    published_at: Some(&session.date),
                    text: &text,
                    note: None,
                })
                .await?;
            let session_id = ctx
                .entity(
                    "vote_session",
                    &format!("camara:vot:{vote_id}"),
                    &format!("{subject} ({})", br_date(&session.date)),
                    json!({"description": session.description, "approved": session.approved, "yes": session.yes, "no": session.no, "body": session.body}),
                    Some(&doc),
                )
                .await?;
            let person = ctx.person.clone();
            ctx.relate(&person, &session_id, "VOTED", &session.date, json!({"vote": vote}), Some(&doc)).await?;
            if let Some(bill) = bill.filter(|b| !b.id.is_empty()) {
                let bill_id = ctx.entity("bill", &format!("camara:prop:{}", bill.id), &bill.title, json!({"ementa": bill.summary}), None).await?;
                ctx.relate(&session_id, &bill_id, "ABOUT", "", json!({}), None).await?;
            }
        }
        ctx.set_cursor("votes_year", &year.to_string()).await?;
    }
    Ok(())
}

#[derive(Default)]
struct Month {
    total: f64,
    by_kind: BTreeMap<String, f64>,
    /// Supplier name → (CNPJ digits when a company, amount).
    by_supplier: HashMap<String, (Option<String>, f64)>,
}

/// The 14 digits of a CNPJ (companies only: an individual's CPF is left out).
fn cnpj(text: &str) -> Option<String> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    (digits.len() == 14).then_some(digits)
}

async fn expenses(ctx: &mut Ctx<'_>, id: &str) -> Result<()> {
    let this_year = Utc::now().year();
    for year in years(ctx, "expenses_year").await? {
        let archive = bulk_file(ctx.pool, ctx.app_data_dir, &format!("https://www.camara.leg.br/cotas/Ano-{year}.csv.zip"), year == this_year).await?;
        let Some(bytes) = super::zip_entry(&archive, &format!("Ano-{year}.csv"))? else {
            continue;
        };
        let table = Table::parse(&String::from_utf8_lossy(&bytes))?;
        let mut months: BTreeMap<u32, Month> = BTreeMap::new();
        for row in table.rows().filter(|row| row.get("ideCadastro") == id) {
            let Ok(month) = row.get("numMes").parse::<u32>() else { continue };
            let amount = number(row.get("vlrLiquido"));
            let entry = months.entry(month).or_default();
            entry.total += amount;
            *entry.by_kind.entry(row.get("txtDescricao").to_string()).or_default() += amount;
            let supplier = entry.by_supplier.entry(row.get("txtFornecedor").to_string()).or_insert((cnpj(row.get("txtCNPJCPF")), 0.0));
            supplier.1 += amount;
        }
        for (month, data) in months {
            let period = format!("{year}-{month:02}");
            let mut suppliers: Vec<(&String, &(Option<String>, f64))> = data.by_supplier.iter().collect();
            suppliers.sort_by(|a, b| b.1 .1.partial_cmp(&a.1 .1).unwrap_or(std::cmp::Ordering::Equal));
            let mut text = format!("Cota parlamentar (CEAP) de {} em {month:02}/{year}: total {}.\n\nPor tipo de despesa:", ctx.name, brl(data.total));
            for (kind, amount) in &data.by_kind {
                text.push_str(&format!("\n- {kind}: {}", brl(*amount)));
            }
            text.push_str("\n\nMaiores fornecedores:");
            for (name, (cnpj, amount)) in suppliers.iter().take(10) {
                let id_text = cnpj.as_deref().map(|c| format!(" (CNPJ {c})")).unwrap_or_default();
                text.push_str(&format!("\n- {name}{id_text}: {}", brl(*amount)));
            }
            let url = format!("https://www.camara.leg.br/deputados/{id}#despesas-{period}");
            let title = format!("Cota parlamentar de {} – {month:02}/{year}: {}", ctx.name, brl(data.total));
            let published = format!("{period}-01");
            let doc = ctx
                .document(Doc {
                    url: &url,
                    kind: "expense",
                    title: &title,
                    author: None,
                    site: Some(SITE),
                    published_at: Some(&published),
                    text: &text,
                    note: None,
                })
                .await?;
            for (name, (cnpj, amount)) in suppliers.iter().take(10) {
                let Some(cnpj) = cnpj else { continue };
                let company = ctx.entity("organization", &format!("cnpj:{cnpj}"), name, json!({"cnpj": cnpj}), None).await?;
                let person = ctx.person.clone();
                ctx.relate(&person, &company, "SPENT_WITH", &period, json!({"amount": (amount * 100.0).round() / 100.0}), Some(&doc)).await?;
            }
        }
        ctx.set_cursor("expenses_year", &year.to_string()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_ids() {
        assert_eq!(br_date("2026-07-07T20:44"), "07/07/2026");
        assert_eq!(cnpj("274.159.110/0013-6"), Some("27415911000136".into()));
        assert_eq!(cnpj("123.456.789-00"), None);
    }
}
