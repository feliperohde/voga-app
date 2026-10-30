//! Collection: runs sources (a page, a feed, a channel, a video, a news or video search),
//! puts what they list in the crawl queue, fetches it and indexes the documents.

pub mod extract;
pub mod feeds;
pub mod fetch;
pub mod gazettes;
pub mod official;
pub mod schedule;
pub mod scheduler;
pub mod thumbs;
pub mod youtube;

use anyhow::{bail, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::path::Path;

use crate::rag::embeddings::EmbeddingProvider;
use fetch::FetchResult;

/// Videos taken from each YouTube search run.
const VIDEOS_PER_SEARCH: usize = 10;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SourceRow {
    pub id: String,
    pub investigation_id: String,
    pub target_id: Option<String>,
    pub kind: String,
    pub url: String,
    pub title: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// The links of what it brings are followed (discover::links).
    #[sqlx(default)]
    pub follow_links: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct QueueItem {
    pub id: i64,
    pub investigation_id: String,
    pub source_id: Option<String>,
    pub url: String,
    pub kind: String,
    pub title: Option<String>,
    pub published_at: Option<String>,
    pub attempts: i64,
    /// feed | search | link
    #[sqlx(default)]
    pub via: String,
    /// Links away from its source.
    #[sqlx(default)]
    pub depth: i64,
}

/// Now, as stored in the scheduling columns (next_run_at, next_attempt_at), which are
/// compared as text: always this one format.
pub fn schedule_time(at: chrono::DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Lowercase without accents, for matching names in text.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Whether `phrase` occurs in `text` as whole words (both already folded).
pub fn contains_phrase(text: &str, phrase: &str) -> bool {
    !phrase.is_empty()
        && text.match_indices(phrase).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + phrase.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
}

/// The address a document is kept under: no fragment, no tracking parameters.
pub fn canonical_url(url: &str) -> String {
    if let Some(video) = youtube::canonical_video_url(url) {
        return video;
    }
    let Ok(mut parsed) = url::Url::parse(url) else { return url.to_string() };
    parsed.set_fragment(None);
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_") && !matches!(k.as_ref(), "fbclid" | "gclid" | "ocid" | "cmpid"))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        parsed.query_pairs_mut().clear().extend_pairs(kept);
    }
    parsed.to_string()
}

fn content_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// The names a target goes by, folded, for telling whether a found page is about it.
async fn target_names(pool: &SqlitePool, target_id: &str) -> Result<Vec<String>> {
    // A topic of an interest ("tratamento diabetes tipo 1") is searched as loose words: relevance
    // sorts what its searches bring, not its exact name
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT t.name, t.aliases FROM targets t JOIN investigations i ON i.id = t.investigation_id
         WHERE t.id = ? AND NOT (t.kind = 'topic' AND i.kind = 'interest')",
    )
    .bind(target_id)
    .fetch_optional(pool)
    .await?;
    let Some((name, aliases)) = row else { return Ok(Vec::new()) };
    let aliases: Vec<String> = serde_json::from_str(&aliases).unwrap_or_default();
    Ok(std::iter::once(name).chain(aliases).map(|n| fold(n.trim())).filter(|n| !n.is_empty()).collect())
}

/// The names of everyone and everything an investigation follows, folded.
pub async fn investigation_names(pool: &SqlitePool, investigation_id: &str) -> Result<Vec<String>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT name, aliases FROM targets WHERE investigation_id = ?").bind(investigation_id).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .flat_map(|(name, aliases)| std::iter::once(name).chain(serde_json::from_str::<Vec<String>>(&aliases).unwrap_or_default()))
        .map(|n| fold(n.trim()))
        .filter(|n| !n.is_empty())
        .collect())
}

pub struct NewDocument<'a> {
    pub investigation_id: &'a str,
    pub source_id: Option<&'a str>,
    pub url: &'a str,
    pub kind: &'a str,
    pub title: &'a str,
    pub author: Option<&'a str>,
    pub site: Option<&'a str>,
    pub published_at: Option<&'a str>,
    pub lang: Option<&'a str>,
    pub text: &'a str,
    pub segments: Option<String>,
    pub note: Option<&'a str>,
}

/// Inserts the document, or updates it when its text changed. Some(id) when there is
/// something new to index; None when it was already there as is.
pub async fn save_document(pool: &SqlitePool, doc: NewDocument<'_>) -> Result<Option<String>> {
    let hash = content_hash(doc.text);
    let existing: Option<(String, String)> = sqlx::query_as("SELECT id, content_hash FROM documents WHERE investigation_id = ? AND url = ?")
        .bind(doc.investigation_id)
        .bind(doc.url)
        .fetch_optional(pool)
        .await?;
    if existing.as_ref().is_some_and(|(_, h)| *h == hash) {
        return Ok(None);
    }
    let id = existing.map(|(id, _)| id).unwrap_or_else(|| format!("doc-{}", uuid::Uuid::new_v4()));
    sqlx::query(
        "INSERT INTO documents (id, investigation_id, source_id, url, kind, title, author, site, published_at, fetched_at, lang, text, segments, note, content_hash)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET title = excluded.title, author = excluded.author, site = excluded.site,
            published_at = excluded.published_at, fetched_at = excluded.fetched_at, lang = excluded.lang, text = excluded.text,
            segments = excluded.segments, note = excluded.note, content_hash = excluded.content_hash, indexed_at = NULL",
    )
    .bind(&id)
    .bind(doc.investigation_id)
    .bind(doc.source_id)
    .bind(doc.url)
    .bind(doc.kind)
    .bind(doc.title)
    .bind(doc.author)
    .bind(doc.site)
    .bind(doc.published_at)
    .bind(Utc::now().to_rfc3339())
    .bind(doc.lang)
    .bind(doc.text)
    .bind(doc.segments)
    .bind(doc.note)
    .bind(hash)
    .execute(pool)
    .await?;
    Ok(Some(id))
}

/// Keeps the picture a document came with, the first one seen (collect::thumbs fetches it).
async fn remember_image(pool: &SqlitePool, investigation_id: &str, url: &str, image: Option<&str>) -> Result<()> {
    if let Some(image) = image {
        sqlx::query("UPDATE documents SET image_url = ? WHERE investigation_id = ? AND url = ? AND image_url IS NULL")
            .bind(image)
            .bind(investigation_id)
            .bind(url)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// What a feed or a search said about an item before it is fetched.
#[derive(Default)]
pub struct Hints<'a> {
    pub title: Option<&'a str>,
    pub published: Option<&'a str>,
    /// Folded names the text must mention (items found by a target's searches).
    pub must_mention: &'a [String],
}

/// A fetched page: the document saved (None when nothing changed) and the page's links.
pub struct Ingested {
    pub saved: Option<String>,
    /// Where the page ended up (after redirects).
    pub url: String,
    pub links: Vec<extract::Link>,
}

/// Fetches a web page and saves its readable text as `kind` (article or page).
pub async fn ingest_page(pool: &SqlitePool, investigation_id: &str, source_id: Option<&str>, url: &str, kind: &str, hints: Hints<'_>) -> Result<Ingested> {
    let page = fetch::get_body(url).await?;
    let content_type = page.content_type.to_ascii_lowercase();
    let body = String::from_utf8_lossy(&page.body);
    let is_html = content_type.contains("html") || content_type.is_empty();
    let extracted = if is_html {
        extract::article(&body, &page.url)
    } else if content_type.starts_with("text/") {
        extract::Extracted { title: String::new(), author: None, site: None, published: None, lang: None, text: body.trim().to_string(), image: None }
    } else if content_type.contains("pdf") {
        let title = page.url.rsplit('/').next().unwrap_or("documento.pdf").to_string();
        extract::Extracted { title, author: None, site: None, published: None, lang: None, text: extract::pdf_text(page.body.clone()).await?, image: None }
    } else {
        bail!("Unsupported content type: {content_type}");
    };
    if extracted.text.trim().is_empty() {
        bail!("No readable text on the page");
    }
    if !hints.must_mention.is_empty() {
        let text = fold(&format!("{} {}", extracted.title, extracted.text));
        if !hints.must_mention.iter().any(|name| contains_phrase(&text, name)) {
            bail!("skip: the page does not mention the target");
        }
    }
    let title = Some(extracted.title.as_str()).filter(|t| !t.is_empty()).or(hints.title).unwrap_or(url);
    let published = extracted.published.as_deref().or(hints.published);
    let url = canonical_url(&page.url);
    let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()));
    let links = if is_html { extract::links(&body, &page.url) } else { Vec::new() };
    let saved = save_document(
        pool,
        NewDocument {
            investigation_id,
            source_id,
            url: &url,
            kind,
            title,
            author: extracted.author.as_deref(),
            site: extracted.site.as_deref().or(host.as_deref()),
            published_at: published,
            lang: extracted.lang.as_deref(),
            text: &extracted.text,
            segments: None,
            note: None,
        },
    )
    .await?;
    remember_image(pool, investigation_id, &url, extracted.image.as_deref()).await?;
    Ok(Ingested { saved, url, links })
}

/// Fetches a YouTube video's subtitles and saves them. Without subtitles, the video is kept
/// with its description and a note saying why.
pub async fn ingest_video(pool: &SqlitePool, app_data_dir: &Path, investigation_id: &str, source_id: Option<&str>, url: &str, hints: Hints<'_>) -> Result<Option<String>> {
    let video = youtube::fetch_video(pool, app_data_dir, url).await?;
    let text = if video.segments.is_empty() {
        video.description.clone()
    } else {
        video.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n")
    };
    if !hints.must_mention.is_empty() {
        let haystack = fold(&format!("{} {} {}", video.title, video.description, text));
        if !hints.must_mention.iter().any(|name| contains_phrase(&haystack, name)) {
            bail!("skip: the video does not mention the target");
        }
    }
    let segments = (!video.segments.is_empty()).then(|| serde_json::to_string(&video.segments).unwrap_or_default());
    let saved = save_document(
        pool,
        NewDocument {
            investigation_id,
            source_id,
            url: &video.url,
            kind: "video",
            title: &video.title,
            author: video.channel.as_deref(),
            site: Some("YouTube"),
            published_at: video.published.as_deref().or(hints.published),
            lang: Some("pt"),
            text: &text,
            segments,
            note: video.note.as_deref(),
        },
    )
    .await?;
    remember_image(pool, investigation_id, &video.url, video.thumbnail.as_deref()).await?;
    Ok(saved)
}

fn is_video(url: &str) -> bool {
    youtube::canonical_video_url(url).is_some()
}

/// Puts what a feed or search listed in the queue, skipping what is already queued or
/// collected. Returns how many were new.
async fn enqueue(pool: &SqlitePool, source: &SourceRow, entries: &[feeds::Entry]) -> Result<usize> {
    let now = schedule_time(Utc::now());
    let via = if source.kind.ends_with("_search") { "search" } else { "feed" };
    let mut added = 0;
    for entry in entries {
        let url = canonical_url(&entry.url);
        let kind = if is_video(&url) { "video" } else { "article" };
        let result = sqlx::query(
            "INSERT OR IGNORE INTO crawl_queue (investigation_id, source_id, url, kind, title, published_at, next_attempt_at, created_at, via)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8
             WHERE NOT EXISTS (SELECT 1 FROM documents WHERE investigation_id = ?1 AND url = ?3)",
        )
        .bind(&source.investigation_id)
        .bind(&source.id)
        .bind(&url)
        .bind(kind)
        .bind(&entry.title)
        .bind(&entry.published)
        .bind(&now)
        .bind(via)
        .execute(pool)
        .await?;
        added += result.rows_affected() as usize;
    }
    Ok(added)
}

/// Runs a feed (site, channel or news search): conditional GET, then its entries to the queue.
async fn run_feed(pool: &SqlitePool, source: &SourceRow, url: &str, unwrap_links: bool) -> Result<usize> {
    let fetched = match fetch::get(url, (source.etag.as_deref(), source.last_modified.as_deref())).await? {
        FetchResult::NotModified => return Ok(0),
        FetchResult::Fetched(fetched) => fetched,
    };
    let feed = feeds::parse(&fetched.body)?;
    let entries: Vec<feeds::Entry> = feed
        .entries
        .into_iter()
        .map(|e| if unwrap_links { feeds::Entry { url: feeds::unwrap_news_link(&e.url), ..e } } else { e })
        .collect();
    sqlx::query("UPDATE sources SET etag = ?, last_modified = ?, title = COALESCE(title, ?) WHERE id = ?")
        .bind(&fetched.etag)
        .bind(&fetched.last_modified)
        .bind(feed.title.filter(|_| !unwrap_links))
        .bind(&source.id)
        .execute(pool)
        .await?;
    enqueue(pool, source, &entries).await
}

/// A social profile's feed: each post is saved from the feed itself (post pages need JavaScript).
async fn run_social(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, source: &SourceRow) -> Result<usize> {
    let fetched = match fetch::get(&source.url, (source.etag.as_deref(), source.last_modified.as_deref())).await? {
        FetchResult::NotModified => return Ok(0),
        FetchResult::Fetched(fetched) => fetched,
    };
    let feed = feeds::parse(&fetched.body)?;
    sqlx::query("UPDATE sources SET etag = ?, last_modified = ?, title = COALESCE(title, ?) WHERE id = ?")
        .bind(&fetched.etag)
        .bind(&fetched.last_modified)
        .bind(&feed.title)
        .bind(&source.id)
        .execute(pool)
        .await?;
    let site = url::Url::parse(&source.url).ok().and_then(|u| u.host_str().map(str::to_string));
    let mut new = 0;
    for entry in &feed.entries {
        let Some(text) = entry.summary.as_deref().or(entry.title.as_deref()) else { continue };
        let title: String = entry.title.as_deref().unwrap_or(text).chars().take(120).collect();
        let url = canonical_url(&entry.url);
        let saved = save_document(
            pool,
            NewDocument {
                investigation_id: &source.investigation_id,
                source_id: Some(&source.id),
                url: &url,
                kind: "post",
                title: &title,
                author: feed.title.as_deref(),
                site: site.as_deref(),
                published_at: entry.published.as_deref(),
                lang: None,
                text,
                segments: None,
                note: None,
            },
        )
        .await?;
        remember_image(pool, &source.investigation_id, &url, entry.image.as_deref()).await?;
        new += index_if_new(pool, embedder, saved).await?;
    }
    Ok(new)
}

/// Runs one source. Returns how many new items it found (documents saved or queued).
pub async fn run_source(pool: &SqlitePool, app_data_dir: &Path, embedder: &dyn EmbeddingProvider, source: &SourceRow) -> Result<usize> {
    match source.kind.as_str() {
        "page" => {
            let page = ingest_page(pool, &source.investigation_id, Some(&source.id), &source.url, "page", Hints::default()).await?;
            let followed = if source.follow_links { follow_links(pool, &source.investigation_id, &source.id, 0, &page).await } else { 0 };
            Ok(index_if_new(pool, embedder, page.saved).await? + followed)
        }
        "youtube_video" => {
            let saved = ingest_video(pool, app_data_dir, &source.investigation_id, Some(&source.id), &source.url, Hints::default()).await?;
            index_if_new(pool, embedder, saved).await
        }
        // A channel is kept as its RSS feed (resolved when the source is added)
        "rss" | "youtube_channel" => run_feed(pool, source, &source.url, false).await,
        "news_search" => run_feed(pool, source, &feeds::news_search_url(&source.url), true).await,
        "youtube_search" => {
            let entries = youtube::search(pool, app_data_dir, &source.url, VIDEOS_PER_SEARCH).await?;
            enqueue(pool, source, &entries).await
        }
        "gazette_search" => {
            let mut new = 0;
            for id in gazettes::run_querido_diario(pool, source).await? {
                new += index_if_new(pool, embedder, Some(id)).await?;
            }
            Ok(new)
        }
        "social" => run_social(pool, embedder, source).await,
        "web_search" => {
            let entries = crate::discover::websearch::search(pool, &source.url).await?;
            enqueue(pool, source, &entries).await
        }
        kind if official::KINDS.contains(&kind) => official::run(pool, app_data_dir, embedder, source).await,
        other => bail!("Unknown source kind: {other}"),
    }
}

/// Queues the page's links worth following; a failure only skips them.
async fn follow_links(pool: &SqlitePool, investigation_id: &str, source_id: &str, depth: i64, page: &Ingested) -> usize {
    let from = crate::discover::links::From { investigation_id, source_id, depth, page_url: &page.url };
    crate::discover::links::follow(pool, crate::discover::links::Triage::Configured, &from, &page.links).await.unwrap_or_else(|e| {
        log::warn!("Discover: links of {} not followed: {e}", page.url);
        0
    })
}

async fn index_if_new(pool: &SqlitePool, embedder: &dyn EmbeddingProvider, saved: Option<String>) -> Result<usize> {
    let Some(id) = saved else { return Ok(0) };
    crate::rag::indexer::index_document(pool, embedder, &id).await?;
    Ok(1)
}

/// Fetches one queued item. Ok(true) when a new document was saved and indexed.
pub async fn process_item(pool: &SqlitePool, app_data_dir: &Path, embedder: &dyn EmbeddingProvider, item: &QueueItem) -> Result<bool> {
    // Items a target's searches found must mention it; what links and the AI's searches found,
    // someone the investigation follows
    let source: Option<(Option<String>, String, bool)> = match &item.source_id {
        Some(id) => sqlx::query_as("SELECT target_id, origin, follow_links FROM sources WHERE id = ?").bind(id).fetch_optional(pool).await?,
        None => None,
    };
    let names = match &source {
        Some((Some(target_id), origin, _)) if origin == "target" => target_names(pool, target_id).await?,
        Some((_, origin, _)) if origin == "llm" => investigation_names(pool, &item.investigation_id).await?,
        _ if item.via == "link" => investigation_names(pool, &item.investigation_id).await?,
        _ => Vec::new(),
    };
    let follows = source.as_ref().is_some_and(|(_, _, follow)| *follow);
    let hints = Hints { title: item.title.as_deref(), published: item.published_at.as_deref(), must_mention: &names };
    let source_id = item.source_id.as_deref();
    let saved = match item.kind.as_str() {
        "video" => ingest_video(pool, app_data_dir, &item.investigation_id, source_id, &item.url, hints).await?,
        _ => {
            let page = ingest_page(pool, &item.investigation_id, source_id, &item.url, "article", hints).await?;
            if let (true, Some(source_id)) = (follows, source_id) {
                follow_links(pool, &item.investigation_id, source_id, item.depth, &page).await;
            }
            page.saved
        }
    };
    Ok(index_if_new(pool, embedder, saved).await? > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_urls_drop_tracking_and_fragments() {
        assert_eq!(canonical_url("https://g1.globo.com/a.ghtml?utm_source=x&id=3#comentarios"), "https://g1.globo.com/a.ghtml?id=3");
        assert_eq!(canonical_url("https://g1.globo.com/a.ghtml?utm_source=x"), "https://g1.globo.com/a.ghtml");
        assert_eq!(canonical_url("https://youtu.be/fFbQi8zCXyY"), "https://www.youtube.com/watch?v=fFbQi8zCXyY");
    }

    #[test]
    fn names_match_whole_words_without_accents() {
        let text = fold("A deputada Tábata Amaral votou; tabatinha não conta.");
        assert!(contains_phrase(&text, &fold("Tabata Amaral")));
        assert!(!contains_phrase(&text, "tabatinh"));
        assert!(!contains_phrase(&text, ""));
    }
}
