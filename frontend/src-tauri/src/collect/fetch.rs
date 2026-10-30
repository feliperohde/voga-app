//! HTTP for collection: identifies itself, obeys robots.txt and waits between requests to
//! the same site. Collection runs one request at a time (see `scheduler`), so a single map
//! of hosts is enough.

use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use texting_robots::Robot;
use tokio::sync::Mutex;

pub const USER_AGENT: &str = concat!("Mozilla/5.0 (compatible; Voga/", env!("CARGO_PKG_VERSION"), "; research app)");
/// The name robots.txt rules are matched against.
const ROBOTS_AGENT: &str = "Voga";
/// Least time between two requests to the same host (robots.txt may ask for more).
const MIN_DELAY: Duration = Duration::from_secs(2);
const ROBOTS_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_BYTES: usize = 15 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct Fetched {
    /// After redirects.
    pub url: String,
    pub content_type: String,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

pub enum FetchResult {
    Fetched(Fetched),
    /// The server says it did not change since the validators sent.
    NotModified,
}

struct Host {
    /// None: no usable robots.txt, everything allowed.
    robots: Option<Robot>,
    robots_at: Instant,
    next_request: Instant,
    /// The host refused the native TLS (it only speaks TLS 1.3, as dados.mg.gov.br): rustls instead.
    rustls: bool,
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .build()
            .expect("HTTP client")
    })
}

/// macOS's native TLS (Secure Transport) stops at TLS 1.2; rustls speaks 1.3. Only for the hosts
/// that need it: the native one trusts what the system trusts.
fn rustls_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .use_rustls_tls()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .build()
            .expect("HTTP client")
    })
}

fn client_for(rustls: bool) -> &'static reqwest::Client {
    if rustls {
        rustls_client()
    } else {
        client()
    }
}

fn hosts() -> &'static Mutex<HashMap<String, Host>> {
    static HOSTS: OnceLock<Mutex<HashMap<String, Host>>> = OnceLock::new();
    HOSTS.get_or_init(Default::default)
}

/// The host's robots.txt, and whether the host needs rustls (it refused to connect otherwise).
async fn load_robots(origin: &str) -> (Option<Robot>, bool) {
    let url = format!("{origin}/robots.txt");
    let (response, rustls) = match client().get(&url).send().await {
        Ok(response) => (response, false),
        Err(e) if e.is_connect() => match rustls_client().get(&url).send().await {
            Ok(response) => (response, true),
            Err(_) => {
                log::info!("Fetch: no robots.txt at {origin} ({e}); allowing");
                return (None, false);
            }
        },
        Err(e) => {
            log::info!("Fetch: no robots.txt at {origin} ({e}); allowing");
            return (None, false);
        }
    };
    if !response.status().is_success() {
        return (None, rustls);
    }
    let robots = match response.bytes().await {
        Ok(body) => Robot::new(ROBOTS_AGENT, &body).ok(),
        Err(_) => None,
    };
    (robots, rustls)
}

/// Waits for the host's turn and checks robots.txt. Err when robots.txt forbids `url`; else
/// whether the host needs rustls.
async fn take_turn(url: &url::Url) -> Result<bool> {
    let host_key = url.host_str().ok_or_else(|| anyhow!("URL without host: {url}"))?.to_string();
    let origin = url.origin().ascii_serialization();
    let mut hosts = hosts().lock().await;
    let stale = hosts.get(&host_key).map_or(true, |h| h.robots_at.elapsed() > ROBOTS_TTL);
    if stale {
        let (robots, rustls) = load_robots(&origin).await;
        let next_request = hosts.get(&host_key).map_or_else(Instant::now, |h| h.next_request);
        hosts.insert(host_key.clone(), Host { robots, robots_at: Instant::now(), next_request, rustls });
    }
    let host = hosts.get_mut(&host_key).expect("inserted above");
    if let Some(robots) = &host.robots {
        if !robots.allowed(url.as_str()) {
            bail!("robots.txt of {host_key} does not allow collecting this page");
        }
    }
    let delay = host
        .robots
        .as_ref()
        .and_then(|r| r.delay)
        .map(|d| Duration::from_secs_f32(d.clamp(0.0, 60.0)))
        .unwrap_or(MIN_DELAY)
        .max(MIN_DELAY);
    let wait = host.next_request.saturating_duration_since(Instant::now());
    host.next_request = Instant::now() + wait + delay;
    let rustls = host.rustls;
    drop(hosts);
    tokio::time::sleep(wait).await;
    Ok(rustls)
}

/// Sends a polite GET. None when the server answers 304 to the `validators` (ETag, Last-Modified).
async fn send(url: &str, validators: (Option<&str>, Option<&str>), accept: Option<&str>, timeout: Duration, range: Option<&str>) -> Result<Option<reqwest::Response>> {
    let parsed = url::Url::parse(url).map_err(|e| anyhow!("Invalid URL {url}: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("Only http(s) URLs can be collected: {url}");
    }
    let rustls = take_turn(&parsed).await?;

    let mut request = client_for(rustls).get(parsed).timeout(timeout);
    if let Some(etag) = validators.0 {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    if let Some(modified) = validators.1 {
        request = request.header(reqwest::header::IF_MODIFIED_SINCE, modified);
    }
    if let Some(accept) = accept {
        request = request.header(reqwest::header::ACCEPT, accept);
    }
    if let Some(range) = range {
        request = request.header(reqwest::header::RANGE, range);
    }
    let retry = request.try_clone();
    let response = match (request.send().await, retry) {
        (Err(e), Some(retry)) if stale_connection(&e) => retry.send().await,
        (sent, _) => sent,
    }
    .map_err(|e| anyhow!("Could not reach {url}: {e}"))?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !response.status().is_success() {
        bail!("{url} answered HTTP {}", response.status());
    }
    Ok(Some(response))
}

/// A pooled connection the server had already closed (S3 behind a redirect does it): nothing was
/// read, and a GET can go again.
fn stale_connection(e: &reqwest::Error) -> bool {
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        if cause.to_string().contains("connection closed before message completed") {
            return true;
        }
        source = cause.source();
    }
    false
}

fn header(response: &reqwest::Response, name: reqwest::header::HeaderName) -> Option<String> {
    response.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from)
}

/// GETs `url` politely. `validators` (ETag, Last-Modified) make it conditional.
pub async fn get(url: &str, validators: (Option<&str>, Option<&str>)) -> Result<FetchResult> {
    let Some(response) = send(url, validators, None, TIMEOUT, None).await? else {
        return Ok(FetchResult::NotModified);
    };
    if response.content_length().is_some_and(|len| len as usize > MAX_BYTES) {
        bail!("{url} is larger than {} MB", MAX_BYTES / 1024 / 1024);
    }
    let content_type = header(&response, reqwest::header::CONTENT_TYPE).unwrap_or_default();
    let etag = header(&response, reqwest::header::ETAG);
    let last_modified = header(&response, reqwest::header::LAST_MODIFIED);
    let final_url = response.url().to_string();
    let body = response.bytes().await.map_err(|e| anyhow!("Could not read {url}: {e}"))?.to_vec();
    if body.len() > MAX_BYTES {
        bail!("{url} is larger than {} MB", MAX_BYTES / 1024 / 1024);
    }
    Ok(FetchResult::Fetched(Fetched { url: final_url, content_type, body, etag, last_modified }))
}

/// GET without validators, for when the content is always wanted.
pub async fn get_body(url: &str) -> Result<Fetched> {
    match get(url, (None, None)).await? {
        FetchResult::Fetched(fetched) => Ok(fetched),
        FetchResult::NotModified => bail!("{url} answered 304 to an unconditional request"),
    }
}

/// GETs a picture: its type (image/…) and bytes, refused when it is something else or larger
/// than `max_bytes`.
pub async fn get_image(url: &str, max_bytes: usize) -> Result<(String, Vec<u8>)> {
    let response = send(url, (None, None), Some("image/*"), TIMEOUT, None).await?.ok_or_else(|| anyhow!("{url} answered 304"))?;
    let content_type = header(&response, reqwest::header::CONTENT_TYPE).unwrap_or_default().to_ascii_lowercase();
    if !content_type.starts_with("image/") {
        bail!("{url} is not a picture ({content_type})");
    }
    if response.content_length().is_some_and(|len| len as usize > max_bytes) {
        bail!("{url} is larger than {} KB", max_bytes / 1024);
    }
    let body = response.bytes().await.map_err(|e| anyhow!("Could not read {url}: {e}"))?;
    if body.len() > max_bytes {
        bail!("{url} is larger than {} KB", max_bytes / 1024);
    }
    Ok((content_type, body.to_vec()))
}

/// GETs a JSON API (asks for JSON: some open-data APIs answer XML by default).
pub async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    let response = send(url, (None, None), Some("application/json"), TIMEOUT, None).await?.ok_or_else(|| anyhow!("{url} answered 304"))?;
    let body = response.bytes().await.map_err(|e| anyhow!("Could not read {url}: {e}"))?;
    serde_json::from_slice(&body).map_err(|e| anyhow!("Unexpected answer from {url}: {e}"))
}

/// Part of a file (`range`: "bytes=a-b", or "bytes=-n" for the last n), and the whole file's size:
/// one entry out of a remote ZIP without downloading all of it.
pub async fn get_range(url: &str, range: &str) -> Result<(Vec<u8>, u64)> {
    let response = send(url, (None, None), None, Duration::from_secs(5 * 60), Some(range)).await?.ok_or_else(|| anyhow!("{url} answered 304"))?;
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        bail!("{url} does not serve parts of the file (HTTP {})", response.status());
    }
    let total = header(&response, reqwest::header::CONTENT_RANGE)
        .and_then(|r| r.rsplit('/').next().and_then(|t| t.parse::<u64>().ok()))
        .ok_or_else(|| anyhow!("{url} gave no file size"))?;
    let body = response.bytes().await.map_err(|e| anyhow!("Could not read {url}: {e}"))?;
    Ok((body.to_vec(), total))
}

/// Downloads a large file (open-data dumps) to `dest`, streaming. Returns the new validators
/// (ETag, Last-Modified), or None when the server says the copy we have is current. Some
/// servers drop long transfers half-way: the download resumes where it stopped (Range).
pub async fn download(url: &str, dest: &std::path::Path, validators: (Option<&str>, Option<&str>), max_bytes: u64) -> Result<Option<(Option<String>, Option<String>)>> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    const ATTEMPTS: usize = 8;
    // Dumps of tens of MB take longer than a page
    let timeout = Duration::from_secs(15 * 60);
    let Some(mut response) = send(url, validators, None, timeout, None).await? else { return Ok(None) };
    let expected = response.content_length();
    if expected.is_some_and(|len| len > max_bytes) {
        bail!("{url} is larger than {} MB", max_bytes / 1024 / 1024);
    }
    let found = (header(&response, reqwest::header::ETAG), header(&response, reqwest::header::LAST_MODIFIED));
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Into a temporary file first, so a broken download never replaces a good copy
    let partial = dest.with_extension("part");
    let mut file = tokio::fs::File::create(&partial).await?;
    let mut written = 0u64;
    for attempt in 1..=ATTEMPTS {
        let mut stream = response.bytes_stream();
        let mut broken = None;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) => {
                    broken = Some(e);
                    break;
                }
            };
            written += chunk.len() as u64;
            if written > max_bytes {
                drop(file);
                let _ = tokio::fs::remove_file(&partial).await;
                bail!("{url} is larger than {} MB", max_bytes / 1024 / 1024);
            }
            file.write_all(&chunk).await?;
        }
        let complete = expected.map_or(broken.is_none(), |len| written >= len);
        if complete {
            break;
        }
        if attempt == ATTEMPTS {
            drop(file);
            let _ = tokio::fs::remove_file(&partial).await;
            bail!("Download of {url} kept breaking ({written} bytes of {expected:?})");
        }
        log::info!("Fetch: {url} stopped at {written} bytes; resuming");
        response = send(url, (None, None), None, timeout, Some(&format!("bytes={written}-"))).await?.ok_or_else(|| anyhow!("{url} answered 304 to a range request"))?;
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            bail!("{url} does not resume downloads (HTTP {})", response.status());
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&partial, dest).await?;
    Ok(Some(found))
}
