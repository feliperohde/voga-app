//! Official open data: the Câmara dos Deputados, the Senado Federal, the TSE, the Portal da
//! Transparência, the ALESP, the PNCP and the states' own transparency portals. Each connector
//! turns what the institution publishes into documents (to search and ask) and into entities and
//! relations (the facts of the graph). Their facts are confirmed: they come from the institutions
//! themselves.
//!
//! A target is linked to its official ids (`targets.external_ids`); linking creates one source
//! per id, of one of the KINDS.

pub mod alesp;
pub mod camara;
pub mod estaduais;
pub mod pncp;
pub mod senado;
pub mod transferegov;
pub mod transparencia;
pub mod tse;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{fold, index_if_new, save_document, NewDocument, SourceRow};
use crate::rag::embeddings::EmbeddingProvider;

/// History is collected from the start of the current legislature (57th, 2023–2027).
pub const HISTORY_START: &str = "2023-02-01";

/// Document kinds that are records published by the institutions themselves: the primary
/// source. News, web pages, videos and posts are secondary: what others report.
pub const RECORD_KINDS: [&str; 11] = ["speech", "bill", "vote", "expense", "profile", "candidacy", "proposal", "amendment", "contract", "procurement", "gazette"];

pub fn is_record(kind: &str) -> bool {
    RECORD_KINDS.contains(&kind)
}

pub const KINDS: [&str; 7] = ["camara_deputado", "senado_senador", "tse_candidato", transparencia::KIND, alesp::KIND, pncp::KIND, estaduais::KIND];

/// A document an official source states (in Portuguese, without timed segments).
pub struct Doc<'a> {
    pub url: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub author: Option<&'a str>,
    pub site: Option<&'a str>,
    pub published_at: Option<&'a str>,
    pub text: &'a str,
    pub note: Option<&'a str>,
}

/// What a connector run works with.
pub struct Ctx<'a> {
    pub pool: &'a SqlitePool,
    pub app_data_dir: &'a Path,
    pub embedder: &'a dyn EmbeddingProvider,
    pub source: &'a SourceRow,
    /// The entity of the target's person (or organization).
    pub person: String,
    pub name: String,
    /// New documents saved in this run.
    pub saved: usize,
}

impl<'a> Ctx<'a> {
    pub async fn new(pool: &'a SqlitePool, app_data_dir: &'a Path, embedder: &'a dyn EmbeddingProvider, source: &'a SourceRow) -> Result<Ctx<'a>> {
        let target_id = source.target_id.as_deref().ok_or_else(|| anyhow!("Official sources belong to a target"))?;
        let (name, kind): (String, String) = sqlx::query_as("SELECT name, kind FROM targets WHERE id = ?")
            .bind(target_id)
            .fetch_one(pool)
            .await
            .context("The target of this source no longer exists")?;
        let entity_type = if kind == "person" { "person" } else { "organization" };
        let person = entity(pool, &source.investigation_id, entity_type, &format!("target:{target_id}"), &name, json!({}), None).await?;
        Ok(Ctx { pool, app_data_dir, embedder, source, person, name, saved: 0 })
    }

    pub fn investigation(&self) -> &str {
        &self.source.investigation_id
    }

    /// Saves (and indexes, when new or changed) a document of this source; returns its id.
    pub async fn document(&mut self, doc: Doc<'_>) -> Result<String> {
        let source = self.source;
        let url = doc.url.to_string();
        let saved = save_document(
            self.pool,
            NewDocument {
                investigation_id: &source.investigation_id,
                source_id: Some(&source.id),
                url: doc.url,
                kind: doc.kind,
                title: doc.title,
                author: doc.author,
                site: doc.site,
                published_at: doc.published_at,
                lang: Some("pt"),
                text: doc.text,
                segments: None,
                note: doc.note,
            },
        )
        .await?;
        if saved.is_some() {
            self.saved += 1;
        }
        index_if_new(self.pool, self.embedder, saved.clone()).await?;
        match saved {
            Some(id) => Ok(id),
            None => Ok(sqlx::query_as::<_, (String,)>("SELECT id FROM documents WHERE investigation_id = ? AND url = ?")
                .bind(self.investigation())
                .bind(&url)
                .fetch_one(self.pool)
                .await?
                .0),
        }
    }

    /// Adds what an official record says to the target's entity (its ids, state…).
    pub async fn describe_person(&self, attrs: Value, document_id: &str) -> Result<()> {
        sqlx::query("UPDATE entities SET attrs = json_patch(attrs, ?), document_id = COALESCE(document_id, ?) WHERE id = ?")
            .bind(attrs.to_string())
            .bind(document_id)
            .bind(&self.person)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    pub async fn entity(&self, entity_type: &str, key: &str, label: &str, attrs: Value, document_id: Option<&str>) -> Result<String> {
        entity(self.pool, self.investigation(), entity_type, key, label, attrs, document_id).await
    }

    pub async fn relate(&self, from: &str, to: &str, label: &str, date: &str, props: Value, document_id: Option<&str>) -> Result<()> {
        relate(self.pool, self.investigation(), from, to, label, date, props, document_id).await
    }

    /// The party entity, and the person's membership of it.
    pub async fn member_of(&self, party: &str, document_id: Option<&str>) -> Result<()> {
        let party = party.trim();
        if party.is_empty() {
            return Ok(());
        }
        let party_id = self.entity("party", &format!("party:{}", party.to_uppercase()), party, json!({}), None).await?;
        self.relate(&self.person.clone(), &party_id, "MEMBER_OF", "", json!({}), document_id).await
    }

    pub async fn cursor(&self, stream: &str) -> Result<Option<String>> {
        let (json,): (String,) = sqlx::query_as("SELECT cursor FROM sources WHERE id = ?").bind(&self.source.id).fetch_one(self.pool).await?;
        let map: HashMap<String, String> = serde_json::from_str(&json).unwrap_or_default();
        Ok(map.get(stream).cloned())
    }

    pub async fn set_cursor(&self, stream: &str, value: &str) -> Result<()> {
        let (json,): (String,) = sqlx::query_as("SELECT cursor FROM sources WHERE id = ?").bind(&self.source.id).fetch_one(self.pool).await?;
        let mut map: HashMap<String, String> = serde_json::from_str(&json).unwrap_or_default();
        map.insert(stream.to_string(), value.to_string());
        sqlx::query("UPDATE sources SET cursor = ? WHERE id = ?").bind(serde_json::to_string(&map)?).bind(&self.source.id).execute(self.pool).await?;
        Ok(())
    }
}

pub use crate::graph::{entity, relate};

/// A dump kept in `<app data>/bulk`: downloaded when missing, and checked again (conditional
/// GET) when `refresh` and the last check is older than a day.
pub async fn bulk_file(pool: &SqlitePool, app_data_dir: &Path, url: &str, refresh: bool) -> Result<PathBuf> {
    bulk_file_up_to(pool, app_data_dir, url, refresh, 200 * 1024 * 1024).await
}

/// `bulk_file` for a dump allowed to be larger (the user asked for it).
pub async fn bulk_file_up_to(pool: &SqlitePool, app_data_dir: &Path, url: &str, refresh: bool, max_bytes: u64) -> Result<PathBuf> {
    let name = url.rsplit('/').next().unwrap_or("file").to_string();
    bulk_file_at(pool, app_data_dir, url, &name, refresh, max_bytes).await
}

/// `bulk_file` kept as `name`: for dumps whose own file name another source already uses
/// (EmendasParlamentares.zip is both the Union's and Bahia's, and the disk may ignore case).
pub async fn bulk_file_named(pool: &SqlitePool, app_data_dir: &Path, url: &str, name: &str, refresh: bool) -> Result<PathBuf> {
    bulk_file_at(pool, app_data_dir, url, name, refresh, 200 * 1024 * 1024).await
}

async fn bulk_file_at(pool: &SqlitePool, app_data_dir: &Path, url: &str, name: &str, refresh: bool, max_bytes: u64) -> Result<PathBuf> {
    let path = app_data_dir.join("bulk").join(name);
    let row: Option<(Option<String>, Option<String>, String)> =
        sqlx::query_as("SELECT etag, last_modified, checked_at FROM bulk_files WHERE url = ?").bind(url).fetch_optional(pool).await?;
    let fresh = row
        .as_ref()
        .and_then(|(_, _, checked)| chrono::DateTime::parse_from_rfc3339(checked).ok())
        .is_some_and(|checked| Utc::now() - checked.with_timezone(&Utc) < chrono::Duration::hours(20));
    if path.exists() && (!refresh || fresh) {
        return Ok(path);
    }
    let (etag, modified) = match (&row, path.exists()) {
        (Some((etag, modified, _)), true) => (etag.clone(), modified.clone()),
        _ => (None, None),
    };
    log::info!("Official: checking {url}");
    let validators = match super::fetch::download(url, &path, (etag.as_deref(), modified.as_deref()), max_bytes).await? {
        Some(found) => found,
        None => (etag, modified),
    };
    sqlx::query(
        "INSERT INTO bulk_files (url, path, etag, last_modified, checked_at) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(url) DO UPDATE SET path = excluded.path, etag = excluded.etag, last_modified = excluded.last_modified, checked_at = excluded.checked_at",
    )
    .bind(url)
    .bind(path.to_string_lossy().as_ref())
    .bind(validators.0)
    .bind(validators.1)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(path)
}

/// One file out of a ZIP archive.
pub fn zip_entry(archive: &Path, name: &str) -> Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let mut file = match zip.by_name(name) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

// ===== One entry out of a remote ZIP (HTTP ranges) =====

fn u16_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as u64)
}
fn u32_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as u64)
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Where the central directory is, from the end of the archive: (offset, size), or Err(offset of
/// the ZIP64 end record) when the sizes do not fit 32 bits.
fn central_directory(tail: &[u8]) -> Result<std::result::Result<(u64, u64), u64>> {
    let at = (0..tail.len().saturating_sub(21)).rev().find(|&i| u32_at(tail, i) == Some(0x0605_4b50)).ok_or_else(|| anyhow!("Not a ZIP (no end of central directory)"))?;
    let (size, offset) = (u32_at(tail, at + 12).unwrap_or(0), u32_at(tail, at + 16).unwrap_or(0));
    if size != 0xFFFF_FFFF && offset != 0xFFFF_FFFF {
        return Ok(Ok((offset, size)));
    }
    let locator = at.checked_sub(20).filter(|&l| u32_at(tail, l) == Some(0x0706_4b50)).ok_or_else(|| anyhow!("ZIP64 locator missing"))?;
    Ok(Err(u64_at(tail, locator + 8).ok_or_else(|| anyhow!("Bad ZIP64 locator"))?))
}

#[derive(Debug, PartialEq)]
struct RemoteEntry {
    method: u64,
    compressed: u64,
    header: u64,
}

/// The PDFs of a candidate: `prefix`.pdf, or its parts `prefix`_01.pdf, _02… (the TSE's 2024
/// files), in order.
pub fn is_part(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|rest| rest == ".pdf" || (rest.starts_with('_') && rest.ends_with(".pdf")))
}

/// The entries of a central directory whose name `wanted` accepts, in order.
fn find_entries(cd: &[u8], wanted: impl Fn(&str) -> bool) -> Vec<(String, RemoteEntry)> {
    let mut found: Vec<(String, RemoteEntry)> = (|| {
        let mut found = Vec::new();
        let mut at = 0;
        while u32_at(cd, at) == Some(0x0201_4b50) {
            let (name_len, extra_len, comment_len) = (u16_at(cd, at + 28)? as usize, u16_at(cd, at + 30)? as usize, u16_at(cd, at + 32)? as usize);
            let name = String::from_utf8_lossy(cd.get(at + 46..at + 46 + name_len)?).into_owned();
            if wanted(&name) {
                found.push((name, entry_at(cd, at, name_len, extra_len)?));
            }
            at += 46 + name_len + extra_len + comment_len;
        }
        Some(found)
    })()
    .unwrap_or_default();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// The entry whose central-directory header is at `at`.
fn entry_at(cd: &[u8], at: usize, name_len: usize, extra_len: usize) -> Option<RemoteEntry> {
    let (uncompressed, mut compressed, mut header) = (u32_at(cd, at + 24)?, u32_at(cd, at + 20)?, u32_at(cd, at + 42)?);
    // ZIP64: the real values are in the extra field (0x0001), in this order, for those maxed out
    let extra = cd.get(at + 46 + name_len..at + 46 + name_len + extra_len)?;
    let mut e = 0;
    while e + 4 <= extra.len() {
        let (id, len) = (u16_at(extra, e)?, u16_at(extra, e + 2)? as usize);
        if id == 1 {
            let mut v = e + 4;
            for field in [None, Some(&mut compressed), Some(&mut header)] {
                let maxed = match &field {
                    None => uncompressed == 0xFFFF_FFFF,
                    Some(value) => **value == 0xFFFF_FFFF,
                };
                if maxed {
                    if let Some(value) = field {
                        *value = u64_at(extra, v)?;
                    }
                    v += 8;
                }
            }
        }
        e += 4 + len;
    }
    Some(RemoteEntry { method: u16_at(cd, at + 10)?, compressed, header })
}

/// An entry's bytes from its local header onwards (stored or deflated).
fn entry_bytes(local: &[u8], entry: &RemoteEntry) -> Result<Vec<u8>> {
    if u32_at(local, 0) != Some(0x0403_4b50) {
        bail!("Bad ZIP local header");
    }
    let start = 30 + u16_at(local, 26).unwrap_or(0) as usize + u16_at(local, 28).unwrap_or(0) as usize;
    let data = local.get(start..start + entry.compressed as usize).ok_or_else(|| anyhow!("ZIP entry cut short"))?;
    match entry.method {
        0 => Ok(data.to_vec()),
        8 => {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::DeflateDecoder::new(data), &mut out)?;
            Ok(out)
        }
        other => bail!("ZIP compression method {other} not supported"),
    }
}

/// The files of a ZIP on a server whose name `wanted` accepts, fetching only the archive's index
/// and those files (the server must serve byte ranges): a 6 MB PDF out of a 1.4 GB dump.
pub async fn remote_zip_entries(url: &str, wanted: impl Fn(&str) -> bool) -> Result<Vec<Vec<u8>>> {
    use super::fetch::get_range;
    let (tail, _) = get_range(url, "bytes=-65536").await?;
    let (cd_offset, cd_size) = match central_directory(&tail)? {
        Ok(found) => found,
        Err(zip64_end) => {
            let (record, _) = get_range(url, &format!("bytes={zip64_end}-{}", zip64_end + 55)).await?;
            (u64_at(&record, 48).ok_or_else(|| anyhow!("Bad ZIP64 end record"))?, u64_at(&record, 40).ok_or_else(|| anyhow!("Bad ZIP64 end record"))?)
        }
    };
    let (cd, _) = get_range(url, &format!("bytes={cd_offset}-{}", cd_offset + cd_size - 1)).await?;
    let mut files = Vec::new();
    for (_, entry) in find_entries(&cd, wanted) {
        // The local header repeats the name and may have its own extra field: read some slack
        let end = (entry.header + 30 + 65_535 * 2 + entry.compressed).min(cd_offset) - 1;
        let (local, _) = get_range(url, &format!("bytes={}-{end}", entry.header)).await?;
        files.push(entry_bytes(&local, &entry)?);
    }
    Ok(files)
}

/// The files of a local ZIP whose name `wanted` accepts, in order.
pub fn zip_entries(archive: &Path, wanted: impl Fn(&str) -> bool) -> Result<Vec<Vec<u8>>> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let mut names: Vec<String> = zip.file_names().filter(|n| wanted(n)).map(String::from).collect();
    names.sort();
    let mut files = Vec::new();
    for name in names {
        let mut bytes = Vec::new();
        zip.by_name(&name)?.read_to_end(&mut bytes)?;
        files.push(bytes);
    }
    Ok(files)
}

/// The TSE publishes Latin-1; each byte is its own character.
///
/// ponytail: Latin-1, not Windows-1252; the few 0x80–0x9F punctuation marks come out wrong.
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// A ';'-separated file with a header row, read as maps from column to value.
pub struct Table {
    columns: HashMap<String, usize>,
    rows: Vec<csv::StringRecord>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Table> {
        let text = text.trim_start_matches('\u{feff}');
        let mut reader = csv::ReaderBuilder::new().delimiter(b';').flexible(true).from_reader(text.as_bytes());
        let columns = reader.headers()?.iter().enumerate().map(|(i, h)| (h.trim().to_string(), i)).collect();
        let rows = reader.records().collect::<Result<Vec<_>, _>>()?;
        Ok(Table { columns, rows })
    }

    pub fn rows(&self) -> impl Iterator<Item = Row<'_>> {
        self.rows.iter().map(|record| Row { columns: &self.columns, record })
    }
}

pub struct Row<'a> {
    columns: &'a HashMap<String, usize>,
    record: &'a csv::StringRecord,
}

impl Row<'_> {
    /// The value of `column`, trimmed; '' when missing. TSE writes "#NULO" for none.
    pub fn get(&self, column: &str) -> &str {
        let value = self.columns.get(column).and_then(|&i| self.record.get(i)).unwrap_or("").trim();
        if value.starts_with("#NULO") || value == "#NE" {
            ""
        } else {
            value
        }
    }
}

/// Reads a ';'-separated Latin-1 file inside a ZIP row by row, without loading it whole
/// (some are hundreds of MB). `f` returns false to stop.
pub fn scan_zip_csv(archive: &Path, name: &str, f: impl FnMut(&Row) -> bool) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let file = zip.by_name(name).with_context(|| format!("{name} is not in {}", archive.display()))?;
    scan_csv(std::io::BufReader::new(file), latin1, f)
}

/// Most states publish UTF-8; a stray invalid byte becomes U+FFFD instead of failing the file.
pub fn utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Reads a ';'-separated file with a header row, row by row, each field read by `decode`.
/// `f` returns false to stop.
pub fn scan_csv(reader: impl std::io::Read, decode: fn(&[u8]) -> String, mut f: impl FnMut(&Row) -> bool) -> Result<()> {
    let mut reader = csv::ReaderBuilder::new().delimiter(b';').flexible(true).from_reader(reader);
    let decode = |record: &csv::ByteRecord| csv::StringRecord::from(record.iter().map(decode).collect::<Vec<_>>());
    let columns: HashMap<String, usize> = decode(reader.byte_headers()?).iter().enumerate().map(|(i, h)| (h.trim().trim_start_matches('\u{feff}').to_string(), i)).collect();
    let mut record = csv::ByteRecord::new();
    while reader.read_byte_record(&mut record)? {
        let decoded = decode(&record);
        if !f(&Row { columns: &columns, record: &decoded }) {
            break;
        }
    }
    Ok(())
}

/// Reads flat XML records (`<record><Field>value</Field>…</record>`, as the ALESP publishes)
/// one by one, without loading the file. `f` returns false to stop.
pub fn scan_xml(reader: impl std::io::BufRead, record: &str, mut f: impl FnMut(&HashMap<String, String>) -> bool) -> Result<()> {
    use quick_xml::events::Event;
    let mut xml = quick_xml::Reader::from_reader(reader);
    let mut buf = Vec::new();
    let mut fields: HashMap<String, String> = HashMap::new();
    let (mut inside, mut field, mut value) = (false, None::<String>, String::new());
    loop {
        match xml.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_string();
                if name == record {
                    inside = true;
                    fields.clear();
                } else if inside {
                    field = Some(name);
                    value.clear();
                }
            }
            Event::Text(t) if field.is_some() => value.push_str(&t.xml10_content()),
            Event::CData(t) if field.is_some() => value.push_str(&t.xml10_content()),
            // &amp; &lt; … and &#233; come as their own events
            Event::GeneralRef(r) if field.is_some() => match r.resolve_char_ref()? {
                Some(c) => value.push(c),
                None => value.push_str(match r.as_ref() {
                    "lt" => "<",
                    "gt" => ">",
                    "amp" => "&",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                }),
            },
            Event::End(e) => {
                let name = e.local_name();
                if name.as_ref() == record {
                    inside = false;
                    if !f(&fields) {
                        return Ok(());
                    }
                } else if let Some(done) = field.take() {
                    fields.insert(done, value.trim().to_string());
                }
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

/// "1.234,56" or "234.9" → 1234.56 / 234.9.
pub fn number(text: &str) -> f64 {
    let text = text.trim();
    let normalized = if text.contains(',') { text.replace('.', "").replace(',', ".") } else { text.to_string() };
    normalized.parse().unwrap_or(0.0)
}

/// R$ 1.234,56
pub fn brl(value: f64) -> String {
    let cents = (value * 100.0).round() as i64;
    let (units, cents) = (cents / 100, (cents % 100).abs());
    let digits = units.abs().to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(c);
    }
    format!("R$ {}{grouped},{cents:02}", if units < 0 { "-" } else { "" })
}

/// Runs an official source.
pub async fn run(pool: &SqlitePool, app_data_dir: &Path, embedder: &dyn EmbeddingProvider, source: &SourceRow) -> Result<usize> {
    let mut ctx = Ctx::new(pool, app_data_dir, embedder, source).await?;
    match source.kind.as_str() {
        "camara_deputado" => camara::run(&mut ctx).await?,
        "senado_senador" => senado::run(&mut ctx).await?,
        "tse_candidato" => tse::run(&mut ctx).await?,
        transparencia::KIND => transparencia::run(&mut ctx).await?,
        alesp::KIND => alesp::run(&mut ctx).await?,
        pncp::KIND => pncp::run(&mut ctx).await?,
        estaduais::KIND => estaduais::run(&mut ctx).await?,
        other => anyhow::bail!("Unknown official source: {other}"),
    }
    Ok(ctx.saved)
}

// ===== Linking a target to its official ids =====

/// Someone in the official records who may be the target.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OfficialMatch {
    /// camara_deputado | senado_senador | tse_candidato | transparencia_emendas | alesp_deputado | pncp_prefeitura | emendas_estaduais
    pub kind: String,
    /// What the source keeps: the deputy id, the senator code, "year:UF:candidate id", the city's IBGE code,
    /// "UF:author" for a state's amendments…
    pub id: String,
    pub label: String,
    pub detail: String,
    /// Only shares words with the name searched (another first or last name): most likely
    /// someone else, shown apart.
    #[serde(default)]
    pub similar: bool,
}

/// Whether every word of `name` is in `candidate` (both folded): "Tabata Amaral" matches
/// "TABATA CLAUDIA AMARAL DE PONTES".
pub fn name_matches(name: &str, candidate: &str) -> bool {
    let candidate = fold(candidate);
    let words: Vec<&str> = candidate.split_whitespace().collect();
    let wanted = fold(name);
    let wanted: Vec<&str> = wanted.split_whitespace().filter(|w| w.len() > 2).collect();
    !wanted.is_empty() && wanted.iter().all(|w| words.contains(w))
}

/// Whether `candidate` is the name searched, not just someone sharing some of its words: the
/// same words, or the same first and last name ("Ricardo Nunes" is "RICARDO LUIS REIS NUNES",
/// not "RICARDO NUNES ARRUDA").
pub fn same_name(name: &str, candidate: &str) -> bool {
    let (wanted, candidate) = (fold(name), fold(candidate));
    let wanted: Vec<&str> = wanted.split_whitespace().collect();
    let words: Vec<&str> = candidate.split_whitespace().collect();
    wanted == words || (!wanted.is_empty() && wanted.first() == words.first() && wanted.last() == words.last())
}

/// Searches the Câmara, the Senado, the ALESP, the Portal da Transparência, the states'
/// amendments and the TSE (general elections of `years`) for the name.
pub async fn find_matches(pool: &SqlitePool, app_data_dir: &Path, name: &str, years: &[i32]) -> Result<Vec<OfficialMatch>> {
    let mut found = Vec::new();
    match camara::find(name).await {
        Ok(matches) => found.extend(matches),
        Err(e) => log::warn!("Official: Câmara search failed: {e}"),
    }
    match senado::find(name).await {
        Ok(matches) => found.extend(matches),
        Err(e) => log::warn!("Official: Senado search failed: {e}"),
    }
    match alesp::find(pool, app_data_dir, name).await {
        Ok(matches) => found.extend(matches),
        Err(e) => log::warn!("Official: ALESP search failed: {e}"),
    }
    match transparencia::find(pool, app_data_dir, name).await {
        Ok(matches) => found.extend(matches),
        Err(e) => log::warn!("Official: Portal da Transparência search failed: {e}"),
    }
    // Each state on its own: one portal down leaves the others
    found.extend(estaduais::find(pool, app_data_dir, name).await);
    // The TSE's own matches know the full name too (tse::find)
    for m in &mut found {
        m.similar = !same_name(name, &m.label);
    }
    for &year in years {
        match tse::find(pool, app_data_dir, name, year).await {
            Ok(matches) => found.extend(matches),
            Err(e) => log::warn!("Official: TSE {year} search failed: {e}"),
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_name_tells_the_person_from_people_sharing_a_word() {
        assert!(same_name("Ricardo Nunes", "RICARDO NUNES"));
        assert!(same_name("Ricardo Nunes", "Ricardo Luís Reis Nunes"));
        assert!(!same_name("Ricardo Nunes", "RICARDO ARRUDA"));
        assert!(!same_name("Ricardo Nunes", "DELEGADO ANTONIO RICARDO"));
        assert!(same_name("Tarcísio", "TARCÍSIO"));
    }

    #[test]
    fn one_entry_is_read_out_of_a_zip_by_its_parts() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("SP/2024SP1.pdf", deflated).unwrap();
        zip.write_all(&b"outro plano ".repeat(100)).unwrap();
        zip.start_file("SP/2024SP2_01.pdf", deflated).unwrap();
        zip.write_all(&b"plano de governo da cidade ".repeat(200)).unwrap();
        zip.start_file("SP/2024SP3.pdf", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
        zip.write_all(b"guardado sem compressao").unwrap();
        let bytes = zip.finish().unwrap().into_inner();

        let (offset, size) = central_directory(&bytes).unwrap().unwrap();
        let cd = &bytes[offset as usize..(offset + size) as usize];
        assert!(find_entries(cd, |n| is_part(n, "SP/2024SP9")).is_empty());
        // "SP/2024SP2" is not a part of "SP/2024SP"
        assert!(find_entries(cd, |n| is_part(n, "SP/2024SP")).is_empty());
        for (prefix, expected) in [("SP/2024SP2", b"plano de governo da cidade ".repeat(200)), ("SP/2024SP3", b"guardado sem compressao".to_vec())] {
            let found = find_entries(cd, |n| is_part(n, prefix));
            assert_eq!(found.len(), 1);
            let entry = &found[0].1;
            assert_eq!(entry_bytes(&bytes[entry.header as usize..], entry).unwrap(), expected);
        }
    }

    #[test]
    fn flat_xml_records() {
        let xml = "<?xml version=\"1.0\"?><list><item><Id>1</Id><Nome>Jo&#227;o &amp; Maria</Nome><Bio>&lt;p&gt;x&lt;/p&gt;</Bio></item><item><Id>2</Id><Nome/></item></list>";
        let mut seen = Vec::new();
        scan_xml(xml.as_bytes(), "item", |r| {
            seen.push((r.get("Id").cloned().unwrap_or_default(), r.get("Nome").cloned().unwrap_or_default(), r.get("Bio").cloned()));
            true
        })
        .unwrap();
        assert_eq!(seen[0], ("1".into(), "João & Maria".into(), Some("<p>x</p>".into())));
        assert_eq!(seen[1].0, "2");
    }

    #[test]
    fn numbers_and_money() {
        assert_eq!(number("1.234,56"), 1234.56);
        assert_eq!(number("234.9"), 234.9);
        assert_eq!(number(""), 0.0);
        assert_eq!(brl(1234567.891), "R$ 1.234.567,89");
        assert_eq!(brl(70000.0), "R$ 70.000,00");
        assert_eq!(brl(0.5), "R$ 0,50");
    }

    #[test]
    fn names_match_word_by_word() {
        assert!(name_matches("Tábata Amaral", "TABATA CLAUDIA AMARAL DE PONTES"));
        assert!(!name_matches("Tabata Amaral", "TABATA SILVA"));
        assert!(!name_matches("de", "MARIA DE SOUZA"));
    }

    #[test]
    fn tables_read_latin1_and_tse_nulls() {
        let text = latin1(b"\"NM_CANDIDATO\";\"DS_CARGO\";\"NM_SOCIAL\"\n\"JO\xc3O\";\"DEPUTADO FEDERAL\";\"#NULO\"\n");
        let table = Table::parse(&text).unwrap();
        let row = table.rows().next().unwrap();
        assert_eq!(row.get("NM_CANDIDATO"), "JOÃO");
        assert_eq!(row.get("NM_SOCIAL"), "");
        assert_eq!(row.get("MISSING"), "");
    }
}
