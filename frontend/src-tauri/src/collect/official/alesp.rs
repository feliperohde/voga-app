//! The Assembleia Legislativa de São Paulo's open data (al.sp.gov.br/dados-abertos): the
//! deputy's profile, the bills they authored and their office expenses by month and supplier.
//! Files, not an API: deputados.xml, documento_autor.zip + proposituras.zip, despesas_gabinetes.xml.
//! Plenary votes are not published per deputy.

use anyhow::{anyhow, Result};
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufReader;
use std::path::{Path, PathBuf};

use super::{brl, bulk_file, name_matches, number, scan_xml, Ctx, Doc, OfficialMatch, HISTORY_START};
use crate::collect::feeds::plain_text;

const DATA: &str = "https://www.al.sp.gov.br/repositorioDados";
const SITE: &str = "Assembleia Legislativa de São Paulo";
pub const KIND: &str = "alesp_deputado";
/// Bills, amendments to the constitution, resolutions, legislative decrees and motions
/// (not requests and indications, which run to thousands).
const BILL_KINDS: [(&str, &str); 6] = [("1", "PL"), ("2", "PLC"), ("3", "PR"), ("4", "PDL"), ("5", "PEC"), ("6", "Moção")];

async fn file(pool: &SqlitePool, app_data_dir: &Path, path: &str) -> Result<PathBuf> {
    bulk_file(pool, app_data_dir, &format!("{DATA}/{path}"), true).await
}

fn deputies(path: &Path) -> Result<Vec<HashMap<String, String>>> {
    let mut all = Vec::new();
    scan_xml(BufReader::new(std::fs::File::open(path)?), "Deputado", |d| {
        all.push(d.clone());
        true
    })?;
    Ok(all)
}

/// Deputies in office whose parliamentary name matches.
pub async fn find(pool: &SqlitePool, app_data_dir: &Path, name: &str) -> Result<Vec<OfficialMatch>> {
    let path = file(pool, app_data_dir, "deputados/deputados.xml").await?;
    Ok(deputies(&path)?
        .into_iter()
        .filter(|d| name_matches(name, d.get("NomeParlamentar").map(String::as_str).unwrap_or_default()))
        .map(|d| OfficialMatch {
            kind: KIND.into(),
            id: d.get("IdDeputado").cloned().unwrap_or_default(),
            label: d.get("NomeParlamentar").cloned().unwrap_or_default(),
            detail: format!("ALESP · {} · deputado(a) estadual", d.get("Partido").map(String::as_str).unwrap_or("")),
            similar: false,
        })
        .collect())
}

pub struct Bill {
    pub id: String,
    pub label: String,
    pub summary: String,
    pub date: String,
}

/// The bills (of BILL_KINDS) the author (IdSPL) signed, published since HISTORY_START.
fn bills(authors_zip: &Path, bills_zip: &Path, author: &str) -> Result<Vec<Bill>> {
    let mut signed = HashSet::new();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(authors_zip)?)?;
    scan_xml(BufReader::new(zip.by_index(0)?), "DocumentoAutor", |row| {
        if row.get("IdAutor").map(String::as_str) == Some(author) {
            signed.extend(row.get("IdDocumento").cloned());
        }
        true
    })?;
    let kinds: HashMap<&str, &str> = BILL_KINDS.into_iter().collect();
    let mut found = Vec::new();
    let mut zip = zip::ZipArchive::new(std::fs::File::open(bills_zip)?)?;
    scan_xml(BufReader::new(zip.by_index(0)?), "propositura", |row| {
        let get = |k: &str| row.get(k).map(String::as_str).unwrap_or_default();
        let date = get("DtPublicacao").get(..10).unwrap_or_default();
        if let (true, Some(kind)) = (signed.contains(get("IdDocumento")), kinds.get(get("IdNatureza"))) {
            if date >= HISTORY_START {
                found.push(Bill {
                    id: get("IdDocumento").into(),
                    label: format!("{kind} {}/{}", get("NroLegislativo"), get("AnoLegislativo")),
                    summary: get("Ementa").into(),
                    date: date.into(),
                });
            }
        }
        true
    })?;
    Ok(found)
}

#[derive(Default)]
pub struct Month {
    pub total: f64,
    pub by_kind: BTreeMap<String, f64>,
    /// name → (CNPJ, amount)
    pub by_supplier: BTreeMap<String, (String, f64)>,
}

/// Office expenses by (year, month) since HISTORY_START.
fn expenses(path: &Path, registration: &str) -> Result<BTreeMap<(i32, u32), Month>> {
    let first_year: i32 = HISTORY_START[..4].parse()?;
    let mut months: BTreeMap<(i32, u32), Month> = BTreeMap::new();
    scan_xml(BufReader::new(std::fs::File::open(path)?), "despesa", |row| {
        let get = |k: &str| row.get(k).map(String::as_str).unwrap_or_default();
        if get("Matricula") != registration {
            return true;
        }
        let (Ok(year), Ok(month)) = (get("Ano").parse::<i32>(), get("Mes").parse::<u32>()) else { return true };
        if year < first_year {
            return true;
        }
        let amount = number(get("Valor"));
        let m = months.entry((year, month)).or_default();
        m.total += amount;
        *m.by_kind.entry(get("Tipo").to_string()).or_default() += amount;
        let supplier = m.by_supplier.entry(get("Fornecedor").to_string()).or_insert_with(|| (get("CNPJ").to_string(), 0.0));
        supplier.1 += amount;
        true
    })?;
    Ok(months)
}

pub async fn run(ctx: &mut Ctx<'_>) -> Result<()> {
    let id = ctx.source.url.clone();
    let list = file(ctx.pool, ctx.app_data_dir, "deputados/deputados.xml").await?;
    let deputy = deputies(&list)?
        .into_iter()
        .find(|d| d.get("IdDeputado") == Some(&id))
        .ok_or_else(|| anyhow!("Deputy {id} is no longer in the ALESP's list (only those in office are published)"))?;
    let get = |k: &str| deputy.get(k).cloned().unwrap_or_default();
    let (name, registration, author) = (get("NomeParlamentar"), get("Matricula"), get("IdSPL"));
    let profile_url = format!("https://www.al.sp.gov.br/deputado/?matricula={registration}");

    let text = format!(
        "{name}, deputado(a) estadual de São Paulo ({}).\n\nÁreas de atuação: {}\n\nBase eleitoral: {}\n\n{}",
        get("Partido"),
        get("txtAreaAtuacao").replace(";\n", "; ").trim_end_matches(';'),
        get("txtBaseEleitoral").replace(";\n", "; ").trim_end_matches(';'),
        plain_text(&get("Biografia"))
    );
    let doc = ctx
        .document(Doc { url: &profile_url, kind: "profile", title: &format!("{name} – ALESP"), author: None, site: Some(SITE), published_at: None, text: &text, note: None })
        .await?;
    ctx.describe_person(json!({"alesp_id": id, "uf": "SP"}), &doc).await?;
    ctx.member_of(&get("Partido"), Some(&doc)).await?;

    let authors_zip = file(ctx.pool, ctx.app_data_dir, "processo_legislativo/documento_autor.zip").await?;
    let bills_zip = file(ctx.pool, ctx.app_data_dir, "processo_legislativo/proposituras.zip").await?;
    let found = tokio::task::spawn_blocking(move || bills(&authors_zip, &bills_zip, &author)).await??;
    let person = ctx.person.clone();
    for bill in found {
        let url = format!("https://www.al.sp.gov.br/propositura/?id={}", bill.id);
        let title = format!("{} – {}", bill.label, bill.summary.chars().take(160).collect::<String>());
        let text = format!("{}, apresentado em {} na ALESP com autoria de {name}.\n\nEmenta: {}", bill.label, bill.date, bill.summary);
        let doc = ctx.document(Doc { url: &url, kind: "bill", title: &title, author: Some(&name), site: Some(SITE), published_at: Some(&bill.date), text: &text, note: None }).await?;
        let entity = ctx.entity("bill", &format!("alesp:prop:{}", bill.id), &bill.label, json!({"ementa": bill.summary}), Some(&doc)).await?;
        ctx.relate(&person, &entity, "AUTHORED", &bill.date, json!({}), Some(&doc)).await?;
    }

    let expenses_file = file(ctx.pool, ctx.app_data_dir, "deputados/despesas_gabinetes.xml").await?;
    let months = tokio::task::spawn_blocking(move || expenses(&expenses_file, &registration)).await??;
    for ((year, month), data) in months {
        let period = format!("{year}-{month:02}");
        let mut suppliers: Vec<(&String, &(String, f64))> = data.by_supplier.iter().collect();
        suppliers.sort_by(|a, b| b.1 .1.total_cmp(&a.1 .1));
        let mut text = format!("Despesas de gabinete de {name} na ALESP em {month:02}/{year}: total {}.\n\nPor tipo:", brl(data.total));
        for (kind, amount) in &data.by_kind {
            text.push_str(&format!("\n- {kind}: {}", brl(*amount)));
        }
        text.push_str("\n\nMaiores fornecedores:");
        for (supplier, (cnpj, amount)) in suppliers.iter().take(10) {
            text.push_str(&format!("\n- {supplier} (CNPJ {cnpj}): {}", brl(*amount)));
        }
        let url = format!("{profile_url}#despesas-{period}");
        let title = format!("Despesas de gabinete de {name} – {month:02}/{year}: {}", brl(data.total));
        let doc = ctx.document(Doc { url: &url, kind: "expense", title: &title, author: None, site: Some(SITE), published_at: Some(&format!("{period}-01")), text: &text, note: None }).await?;
        for (supplier, (cnpj, amount)) in suppliers.iter().take(10) {
            if cnpj.len() != 14 {
                continue;
            }
            let company = ctx.entity("organization", &format!("cnpj:{cnpj}"), supplier, json!({"cnpj": cnpj}), None).await?;
            ctx.relate(&person, &company, "SPENT_WITH", &period, json!({"amount": (amount * 100.0).round() / 100.0}), Some(&doc)).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zipped(dir: &Path, name: &str, xml: &str) -> PathBuf {
        let path = dir.join(format!("{name}.zip"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file(format!("{name}.xml"), zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn bills_and_expenses_of_one_deputy() {
        let dir = std::env::temp_dir().join(format!("voga-alesp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let authors = zipped(&dir, "documento_autor", "<documentos_autores><DocumentoAutor><IdAutor>7</IdAutor><IdDocumento>10</IdDocumento><NomeAutor>A</NomeAutor></DocumentoAutor><DocumentoAutor><IdAutor>7</IdAutor><IdDocumento>11</IdDocumento><NomeAutor>A</NomeAutor></DocumentoAutor><DocumentoAutor><IdAutor>8</IdAutor><IdDocumento>12</IdDocumento><NomeAutor>B</NomeAutor></DocumentoAutor></documentos_autores>");
        let props = zipped(&dir, "proposituras", "<proposituras>\
            <propositura><AnoLegislativo>2024</AnoLegislativo><Ementa>Cria creches.</Ementa><DtPublicacao>2024-03-01T00:00:00-03:00</DtPublicacao><IdDocumento>10</IdDocumento><IdNatureza>1</IdNatureza><NroLegislativo>55</NroLegislativo></propositura>\
            <propositura><AnoLegislativo>2024</AnoLegislativo><Ementa>Indica obra.</Ementa><DtPublicacao>2024-03-02T00:00:00-03:00</DtPublicacao><IdDocumento>11</IdDocumento><IdNatureza>9</IdNatureza><NroLegislativo>9</NroLegislativo></propositura>\
            <propositura><AnoLegislativo>2024</AnoLegislativo><Ementa>Outro.</Ementa><DtPublicacao>2024-03-03T00:00:00-03:00</DtPublicacao><IdDocumento>12</IdDocumento><IdNatureza>1</IdNatureza><NroLegislativo>56</NroLegislativo></propositura></proposituras>");
        let found = bills(&authors, &props, "7").unwrap();
        assert_eq!(found.len(), 1, "only the bill, not the indication or another's bill");
        assert_eq!((found[0].label.as_str(), found[0].date.as_str()), ("PL 55/2024", "2024-03-01"));

        let expenses_file = dir.join("despesas.xml");
        std::fs::write(&expenses_file, "<despesas>\
            <despesa><Ano>2024</Ano><Matricula>300607</Matricula><Mes>5</Mes><Valor>100.5</Valor><CNPJ>02558157000162</CNPJ><Tipo>A - COMBUSTÍVEIS</Tipo><Fornecedor>POSTO</Fornecedor></despesa>\
            <despesa><Ano>2024</Ano><Matricula>300607</Matricula><Mes>5</Mes><Valor>50</Valor><CNPJ>02558157000162</CNPJ><Tipo>A - COMBUSTÍVEIS</Tipo><Fornecedor>POSTO</Fornecedor></despesa>\
            <despesa><Ano>2013</Ano><Matricula>300607</Matricula><Mes>1</Mes><Valor>9</Valor><CNPJ>1</CNPJ><Tipo>X</Tipo><Fornecedor>Y</Fornecedor></despesa>\
            <despesa><Ano>2024</Ano><Matricula>999</Matricula><Mes>5</Mes><Valor>9</Valor><CNPJ>1</CNPJ><Tipo>X</Tipo><Fornecedor>Y</Fornecedor></despesa></despesas>").unwrap();
        let months = expenses(&expenses_file, "300607").unwrap();
        assert_eq!(months.len(), 1);
        let may = &months[&(2024, 5)];
        assert_eq!(may.total, 150.5);
        assert_eq!(may.by_supplier["POSTO"], ("02558157000162".to_string(), 150.5));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
