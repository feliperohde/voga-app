//! Parliamentary amendments (emendas parlamentares) from the Portal da Transparência's open data:
//! one ZIP (no key needed) with every amendment since 2014, by author, place, policy area and
//! amounts committed and paid, the agreements (convênios) they funded and who received the money
//! (companies, towns and funds, by CNPJ). A target is linked to its author code; each amendment
//! becomes a document and an entity the person AUTHORED, PAID_TO each recipient. Its emendas Pix
//! come from the Transferegov (transferegov.rs), under the same author code.

use anyhow::Result;
use serde_json::json;
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{brl, bulk_file, name_matches, number, scan_zip_csv, Ctx, Doc, OfficialMatch};

const ZIP: &str = "https://dadosabertos-download.cgu.gov.br/PortalDaTransparencia/saida/emendas-parlamentares/EmendasParlamentares.zip";
const AMENDMENTS: &str = "EmendasParlamentares.csv";
const AGREEMENTS: &str = "EmendasParlamentares_Convenios.csv";
const RECIPIENTS: &str = "EmendasParlamentares_PorFavorecido.csv";
/// Recipients of one amendment linked in the graph (the document lists them all).
/// ponytail: amendments to health funds pay hundreds of suppliers; the largest keep the Map readable.
const GRAPH_RECIPIENTS: usize = 30;
pub const KIND: &str = "transparencia_emendas";

async fn file(pool: &SqlitePool, app_data_dir: &Path) -> Result<PathBuf> {
    bulk_file(pool, app_data_dir, ZIP, true).await
}

/// Authors whose name matches, with how many amendments and which years.
pub async fn find(pool: &SqlitePool, app_data_dir: &Path, name: &str) -> Result<Vec<OfficialMatch>> {
    let path = file(pool, app_data_dir).await?;
    let name = name.to_string();
    let authors = tokio::task::spawn_blocking(move || -> Result<BTreeMap<String, (String, usize, String, String)>> {
        let mut authors: BTreeMap<String, (String, usize, String, String)> = BTreeMap::new();
        scan_zip_csv(&path, AMENDMENTS, |row| {
            let author = row.get("Nome do Autor da Emenda");
            if name_matches(&name, author) {
                let year = row.get("Ano da Emenda").to_string();
                let entry = authors.entry(row.get("Código do Autor da Emenda").to_string()).or_insert_with(|| (author.to_string(), 0, year.clone(), year.clone()));
                entry.1 += 1;
                entry.2 = entry.2.clone().min(year.clone());
                entry.3 = entry.3.clone().max(year);
            }
            true
        })?;
        Ok(authors)
    })
    .await??;
    Ok(authors
        .into_iter()
        .filter(|(code, _)| code != "S/I")
        .map(|(code, (author, count, first, last))| OfficialMatch {
            kind: KIND.into(),
            id: code,
            label: author,
            detail: format!("Portal da Transparência · {count} emendas parlamentares ({first}–{last})"),
            similar: false,
        })
        .collect())
}

#[derive(Debug, Default, Clone)]
pub struct Amendment {
    pub code: String,
    pub year: String,
    pub number: String,
    pub kind: String,
    pub places: Vec<String>,
    pub areas: Vec<String>,
    pub actions: Vec<String>,
    pub committed: f64,
    pub paid: f64,
    pub agreements: Vec<String>,
    /// Companies, towns and funds paid (by CNPJ), largest first.
    pub recipients: Vec<Recipient>,
    /// Paid to people (by CPF): only how many and how much, never who.
    pub people_paid: (usize, f64),
}

#[derive(Debug, Default, Clone)]
pub struct Recipient {
    pub cnpj: String,
    pub name: String,
    pub nature: String,
    pub place: String,
    pub amount: f64,
    /// Last month paid, yyyy-mm.
    pub month: String,
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !value.is_empty() && !list.iter().any(|v| v == value) {
        list.push(value.to_string());
    }
}

/// The author's amendments (one per code; a code can have rows per place and action).
fn read(path: &Path, author_code: &str) -> Result<Vec<Amendment>> {
    let mut by_code: BTreeMap<String, Amendment> = BTreeMap::new();
    scan_zip_csv(path, AMENDMENTS, |row| {
        if row.get("Código do Autor da Emenda") == author_code && row.get("Código da Emenda") != "Sem informação" {
            let code = row.get("Código da Emenda").to_string();
            let a = by_code.entry(code.clone()).or_insert_with(|| Amendment {
                code,
                year: row.get("Ano da Emenda").into(),
                number: row.get("Número da emenda").into(),
                kind: row.get("Tipo de Emenda").into(),
                ..Default::default()
            });
            push_unique(&mut a.places, row.get("Localidade de aplicação do recurso"));
            push_unique(&mut a.areas, &format!("{} / {}", row.get("Nome Função"), row.get("Nome Subfunção")));
            push_unique(&mut a.actions, row.get("Nome Ação"));
            a.committed += number(row.get("Valor Empenhado"));
            a.paid += number(row.get("Valor Pago")) + number(row.get("Valor Restos A Pagar Pagos"));
        }
        true
    })?;
    if !by_code.is_empty() {
        scan_zip_csv(path, AGREEMENTS, |row| {
            if let Some(a) = by_code.get_mut(row.get("Código da Emenda")) {
                let agreement = format!(
                    "Convênio {} com {} ({}): {} – {}",
                    row.get("Número Convênio"),
                    row.get("Convenente"),
                    row.get("Data Publicação Convênio"),
                    row.get("Objeto Convênio"),
                    brl(number(row.get("Valor Convênio")))
                );
                push_unique(&mut a.agreements, &agreement);
            }
            true
        })?;
        let mut recipients: BTreeMap<(String, String), Recipient> = BTreeMap::new();
        let mut people: BTreeMap<String, std::collections::HashSet<String>> = BTreeMap::new();
        scan_zip_csv(path, RECIPIENTS, |row| {
            let code = row.get("Código da Emenda");
            let Some(a) = by_code.get_mut(code) else { return true };
            let amount = number(row.get("Valor Recebido"));
            let id: String = row.get("Código do Favorecido").chars().filter(char::is_ascii_digit).collect();
            if row.get("Tipo Favorecido") != "Pessoa Jurídica" || id.len() != 14 {
                if row.get("Tipo Favorecido") == "Pessoa Fisica" {
                    a.people_paid.1 += amount;
                    // Only to count them: the documents are never stored
                    people.entry(code.to_string()).or_default().insert(id);
                }
                return true;
            }
            let month = row.get("Ano/Mês");
            let month = if month.len() == 6 { format!("{}-{}", &month[..4], &month[4..]) } else { String::new() };
            let r = recipients.entry((code.to_string(), id.clone())).or_insert_with(|| Recipient {
                cnpj: id,
                name: row.get("Favorecido").into(),
                nature: row.get("Natureza Jurídica").into(),
                place: format!("{} - {}", row.get("Município Favorecido"), row.get("UF Favorecido")),
                ..Default::default()
            });
            r.amount += amount;
            r.month = r.month.clone().max(month);
            true
        })?;
        for ((code, _), r) in recipients {
            by_code.get_mut(&code).unwrap().recipients.push(r);
        }
        for (code, set) in people {
            by_code.get_mut(&code).unwrap().people_paid.0 = set.len();
        }
        for a in by_code.values_mut() {
            a.recipients.sort_by(|x, y| y.amount.total_cmp(&x.amount));
        }
    }
    Ok(by_code.into_values().collect())
}

pub fn text(author: &str, a: &Amendment) -> String {
    let mut text = format!(
        "Emenda parlamentar {} de {} ({}), nº {}.\nTipo: {}.\nOnde: {}.\nÁrea: {}.\nAção: {}.\nValor empenhado: {}. Valor pago: {}.",
        a.code,
        author,
        a.year,
        a.number,
        a.kind,
        a.places.join("; "),
        a.areas.join("; "),
        a.actions.join("; "),
        brl(a.committed),
        brl(a.paid)
    );
    if !a.agreements.is_empty() {
        text.push_str(&format!("\n\nConvênios:\n- {}", a.agreements.join("\n- ")));
    }
    if !a.recipients.is_empty() || a.people_paid.0 > 0 {
        text.push_str("\n\nQuem recebeu:");
        for r in &a.recipients {
            text.push_str(&format!("\n- {} (CNPJ {}, {}, {}): {} até {}", r.name, r.cnpj, r.nature, r.place, brl(r.amount), r.month));
        }
        if a.people_paid.0 > 0 {
            text.push_str(&format!("\n- Pessoas físicas ({}): {} no total", a.people_paid.0, brl(a.people_paid.1)));
        }
    }
    text
}

pub async fn run(ctx: &mut Ctx<'_>) -> Result<()> {
    let path = file(ctx.pool, ctx.app_data_dir).await?;
    let code = ctx.source.url.clone();
    let amendments = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || read(&path, &code)).await??
    };
    let author = ctx.name.clone();
    let person = ctx.person.clone();
    for a in amendments {
        let area = a.areas.first().and_then(|s| s.split(" / ").next()).unwrap_or("").to_string();
        let title = format!("Emenda {}/{} – {} – {}: {} pagos", a.number, a.year, area, a.places.first().map(String::as_str).unwrap_or(""), brl(a.paid));
        let url = format!("https://portaldatransparencia.gov.br/emendas/detalhe?codigoEmenda={}", a.code);
        let date = format!("{}-01-01", a.year);
        let id = ctx
            .document(Doc { url: &url, kind: "amendment", title: &title, author: Some(&author), site: Some("Portal da Transparência"), published_at: Some(&date), text: &text(&author, &a), note: None })
            .await?;
        let entity = ctx
            .entity(
                "amendment",
                &format!("emenda:{}", a.code),
                &format!("Emenda {}/{} ({area})", a.number, a.year),
                json!({"year": a.year, "area": area, "places": a.places, "committed": a.committed, "paid": a.paid}),
                Some(&id),
            )
            .await?;
        ctx.relate(&person, &entity, "AUTHORED", &date, json!({}), Some(&id)).await?;
        for r in a.recipients.iter().take(GRAPH_RECIPIENTS) {
            let org = ctx.entity("organization", &format!("cnpj:{}", r.cnpj), &r.name, json!({"cnpj": r.cnpj, "natureza": r.nature}), None).await?;
            ctx.relate(&entity, &org, "PAID_TO", &r.month, json!({"amount": (r.amount * 100.0).round() / 100.0}), Some(&id)).await?;
        }
    }
    // The emendas Pix are a second service: when it is down, the amendments above still count
    if let Err(e) = super::transferegov::run(ctx).await {
        log::warn!("Official: Transferegov for author {}: {e}", ctx.source.url);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ZIP like the Portal's: Latin-1, ';', quoted.
    fn zip(dir: &Path) -> PathBuf {
        use std::io::Write;
        let path = dir.join("emendas.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let latin1 = |s: &str| s.chars().map(|c| c as u8).collect::<Vec<u8>>();
        zip.start_file(AMENDMENTS, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&latin1(concat!(
            "\"Código da Emenda\";\"Ano da Emenda\";\"Tipo de Emenda\";\"Código do Autor da Emenda\";\"Nome do Autor da Emenda\";\"Número da emenda\";\"Localidade de aplicação do recurso\";\"Nome Função\";\"Nome Subfunção\";\"Nome Ação\";\"Valor Empenhado\";\"Valor Pago\";\"Valor Restos A Pagar Pagos\"\n",
            "\"202341320018\";\"2023\";\"Individual\";\"4132\";\"MARINA EXEMPLO\";\"0018\";\"SALTO - SP\";\"Educação\";\"Educação infantil\";\"CRECHES\";\"500000,00\";\"200000,00\";\"100000,00\"\n",
            "\"202341320018\";\"2023\";\"Individual\";\"4132\";\"MARINA EXEMPLO\";\"0018\";\"ITU - SP\";\"Educação\";\"Educação infantil\";\"CRECHES\";\"100000,00\";\"0,00\";\"0,00\"\n",
            "\"202399990001\";\"2023\";\"Individual\";\"9999\";\"OUTRO NOME\";\"0001\";\"X\";\"Saúde\";\"Atenção\";\"Y\";\"1,00\";\"0,00\";\"0,00\"\n",
        )))
        .unwrap();
        zip.start_file(AGREEMENTS, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&latin1("\"Código da Emenda\";\"Data Publicação Convênio\";\"Convenente\";\"Objeto Convênio\";\"Número Convênio\";\"Valor Convênio\"\n\"202341320018\";\"01/06/2023\";\"PREFEITURA DE SALTO\";\"CONSTRUÇÃO DE CRECHE\";\"937226\";\"300000,00\"\n")).unwrap();
        zip.start_file(RECIPIENTS, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&latin1(concat!(
            "\"Código da Emenda\";\"Código do Autor da Emenda\";\"Ano/Mês\";\"Código do Favorecido\";\"Favorecido\";\"Natureza Jurídica\";\"Tipo Favorecido\";\"UF Favorecido\";\"Município Favorecido\";\"Valor Recebido\"\n",
            "\"202341320018\";\"4132\";\"202306\";\"46634358000177\";\"MUNICIPIO DE SALTO\";\"Município\";\"Pessoa Jurídica\";\"SP\";\"SALTO\";\"100000,00\"\n",
            "\"202341320018\";\"4132\";\"202308\";\"46634358000177\";\"MUNICIPIO DE SALTO\";\"Município\";\"Pessoa Jurídica\";\"SP\";\"SALTO\";\"50000,00\"\n",
            "\"202341320018\";\"4132\";\"202307\";\"11222333000181\";\"GRAFICA EXEMPLO LTDA\";\"Sociedade Empresária Limitada\";\"Pessoa Jurídica\";\"SP\";\"ITU\";\"20000,00\"\n",
            "\"202341320018\";\"4132\";\"202307\";\"***550179**\";\"FULANO FICTICIO\";\"\";\"Pessoa Fisica\";\"SP\";\"ITU\";\"3000,00\"\n",
        )))
        .unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn amendments_are_grouped_with_their_agreements() {
        let dir = std::env::temp_dir().join(format!("voga-emendas-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let found = read(&zip(&dir), "4132").unwrap();
        assert_eq!(found.len(), 1);
        let a = &found[0];
        assert_eq!(a.places, ["SALTO - SP", "ITU - SP"]);
        assert_eq!(a.committed, 600000.0);
        assert_eq!(a.paid, 300000.0);
        assert!(a.agreements[0].contains("PREFEITURA DE SALTO") && a.agreements[0].contains("CONSTRUÇÃO DE CRECHE"));
        let text = text("Marina Exemplo", a);
        assert!(text.contains("Valor pago: R$ 300.000,00") && text.contains("Área: Educação / Educação infantil"));
        // Recipients by CNPJ, summed, largest first; people only counted
        assert_eq!(a.recipients.len(), 2);
        assert_eq!((a.recipients[0].cnpj.as_str(), a.recipients[0].amount, a.recipients[0].month.as_str()), ("46634358000177", 150000.0, "2023-08"));
        assert_eq!(a.people_paid, (1, 3000.0));
        assert!(text.contains("MUNICIPIO DE SALTO (CNPJ 46634358000177") && text.contains("Pessoas físicas (1): R$ 3.000,00"));
        assert!(!text.contains("FULANO") && !text.contains("550179"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
