//! Senado Federal (legis.senado.leg.br/dadosabertos): a senator's profile, speeches (their full
//! text from the Senado's site), nominal votes and authored bills.

use anyhow::Result;
use chrono::{Datelike, Utc};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{name_matches, Ctx, Doc, OfficialMatch, HISTORY_START};
use crate::collect::{extract, fetch};

const API: &str = "https://legis.senado.leg.br/dadosabertos";
const SITE: &str = "Senado Federal";
const BILL_TYPES: [&str; 5] = ["PL", "PLP", "PEC", "PDL", "PRS"];
/// Speech pages fetched per run: the Senado's site asks for 10 s between requests.
const SPEECHES_PER_RUN: usize = 25;

/// The XML-born JSON of the older endpoints has an object where a list has one item.
fn list(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

fn text<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    path.iter().fold(value, |v, key| &v[*key]).as_str().unwrap_or("").trim()
}

fn br_date(iso: &str) -> String {
    match (iso.get(..4), iso.get(5..7), iso.get(8..10)) {
        (Some(y), Some(m), Some(d)) => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

pub async fn find(name: &str) -> Result<Vec<OfficialMatch>> {
    let listed: Value = fetch::get_json(&format!("{API}/senador/lista/atual")).await?;
    Ok(list(&listed["ListaParlamentarEmExercicio"]["Parlamentares"]["Parlamentar"])
        .into_iter()
        .map(|p| &p["IdentificacaoParlamentar"])
        .filter(|id| name_matches(name, text(id, &["NomeParlamentar"])) || name_matches(name, text(id, &["NomeCompletoParlamentar"])))
        .map(|id| OfficialMatch {
            kind: "senado_senador".into(),
            id: text(id, &["CodigoParlamentar"]).to_string(),
            label: text(id, &["NomeParlamentar"]).to_string(),
            detail: format!("Senado · {}-{}", text(id, &["SiglaPartidoParlamentar"]), text(id, &["UfParlamentar"])),
            similar: false,
        })
        .collect())
}

pub async fn run(ctx: &mut Ctx<'_>) -> Result<()> {
    let code = ctx.source.url.clone();
    profile(ctx, &code).await?;
    votes(ctx, &code).await?;
    bills(ctx, &code).await?;
    speeches(ctx, &code).await?;
    Ok(())
}

fn profile_url(code: &str) -> String {
    format!("https://www25.senado.leg.br/web/senadores/senador/-/perfil/{code}")
}

async fn profile(ctx: &mut Ctx<'_>, code: &str) -> Result<()> {
    let detail: Value = fetch::get_json(&format!("{API}/senador/{code}")).await?;
    let senator = &detail["DetalheParlamentar"]["Parlamentar"];
    let id = &senator["IdentificacaoParlamentar"];
    let party = text(id, &["SiglaPartidoParlamentar"]).to_string();
    let uf = text(id, &["UfParlamentar"]).to_string();
    let basics = &senator["DadosBasicosParlamentar"];
    let mut body = format!("{} ({party}-{uf}), senador(a) da República.", text(id, &["NomeParlamentar"]));
    for (label, value) in [
        ("Nome completo", text(id, &["NomeCompletoParlamentar"])),
        ("Naturalidade", text(basics, &["Naturalidade"])),
        ("E-mail", text(id, &["EmailParlamentar"])),
    ] {
        if !value.is_empty() {
            body.push_str(&format!("\n{label}: {value}."));
        }
    }
    let url = profile_url(code);
    let title = format!("Perfil de {} no Senado", text(id, &["NomeParlamentar"]));
    let doc = ctx
        .document(Doc {
            url: &url,
            kind: "profile",
            title: &title,
            author: None,
            site: Some(SITE),
            published_at: None,
            text: &body,
            note: None,
        })
        .await?;
    ctx.describe_person(json!({"senado_id": code, "uf": uf}), &doc).await?;
    ctx.member_of(&party, Some(&doc)).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vote {
    codigo_sessao_votacao: Option<i64>,
    codigo_materia: Option<i64>,
    data_sessao: String,
    descricao_votacao: Option<String>,
    identificacao: Option<String>,
    ementa: Option<String>,
    resultado_votacao: Option<String>,
    #[serde(default)]
    votos: Vec<SenatorVote>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SenatorVote {
    sigla_voto_parlamentar: Option<String>,
}

/// What a senator's vote code says: "votou Sim", "participou de votação secreta", "esteve ausente".
fn vote_phrase(code: &str) -> String {
    match code {
        "Sim" | "Não" | "Abstenção" | "Obstrução" => format!("votou \"{code}\""),
        "Votou" => "participou da votação (voto secreto)".into(),
        "P-NRV" => "esteve presente sem registrar voto".into(),
        "AP" | "NCom" => "esteve ausente".into(),
        "MIS" => "estava em missão".into(),
        "REP" => "estava em representação da Casa".into(),
        code if code.starts_with('L') => "estava de licença".into(),
        other => format!("teve o registro \"{other}\""),
    }
}

fn matter_url(code: Option<i64>) -> String {
    match code {
        Some(code) => format!("https://www25.senado.leg.br/web/atividade/materias/-/materia/{code}"),
        None => "https://www25.senado.leg.br/web/atividade/votacoes".to_string(),
    }
}

async fn votes(ctx: &mut Ctx<'_>, code: &str) -> Result<()> {
    let from = ctx.cursor("votes").await?.unwrap_or_else(|| HISTORY_START.to_string());
    let today = Utc::now().date_naive().to_string();
    let listed: Vec<Vote> = fetch::get_json(&format!("{API}/votacao?codigoParlamentar={code}&dataInicio={from}&dataFim={today}")).await?;
    let mut latest = from.clone();
    for vote in listed {
        let Some(choice) = vote.votos.first().and_then(|v| v.sigla_voto_parlamentar.clone()) else { continue };
        let session = vote.codigo_sessao_votacao.unwrap_or_default();
        let subject = vote.identificacao.clone().unwrap_or_else(|| "Votação".into());
        let description = vote.descricao_votacao.clone().unwrap_or_default();
        let result = match vote.resultado_votacao.as_deref() {
            Some("A") => "aprovada",
            Some("R") => "rejeitada",
            _ => "resultado não informado",
        };
        let phrase = vote_phrase(&choice);
        let title = format!("Votação {} – {subject}: {} {phrase}", br_date(&vote.data_sessao), ctx.name);
        let mut body = format!("Em {}, no Senado Federal, {} {phrase} na votação: {description}", br_date(&vote.data_sessao), ctx.name);
        if let Some(summary) = &vote.ementa {
            body.push_str(&format!("\nMatéria: {subject} – {summary}"));
        }
        body.push_str(&format!("\nResultado: {result}."));
        let url = format!("{}#votacao-{session}-{code}", matter_url(vote.codigo_materia));
        let doc = ctx
            .document(Doc {
                url: &url,
                kind: "vote",
                title: &title,
                author: None,
                site: Some(SITE),
                published_at: Some(&vote.data_sessao),
                text: &body,
                note: None,
            })
            .await?;
        let session_id = ctx
            .entity(
                "vote_session",
                &format!("senado:vot:{session}"),
                &format!("{subject} ({})", br_date(&vote.data_sessao)),
                json!({"description": description, "result": result}),
                Some(&doc),
            )
            .await?;
        let person = ctx.person.clone();
        ctx.relate(&person, &session_id, "VOTED", &vote.data_sessao, json!({"vote": choice}), Some(&doc)).await?;
        if let Some(matter) = vote.codigo_materia {
            let bill_id = ctx.entity("bill", &format!("senado:materia:{matter}"), &subject, json!({"ementa": vote.ementa}), None).await?;
            ctx.relate(&session_id, &bill_id, "ABOUT", "", json!({}), None).await?;
        }
        latest = latest.max(vote.data_sessao);
    }
    ctx.set_cursor("votes", &latest).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Matter {
    codigo_materia: Option<i64>,
    identificacao: Option<String>,
    ementa: Option<String>,
    data_apresentacao: Option<String>,
}

async fn bills(ctx: &mut Ctx<'_>, code: &str) -> Result<()> {
    let from = ctx.cursor("bills").await?.unwrap_or_else(|| HISTORY_START.to_string());
    let listed: Vec<Matter> = fetch::get_json(&format!("{API}/processo?codigoParlamentarAutor={code}")).await?;
    let mut latest = from.clone();
    for matter in listed {
        let (Some(label), Some(date)) = (matter.identificacao.as_deref(), matter.data_apresentacao.as_deref()) else { continue };
        let kind = label.split_whitespace().next().unwrap_or_default();
        if !BILL_TYPES.contains(&kind) || date < from.as_str() {
            continue;
        }
        let summary = matter.ementa.clone().unwrap_or_default();
        let short: String = summary.chars().take(110).collect();
        let title = format!("{label} – {short}{}", if summary.chars().count() > 110 { "…" } else { "" });
        let body = format!("{label}, apresentado em {} com autoria (ou coautoria) de {}.\n\nEmenta: {summary}", br_date(date), ctx.name);
        let url = matter_url(matter.codigo_materia);
        let doc = ctx
            .document(Doc {
                url: &url,
                kind: "bill",
                title: &title,
                author: None,
                site: Some(SITE),
                published_at: Some(date),
                text: &body,
                note: None,
            })
            .await?;
        let key = matter.codigo_materia.map_or_else(|| format!("senado:materia:{label}"), |c| format!("senado:materia:{c}"));
        let bill_id = ctx.entity("bill", &key, label, json!({"ementa": summary}), Some(&doc)).await?;
        let person = ctx.person.clone();
        ctx.relate(&person, &bill_id, "AUTHORED", date.get(..10).unwrap_or(""), json!({}), Some(&doc)).await?;
        latest = latest.max(date.to_string());
    }
    ctx.set_cursor("bills", &latest).await
}

async fn speeches(ctx: &mut Ctx<'_>, code: &str) -> Result<()> {
    let from = ctx.cursor("speeches").await?.unwrap_or_else(|| HISTORY_START.to_string());
    let start_year: i32 = from.get(..4).and_then(|y| y.parse().ok()).unwrap_or(2023);
    let mut latest = from.clone();
    let mut fetched = 0;
    // One year per request: the endpoint answers windows of at most a year
    'years: for year in start_year..=Utc::now().year() {
        let begin = if year == start_year { from.replace('-', "") } else { format!("{year}0101") };
        let url = format!("{API}/senador/{code}/discursos?dataInicio={}&dataFim={year}1231", &begin[..8.min(begin.len())]);
        let listed: Value = fetch::get_json(&url).await?;
        let mut speeches = list(&listed["DiscursosParlamentar"]["Parlamentar"]["Pronunciamentos"]["Pronunciamento"]);
        speeches.sort_by_key(|s| text(s, &["DataPronunciamento"]).to_string());
        for speech in speeches {
            let date = text(speech, &["DataPronunciamento"]).to_string();
            let page = text(speech, &["UrlTexto"]).to_string();
            if date.is_empty() || page.is_empty() {
                continue;
            }
            let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM documents WHERE investigation_id = ? AND url = ?")
                .bind(ctx.investigation())
                .bind(&page)
                .fetch_optional(ctx.pool)
                .await?;
            if exists.is_none() {
                if fetched == SPEECHES_PER_RUN {
                    break 'years;
                }
                fetched += 1;
                let summary = text(speech, &["TextoResumo"]);
                let body = match fetch::get_body(&page).await {
                    Ok(fetched) => extract::article(&String::from_utf8_lossy(&fetched.body), &page).text,
                    Err(e) => {
                        log::warn!("Senado: speech text {page} unavailable: {e}");
                        String::new()
                    }
                };
                let full = if body.trim().is_empty() { summary.to_string() } else { format!("Resumo: {summary}\n\n{body}") };
                let kind = text(speech, &["TipoUsoPalavra", "Descricao"]);
                let title = format!("Discurso de {} ({}) – {}", ctx.name, kind.to_lowercase(), br_date(&date));
                let author = ctx.name.clone();
                ctx.document(Doc {
                    url: &page,
                    kind: "speech",
                    title: &title,
                    author: Some(&author),
                    site: Some(SITE),
                    published_at: Some(&date),
                    text: &full,
                    note: None,
                })
                .await?;
            }
            latest = latest.max(date);
        }
    }
    ctx.set_cursor("speeches", &latest).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_items_and_lists_read_alike() {
        let one = json!({"a": {"b": "x"}});
        let many = json!({"a": [{"b": "x"}, {"b": "y"}]});
        assert_eq!(list(&one["a"]).len(), 1);
        assert_eq!(list(&many["a"]).len(), 2);
        assert!(list(&one["missing"]).is_empty());
        assert_eq!(text(&many["a"][1], &["b"]), "y");
    }

    #[test]
    fn vote_codes_read_as_sentences() {
        assert_eq!(vote_phrase("Não"), "votou \"Não\"");
        assert_eq!(vote_phrase("Votou"), "participou da votação (voto secreto)");
        assert_eq!(vote_phrase("LS"), "estava de licença");
    }
}
