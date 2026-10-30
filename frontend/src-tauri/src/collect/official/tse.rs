//! TSE (dadosabertos.tse.jus.br): a candidacy (office, party, number, result), the assets the
//! candidate declared and, for executive offices, the government proposal filed with the
//! candidacy: the written promises of the campaign. Read from the TSE's yearly dumps.
//!
//! Personal data the TSE publishes but an investigation does not need (CPF, voter id, e-mail,
//! birth date) is not stored.

use anyhow::{anyhow, bail, Result};
use chrono::{Datelike, Utc};
use serde_json::json;
use sqlx::SqlitePool;
use std::path::Path;

use super::{brl, bulk_file, same_name, bulk_file_up_to, latin1, name_matches, number, remote_zip_entries, zip_entries, zip_entry, Ctx, Doc, OfficialMatch, Table};
use crate::collect::extract;

const CDN: &str = "https://cdn.tse.jus.br/estatistica/sead/odsele";
const SITE: &str = "TSE";
/// Offices whose candidates file a government proposal.
const EXECUTIVE: [&str; 3] = ["PRESIDENTE", "GOVERNADOR", "PREFEITO"];

fn dataset_url(year: i32) -> String {
    format!("https://dadosabertos.tse.jus.br/dataset/candidatos-{year}")
}

/// dd/mm/yyyy → yyyy-mm-dd
fn iso_date(br: &str) -> String {
    match (br.get(6..10), br.get(3..5), br.get(..2)) {
        (Some(y), Some(m), Some(d)) => format!("{y}-{m}-{d}"),
        _ => String::new(),
    }
}

async fn candidates_archive(pool: &SqlitePool, app_data_dir: &Path, year: i32) -> Result<std::path::PathBuf> {
    // The current election's file still changes (registrations, results)
    bulk_file(pool, app_data_dir, &format!("{CDN}/consulta_cand/consulta_cand_{year}.zip"), year >= Utc::now().year() - 1).await
}

/// Candidates of `year` (all states) whose name or ballot name has every word of `name`; for a
/// mayor, also the city hall (its purchases and contracts, pncp.rs).
pub async fn find(pool: &SqlitePool, app_data_dir: &Path, name: &str, year: i32) -> Result<Vec<OfficialMatch>> {
    let archive = candidates_archive(pool, app_data_dir, year).await?;
    let name = name.to_string();
    let (mut found, cities) = tokio::task::spawn_blocking(move || -> Result<(Vec<OfficialMatch>, Vec<(String, String)>)> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&archive)?)?;
        let files: Vec<String> = zip.file_names().filter(|f| f.ends_with(".csv")).map(String::from).collect();
        let mut found: Vec<OfficialMatch> = Vec::new();
        let mut cities: Vec<(String, String)> = Vec::new();
        for file in files {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut zip.by_name(&file)?, &mut bytes)?;
            for row in Table::parse(&latin1(&bytes))?.rows() {
                if !name_matches(&name, row.get("NM_CANDIDATO")) && !name_matches(&name, row.get("NM_URNA_CANDIDATO")) {
                    continue;
                }
                let id = format!("{year}:{}:{}", row.get("SG_UF"), row.get("SQ_CANDIDATO"));
                // Known by the ballot name ("RICARDO ARRUDA NUNES" is Ricardo Arruda); the full name counts only when typed whole
                let similar = !same_name(&name, row.get("NM_URNA_CANDIDATO")) && crate::collect::fold(&name) != crate::collect::fold(row.get("NM_CANDIDATO"));
                let city = (row.get("SG_UF").to_string(), row.get("NM_UE").to_string());
                // A homonym's city hall is not offered
                if row.get("DS_CARGO") == "PREFEITO" && !similar && !cities.contains(&city) {
                    cities.push(city);
                }
                // One row per round: the last one has the final result
                found.retain(|m| m.id != id);
                found.push(OfficialMatch {
                    kind: "tse_candidato".into(),
                    id,
                    label: row.get("NM_URNA_CANDIDATO").to_string(),
                    detail: format!(
                        "TSE {year} · {} · {} {} · {} · {}",
                        row.get("DS_CARGO"),
                        row.get("SG_PARTIDO"),
                        row.get("NR_CANDIDATO"),
                        row.get("NM_UE"),
                        row.get("DS_SIT_TOT_TURNO")
                    ),
                    similar,
                });
            }
        }
        Ok((found, cities))
    })
    .await??;
    for (uf, city) in cities {
        match super::pncp::city_hall(&uf, &city).await {
            Ok(Some(hall)) => found.push(hall),
            Ok(None) => log::warn!("Official: no IBGE code for {city} ({uf})"),
            Err(e) => log::warn!("Official: IBGE lookup for {city} failed: {e}"),
        }
    }
    Ok(found)
}

pub async fn run(ctx: &mut Ctx<'_>) -> Result<()> {
    let parts: Vec<&str> = ctx.source.url.split(':').collect();
    let [year, uf, sq] = parts[..] else { bail!("Invalid TSE candidate id: {}", ctx.source.url) };
    let year: i32 = year.parse()?;
    let (uf, sq) = (uf.to_string(), sq.to_string());

    let archive = candidates_archive(ctx.pool, ctx.app_data_dir, year).await?;
    let bytes = zip_entry(&archive, &format!("consulta_cand_{year}_{uf}.csv"))?.ok_or_else(|| anyhow!("No {uf} candidates in {year}"))?;
    let table = Table::parse(&latin1(&bytes))?;
    let row = table
        .rows()
        .filter(|r| r.get("SQ_CANDIDATO") == sq)
        .max_by_key(|r| r.get("NR_TURNO").parse::<u32>().unwrap_or(0))
        .ok_or_else(|| anyhow!("Candidate {sq} not found in the TSE {year} data"))?;

    let office = row.get("DS_CARGO").to_string();
    let party = row.get("SG_PARTIDO").to_string();
    let number_on_ballot = row.get("NR_CANDIDATO").to_string();
    let result = row.get("DS_SIT_TOT_TURNO").to_string();
    let election_date = iso_date(row.get("DT_ELEICAO"));
    let place = row.get("NM_UE").to_string();
    let ballot_name = row.get("NM_URNA_CANDIDATO").to_string();

    let mut text = format!(
        "Candidatura de {} ({ballot_name}) a {office} em {place} nas eleições de {year}, pelo {party} ({}), número {number_on_ballot}.",
        row.get("NM_CANDIDATO"),
        row.get("NM_PARTIDO"),
    );
    for (label, column) in [
        ("Federação", "NM_FEDERACAO"),
        ("Coligação", "NM_COLIGACAO"),
        ("Situação da candidatura", "DS_SITUACAO_CANDIDATURA"),
        ("Resultado", "DS_SIT_TOT_TURNO"),
        ("Ocupação declarada", "DS_OCUPACAO"),
        ("Grau de instrução", "DS_GRAU_INSTRUCAO"),
    ] {
        let value = row.get(column);
        if !value.is_empty() && value != "PARTIDO ISOLADO" {
            text.push_str(&format!("\n{label}: {value}."));
        }
    }

    // Declared assets
    let assets_archive = bulk_file(ctx.pool, ctx.app_data_dir, &format!("{CDN}/bem_candidato/bem_candidato_{year}.zip"), false).await?;
    if let Some(bytes) = zip_entry(&assets_archive, &format!("bem_candidato_{year}_{uf}.csv"))? {
        let assets = Table::parse(&latin1(&bytes))?;
        let mine: Vec<(String, String, f64)> = assets
            .rows()
            .filter(|r| r.get("SQ_CANDIDATO") == sq)
            .map(|r| (r.get("DS_TIPO_BEM_CANDIDATO").to_string(), r.get("DS_BEM_CANDIDATO").to_string(), number(r.get("VR_BEM_CANDIDATO"))))
            .collect();
        let total: f64 = mine.iter().map(|(_, _, value)| value).sum();
        if mine.is_empty() {
            text.push_str("\n\nNenhum bem declarado ao TSE.");
        } else {
            text.push_str(&format!("\n\nBens declarados ao TSE (total {}):", brl(total)));
            for (kind, description, value) in &mine {
                text.push_str(&format!("\n- {kind}: {description} – {}", brl(*value)));
            }
        }
    }

    let url = format!("{}#candidato-{sq}", dataset_url(year));
    let title = format!("Candidatura de {ballot_name} a {} em {year} ({party} {number_on_ballot}): {}", office.to_lowercase(), result.to_lowercase());
    let doc = ctx
        .document(Doc {
            url: &url,
            kind: "candidacy",
            title: &title,
            author: None,
            site: Some(SITE),
            published_at: Some(&election_date),
            text: &text,
            note: None,
        })
        .await?;
    let candidacy = ctx
        .entity(
            "candidacy",
            &format!("tse:cand:{sq}"),
            &format!("{office} {year} ({party} {number_on_ballot})"),
            json!({
                "office": office, "party": party, "number": number_on_ballot, "result": result, "place": place,
                // The parties it ran with: their acts are the political group's, not someone else's
                "coalition": crate::analyze::actor::coalition_parties(row.get("DS_COMPOSICAO_COLIGACAO")),
                "federation": crate::analyze::actor::coalition_parties(row.get("DS_COMPOSICAO_FEDERACAO")),
            }),
            Some(&doc),
        )
        .await?;
    let person = ctx.person.clone();
    ctx.relate(&person, &candidacy, "CANDIDATE_IN", &election_date, json!({"result": result}), Some(&doc)).await?;
    if !party.is_empty() {
        let party_id = ctx.entity("party", &format!("party:{}", party.to_uppercase()), &party, json!({}), None).await?;
        ctx.relate(&person, &party_id, "MEMBER_OF", &election_date, json!({"source": "TSE"}), Some(&doc)).await?;
    }

    if EXECUTIVE.iter().any(|e| office.to_uppercase().starts_with(e)) {
        proposal(ctx, year, &uf, &sq, &ballot_name, &office, &election_date).await?;
    }
    Ok(())
}

/// The government proposal filed with an executive candidacy, as text.
async fn proposal(ctx: &mut Ctx<'_>, year: i32, uf: &str, sq: &str, name: &str, office: &str, election_date: &str) -> Result<()> {
    let zip_url = format!("{CDN}/proposta_governo/proposta_governo_{year}_{uf}.zip");
    // {uf}/{year}{uf}{sq}.pdf, or in parts: …_01.pdf, _02.pdf (2024)
    let prefix = format!("{uf}/{year}{uf}{sq}");
    let wanted = |name: &str| super::is_part(name, &prefix);
    // A state's file has every candidate's PDF (São Paulo's, 1.4 GB): by default only this one is
    // read out of it, unless the user chose to keep the whole file or it is already here
    let full = crate::prefs::get(ctx.pool, crate::prefs::FULL_PROPOSALS).await?.is_some_and(|v| v == "1");
    let local = ctx.app_data_dir.join("bulk").join(zip_url.rsplit('/').next().unwrap_or(""));
    let pdfs = if full || local.exists() {
        let archive = bulk_file_up_to(ctx.pool, ctx.app_data_dir, &zip_url, false, 3 * 1024 * 1024 * 1024).await?;
        zip_entries(&archive, wanted)?
    } else {
        remote_zip_entries(&zip_url, wanted).await?
    };
    if pdfs.is_empty() {
        log::info!("TSE: no government proposal for {sq} in {year}");
        return Ok(());
    }
    let mut parts = Vec::new();
    let mut unreadable = false;
    for pdf in pdfs {
        match extract::pdf_text(pdf).await {
            Ok(text) if !text.trim().is_empty() => parts.push(text),
            Ok(_) => {}
            Err(e) => {
                log::warn!("TSE: proposal of {sq} unreadable: {e}");
                unreadable = true;
            }
        }
    }
    let text = parts.join("\n\n");
    let note = match (text.trim().is_empty(), unreadable) {
        (false, _) => None,
        (true, false) => Some("O PDF da proposta não tem texto (provavelmente é uma imagem escaneada)."),
        (true, true) => Some("Não foi possível ler o PDF da proposta."),
    };
    let url = format!("{}#proposta-{sq}", dataset_url(year));
    let title = format!("Proposta de governo de {name} – {} {year}", office.to_lowercase());
    let author = ctx.name.clone();
    ctx.document(Doc {
        url: &url,
        kind: "proposal",
        title: &title,
        author: Some(&author),
        site: Some(SITE),
        published_at: Some(election_date),
        text: &text,
        note,
    })
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn election_dates() {
        assert_eq!(iso_date("02/10/2022"), "2022-10-02");
        assert_eq!(iso_date(""), "");
    }
}
