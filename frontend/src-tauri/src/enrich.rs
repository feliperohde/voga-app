//! Free public data, no key needed, about the companies in the graph (`cnpj:` organizations):
//! the Receita record and its partners (minhareceita.org, an open mirror of the Receita's CNPJ
//! data), the federal sanction lists CEIS, CNEP and CEPIM (non-profits barred from federal
//! agreements; CGU open data, one ZIP a day) and the São Paulo state's sanctions (its
//! transparency portal). The worker fills in what is missing when it is idle; the Map asks again
//! for one company.
//!
//! The Receita publishes partners' CPFs masked (***123456**); even those digits are never stored.

use anyhow::Result;
use chrono::{Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::collect::fetch;
use crate::collect::official::{bulk_file, bulk_file_named, scan_zip_csv, Row};
use crate::graph::{entity, relate};

/// More partners than this (banks, large S.A.s with dozens of directors) are only counted.
/// ponytail: keeps the Map readable; list them behind a toggle if someone needs them.
const MAX_PARTNERS: usize = 10;
const COMPANIES_PER_STEP: i64 = 10;
const SANCTION_LISTS: [&str; 3] = ["ceis", "cnep", "cepim"];
/// The São Paulo state's sanctions (e-Sanções, and SICAF's under the 2021 law), refreshed daily.
const SP_SANCTIONS: &str = "https://www.transparencia.sp.gov.br/Dados%20Abertos/Sancoes/Sancoes.zip";
/// The day the sanction lists were last checked against every company.
const SANCTIONS_PREF: &str = "sanctions_checked_on";

/// Fetches a company's Receita record and adds it to every investigation that has the company.
pub async fn company(pool: &SqlitePool, cnpj: &str) -> Result<()> {
    let record: Value = fetch::get_json(&format!("https://minhareceita.org/{cnpj}")).await?;
    apply_company(pool, cnpj, &record).await
}

async fn apply_company(pool: &SqlitePool, cnpj: &str, record: &Value) -> Result<()> {
    let partners = record["qsa"].as_array().cloned().unwrap_or_default();
    let attrs = json!({
        "razao_social": record["razao_social"],
        "situacao": record["descricao_situacao_cadastral"],
        "cnae": record["cnae_fiscal_descricao"],
        "municipio": record["municipio"],
        "uf": record["uf"],
        "abertura": record["data_inicio_atividade"],
        "capital_social": record["capital_social"],
        "socios": partners.len(),
        "receita_at": Utc::now().to_rfc3339(),
    });
    let companies: Vec<(String, String)> =
        sqlx::query_as("SELECT id, investigation_id FROM entities WHERE key = ?").bind(format!("cnpj:{cnpj}")).fetch_all(pool).await?;
    for (company_id, investigation_id) in companies {
        sqlx::query("UPDATE entities SET attrs = json_patch(attrs, ?) WHERE id = ?").bind(attrs.to_string()).bind(&company_id).execute(pool).await?;
        if partners.len() > MAX_PARTNERS {
            continue;
        }
        for partner in &partners {
            let name = partner["nome_socio"].as_str().unwrap_or("").trim();
            if name.is_empty() {
                continue;
            }
            let document = partner["cnpj_cpf_do_socio"].as_str().unwrap_or("");
            let digits: String = document.chars().filter(char::is_ascii_digit).collect();
            let partner_id = if digits.len() == 14 {
                entity(pool, &investigation_id, "organization", &format!("cnpj:{digits}"), name, json!({"cnpj": digits}), None).await?
            } else {
                entity(pool, &investigation_id, "person", &partner_key(name, document), name, json!({}), None).await?
            };
            let since = partner["data_entrada_sociedade"].as_str().unwrap_or("");
            relate(pool, &investigation_id, &partner_id, &company_id, "PARTNER_OF", since, json!({"role": partner["qualificacao_socio"]}), None).await?;
        }
    }
    Ok(())
}

/// A partner who is a person: name and masked document, hashed, so the same partner links
/// across companies without keeping any CPF digits.
fn partner_key(name: &str, masked: &str) -> String {
    let digest = Sha256::digest(format!("{}|{masked}", name.to_uppercase()).as_bytes());
    format!("socio:{:x}", digest)[..22].to_string()
}

/// The newest dump of a sanction list: today's appears during the day, so up to a week back.
/// Older dumps are deleted.
async fn latest_list(pool: &SqlitePool, app_data_dir: &Path, list: &str) -> Result<(NaiveDate, PathBuf)> {
    let suffix = format!("_{}.zip", list.to_uppercase());
    let today = Utc::now().date_naive();
    let mut last_error = None;
    for back in 0..7 {
        let date = today - Duration::days(back);
        let name = format!("{}{suffix}", date.format("%Y%m%d"));
        let url = format!("https://dadosabertos-download.cgu.gov.br/PortalDaTransparencia/saida/{list}/{name}");
        match bulk_file(pool, app_data_dir, &url, false).await {
            Ok(path) => {
                for old in std::fs::read_dir(app_data_dir.join("bulk"))?.flatten() {
                    let file = old.file_name().to_string_lossy().to_string();
                    if file.ends_with(&suffix) && file != name {
                        std::fs::remove_file(old.path()).ok();
                    }
                }
                return Ok((date, path));
            }
            Err(e) => last_error = Some(e),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No {list} list")))
}

fn digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// A company's sanction in a row of CEIS, CNEP, CEPIM or the São Paulo list, told apart by their
/// columns. Rows about people (by CPF) are None.
fn sanction(row: &Row) -> Option<(String, Value)> {
    let cepim = digits(row.get("CNPJ ENTIDADE"));
    if !cepim.is_empty() {
        let agreement = row.get("NÚMERO CONVÊNIO");
        return Some((
            cepim,
            json!({"cadastro": "CEPIM", "categoria": row.get("MOTIVO DO IMPEDIMENTO"), "orgao": row.get("ÓRGÃO CONCEDENTE"), "inicio": "", "fim": "", "processo": if agreement.is_empty() { String::new() } else { format!("Convênio {agreement}") }}),
        ));
    }
    let sp = digits(row.get("CPF/CNPJ"));
    if !sp.is_empty() {
        let date = |column: &str| Some(row.get(column)).filter(|d| *d != "Nao Informado").unwrap_or("").to_string();
        return (sp.len() == 14).then(|| {
            (sp, json!({"cadastro": format!("Estado de SP ({})", row.get("Origem da Informacao")), "categoria": row.get("Tipo de Sancao"), "orgao": row.get("Nome UGE"), "inicio": date("Data Inicio"), "fim": date("Data Final"), "processo": ""}))
        });
    }
    (row.get("TIPO DE PESSOA") == "J").then(|| {
        (
            digits(row.get("CPF OU CNPJ DO SANCIONADO")),
            json!({
                "cadastro": row.get("CADASTRO"),
                "categoria": row.get("CATEGORIA DA SANÇÃO"),
                "orgao": row.get("ÓRGÃO SANCIONADOR"),
                "inicio": row.get("DATA INÍCIO SANÇÃO"),
                "fim": row.get("DATA FINAL SANÇÃO"),
                "processo": row.get("NÚMERO DO PROCESSO"),
            }),
        )
    })
}

/// The sanctions of the companies in `wanted`, from one list's file.
fn sanctions_in(archive: &Path, entry: &str, wanted: &HashSet<String>, found: &mut HashMap<String, Vec<Value>>) -> Result<()> {
    scan_zip_csv(archive, entry, |row| {
        if let Some((cnpj, sanction)) = sanction(row).filter(|(cnpj, _)| wanted.contains(cnpj)) {
            found.entry(cnpj).or_default().push(sanction);
        }
        true
    })
}

/// Checks every company against the newest CEIS, CNEP, CEPIM and São Paulo list: once a day, or
/// sooner when there are companies never checked (or `force`).
pub async fn sanctions(pool: &SqlitePool, app_data_dir: &Path, force: bool) -> Result<()> {
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let (unchecked,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM entities WHERE key LIKE 'cnpj:%' AND json_extract(attrs, '$.sanctions_checked') IS NULL").fetch_one(pool).await?;
    if !force && unchecked == 0 && crate::prefs::get(pool, SANCTIONS_PREF).await?.as_deref() == Some(today.as_str()) {
        return Ok(());
    }
    let wanted: HashSet<String> = sqlx::query_as::<_, (String,)>("SELECT DISTINCT substr(key, 6) FROM entities WHERE key LIKE 'cnpj:%'")
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(cnpj,)| cnpj)
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let mut files = Vec::new();
    let mut checked = today.clone();
    for list in SANCTION_LISTS {
        let (date, path) = latest_list(pool, app_data_dir, list).await?;
        files.push((path, format!("{}_{}.csv", date.format("%Y%m%d"), list.to_uppercase())));
        checked = checked.min(date.format("%Y-%m-%d").to_string());
    }
    files.push((bulk_file_named(pool, app_data_dir, SP_SANCTIONS, "sp-sancoes.zip", true).await?, "Sancoes.csv".to_string()));
    let scan = wanted.clone();
    let found = tokio::task::spawn_blocking(move || -> Result<HashMap<String, Vec<Value>>> {
        let mut found = HashMap::new();
        for (path, entry) in &files {
            sanctions_in(path, entry, &scan, &mut found)?;
        }
        Ok(found)
    })
    .await??;
    for cnpj in &wanted {
        let attrs = json!({"sanctions": found.get(cnpj).cloned().unwrap_or_default(), "sanctions_checked": checked});
        sqlx::query("UPDATE entities SET attrs = json_patch(attrs, ?) WHERE key = ?").bind(attrs.to_string()).bind(format!("cnpj:{cnpj}")).execute(pool).await?;
    }
    crate::prefs::set(pool, SANCTIONS_PREF, &today).await?;
    Ok(())
}

/// Companies whose Receita record was never fetched. Companies that are only someone's partner
/// wait for the Map's button: holdings can chain far.
async fn due(pool: &SqlitePool) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT substr(e.key, 6) FROM entities e
         WHERE e.key LIKE 'cnpj:%' AND json_extract(e.attrs, '$.receita_at') IS NULL
           AND EXISTS (SELECT 1 FROM relations r WHERE (r.to_id = e.id OR r.from_id = e.id) AND NOT (r.from_id = e.id AND r.label = 'PARTNER_OF'))
         LIMIT ?",
    )
    .bind(COMPANIES_PER_STEP)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(cnpj,)| cnpj).collect())
}

/// The worker's turn when idle: a few companies' Receita records, then the sanction lists.
pub async fn step(pool: &SqlitePool, app_data_dir: &Path) {
    let companies = due(pool).await.unwrap_or_else(|e| {
        log::warn!("Enrich: {e}");
        Vec::new()
    });
    for cnpj in companies {
        if let Err(e) = company(pool, &cnpj).await {
            let message = e.to_string();
            // Offline or the service down: try again on a later turn
            if !message.contains("answered HTTP 4") {
                log::warn!("Enrich: CNPJ {cnpj}: {message}");
                break;
            }
            // Unknown to the Receita: not asked again unless someone presses the Map's button
            let mark = json!({"receita_at": Utc::now().to_rfc3339(), "receita_error": message});
            sqlx::query("UPDATE entities SET attrs = json_patch(attrs, ?) WHERE key = ?").bind(mark.to_string()).bind(format!("cnpj:{cnpj}")).execute(pool).await.ok();
        }
    }
    if let Err(e) = sanctions(pool, app_data_dir, false).await {
        log::warn!("Enrich: sanctions: {e}");
    }
}

/// The Map's "public data" button for one company.
#[tauri::command]
pub async fn enrich_entity<R: tauri::Runtime>(app: tauri::AppHandle<R>, state: tauri::State<'_, crate::state::AppState>, entity_id: String) -> Result<(), String> {
    use tauri::Manager;
    let pool = state.db_manager.pool();
    let (attrs,): (String,) = sqlx::query_as("SELECT attrs FROM entities WHERE id = ?").bind(&entity_id).fetch_one(pool).await.map_err(|e| e.to_string())?;
    let cnpj = serde_json::from_str::<Value>(&attrs).ok().and_then(|a| a["cnpj"].as_str().map(String::from)).ok_or("This node has no CNPJ")?;
    company(pool, &cnpj).await.map_err(|e| e.to_string())?;
    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    sanctions(pool, &app_data_dir, true).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn partners_are_linked_without_cpf_digits_and_only_companies_are_matched_to_sanctions() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'T', '2026')").execute(&pool).await.unwrap();
        let person = entity(&pool, "inv", "person", "target:t1", "Ana Lima", json!({}), None).await.unwrap();
        let supplier = entity(&pool, "inv", "organization", "cnpj:11222333000181", "Gráfica Aurora", json!({"cnpj": "11222333000181"}), None).await.unwrap();
        relate(&pool, "inv", &person, &supplier, "SPENT_WITH", "2026-01", json!({"amount": 10.0}), None).await.unwrap();
        assert_eq!(due(&pool).await.unwrap(), vec!["11222333000181"]);

        let record = json!({
            "razao_social": "GRAFICA AURORA LTDA", "descricao_situacao_cadastral": "ATIVA", "uf": "SP",
            "qsa": [
                {"nome_socio": "BRUNO FICTICIO", "cnpj_cpf_do_socio": "***550179**", "qualificacao_socio": "Sócio-Administrador", "data_entrada_sociedade": "2020-01-02"},
                {"nome_socio": "HOLDING EXEMPLO SA", "cnpj_cpf_do_socio": "99888777000166", "qualificacao_socio": "Sócio", "data_entrada_sociedade": "2021-05-06"}
            ]
        });
        apply_company(&pool, "11222333000181", &record).await.unwrap();

        let rows: Vec<(String, String, String)> = sqlx::query_as("SELECT key, label, attrs FROM entities").fetch_all(&pool).await.unwrap();
        assert!(rows.iter().all(|(key, label, attrs)| !format!("{key}{label}{attrs}").contains("550179")), "CPF digits must not be stored");
        assert!(rows.iter().any(|(key, label, _)| key.starts_with("socio:") && label == "BRUNO FICTICIO"));
        let (attrs,): (String,) = sqlx::query_as("SELECT attrs FROM entities WHERE id = ?").bind(&supplier).fetch_one(&pool).await.unwrap();
        assert!(attrs.contains("GRAFICA AURORA LTDA") && attrs.contains("receita_at"));
        let (partners,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM relations WHERE label = 'PARTNER_OF' AND to_id = ?").bind(&supplier).fetch_one(&pool).await.unwrap();
        assert_eq!(partners, 2);
        // The holding is only a partner: the worker leaves it for the Map's button
        assert!(due(&pool).await.unwrap().is_empty());

        let archive = std::env::temp_dir().join(format!("voga-ceis-{}.zip", uuid::Uuid::new_v4()));
        {
            use std::io::Write;
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            zip.start_file("20261014_CEIS.csv", zip::write::SimpleFileOptions::default()).unwrap();
            let csv = "\"CADASTRO\";\"TIPO DE PESSOA\";\"CPF OU CNPJ DO SANCIONADO\";\"CATEGORIA DA SANÇÃO\";\"ÓRGÃO SANCIONADOR\"\n\
                       \"CEIS\";\"F\";\"11222333000181\";\"Pessoa\";\"X\"\n\
                       \"CEIS\";\"J\";\"11.222.333/0001-81\";\"Inidoneidade\";\"Prefeitura Exemplo\"\n";
            let latin1: Vec<u8> = csv.chars().map(|c| c as u32 as u8).collect();
            zip.write_all(&latin1).unwrap();
            zip.finish().unwrap();
        }
        let wanted: HashSet<String> = ["11222333000181".to_string()].into();
        let mut found = HashMap::new();
        sanctions_in(&archive, "20261014_CEIS.csv", &wanted, &mut found).unwrap();
        std::fs::remove_file(&archive).ok();
        let found = &found["11222333000181"];
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["orgao"], "Prefeitura Exemplo");
        assert_eq!(found[0]["categoria"], "Inidoneidade");
    }

    #[test]
    fn cepim_and_sao_paulo_rows_are_company_sanctions() {
        use std::io::Write;
        let archive = std::env::temp_dir().join(format!("voga-sanctions-{}.zip", uuid::Uuid::new_v4()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        let latin1 = |s: &str| s.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>();
        zip.start_file("20261027_CEPIM.csv", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&latin1("\"CNPJ ENTIDADE\";\"NOME ENTIDADE\";\"NÚMERO CONVÊNIO\";\"ÓRGÃO CONCEDENTE\";\"MOTIVO DO IMPEDIMENTO\"\n\"11222333000181\";\"ASSOCIACAO EXEMPLO\";\"619481\";\"Ministério da Cultura\";\"OMISSAO NO DEVER DE PRESTAR CONTAS\"\n")).unwrap();
        zip.start_file("Sancoes.csv", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&latin1(
            "CPF/CNPJ;Nome ou Razao Social;Tipo de Sancao;Data Inicio;Data Final;Nome UGE;Origem da Informacao\r\n\
             11222333000181;ASSOCIACAO EXEMPLO;Suspensão Temporária;Nao Informado;Nao Informado;CIA. DO METRO;e-Sanções\r\n\
             12345678901;FULANO FICTICIO;Multa;01/01/2026;Nao Informado;X;e-Sanções\r\n",
        ))
        .unwrap();
        zip.finish().unwrap();
        let wanted: HashSet<String> = ["11222333000181".to_string()].into();
        let mut found = HashMap::new();
        sanctions_in(&archive, "20261027_CEPIM.csv", &wanted, &mut found).unwrap();
        sanctions_in(&archive, "Sancoes.csv", &wanted, &mut found).unwrap();
        std::fs::remove_file(&archive).ok();
        let found = &found["11222333000181"];
        assert_eq!(found.len(), 2);
        assert_eq!((found[0]["cadastro"].as_str(), found[0]["processo"].as_str()), (Some("CEPIM"), Some("Convênio 619481")));
        assert_eq!((found[1]["cadastro"].as_str(), found[1]["categoria"].as_str(), found[1]["inicio"].as_str()), (Some("Estado de SP (e-Sanções)"), Some("Suspensão Temporária"), Some("")));
    }
}
