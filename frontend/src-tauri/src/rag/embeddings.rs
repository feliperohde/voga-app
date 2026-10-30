//! Embedding providers: the built-in model (default, nothing to install), Ollama (this
//! computer, the network or an external server), or a provider's OpenAI-compatible API
//! (OpenAI, Gemini, a server of the user's) for computers too modest for the built-in model.
//! The indexer and retriever only see the trait.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";
/// Texts sent per request; keeps request bodies and latency bounded.
const BATCH_SIZE: usize = 16;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Identifier stored with each vector; vectors from different models are never compared.
    fn model_id(&self) -> &str;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// The configured embedder is chosen at run time (indexer::embedder).
#[async_trait]
impl<T: EmbeddingProvider + ?Sized> EmbeddingProvider for Box<T> {
    fn model_id(&self) -> &str {
        (**self).model_id()
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        (**self).embed(texts).await
    }
}

pub struct OllamaEmbedder {
    client: reqwest::Client,
    endpoint: String,
    model: String,
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

/// Accepts what people type ("192.168.3.16", "ollama.lan:11434", "https://host")
/// and returns a base URL: scheme defaults to http and port to 11434.
/// Empty input gives the default local endpoint.
pub fn normalize_endpoint(input: Option<&str>) -> String {
    let raw = input.map(str::trim).filter(|e| !e.is_empty()).unwrap_or(DEFAULT_OLLAMA_ENDPOINT);
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("http://{raw}") };
    match url::Url::parse(&with_scheme) {
        Ok(mut parsed) => {
            if parsed.port().is_none() && !raw.contains("://") {
                let _ = parsed.set_port(Some(11434));
            }
            parsed.to_string().trim_end_matches('/').to_string()
        }
        Err(_) => with_scheme.trim_end_matches('/').to_string(),
    }
}

/// Result of probing an Ollama server for the embedding model.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaProbe {
    pub endpoint: String,
    pub reachable: bool,
    pub models: Vec<String>,
    pub model_available: bool,
    pub error: Option<String>,
}

/// Lists the server's models (GET /api/tags) and checks whether `model` is among them
/// (a bare name matches its ":latest" tag).
pub async fn probe_ollama(endpoint: Option<&str>, model: &str) -> OllamaProbe {
    #[derive(Deserialize)]
    struct Tags {
        models: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        name: String,
    }

    let endpoint = normalize_endpoint(endpoint);
    let result = async {
        let response = crate::net::client_for(&endpoint)
            .get(format!("{endpoint}/api/tags"))
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| connection_error(&endpoint, &e))?
            .error_for_status()
            .map_err(|e| anyhow!("Ollama at {endpoint} returned an error: {e}"))?;
        let tags: Tags = response.json().await.context("Unexpected response from /api/tags")?;
        Ok::<_, anyhow::Error>(tags.models.into_iter().map(|t| t.name).collect::<Vec<_>>())
    }
    .await;

    match result {
        Ok(models) => {
            let wanted = model.trim();
            let model_available = models
                .iter()
                .any(|m| m == wanted || m.strip_suffix(":latest") == Some(wanted));
            OllamaProbe { endpoint, reachable: true, models, model_available, error: None }
        }
        Err(e) => OllamaProbe {
            endpoint,
            reachable: false,
            models: Vec::new(),
            model_available: false,
            error: Some(e.to_string()),
        },
    }
}

/// The innermost error message (e.g. "Connection refused (os error 61)",
/// "No route to host (os error 65)"), which tells apart a stopped server, a
/// firewall and a blocked Local Network permission.
fn root_cause(e: &reqwest::Error) -> String {
    let mut source: &dyn std::error::Error = e;
    while let Some(next) = source.source() {
        source = next;
    }
    source.to_string()
}

fn connection_error(endpoint: &str, e: &reqwest::Error) -> anyhow::Error {
    if e.is_connect() || e.is_timeout() {
        anyhow!(connection_error_message(endpoint, &root_cause(e), cfg!(target_os = "macos")))
    } else {
        anyhow!("Ollama request to {endpoint} failed: {e}")
    }
}

/// Explains a failed connection. On macOS, "No route to host" (EHOSTUNREACH, os
/// error 65) to a LAN address while the server answers `curl` means the system's
/// Local Network privacy blocked this app. An unbundled dev binary cannot be granted
/// that permission, so the message offers the two things that work.
pub fn connection_error_message(endpoint: &str, cause: &str, macos: bool) -> String {
    let host = url::Url::parse(endpoint)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    let remote = !host.is_empty() && host != "localhost" && host != "127.0.0.1" && host != "::1";
    let blocked_by_macos = macos && remote && (cause.contains("os error 65") || cause.contains("No route to host"));

    if blocked_by_macos {
        let port = url::Url::parse(endpoint).ok().and_then(|u| u.port_or_known_default()).unwrap_or(11434);
        return format!(
            "Cannot connect to Ollama at {endpoint}: {cause}. macOS is blocking this app's access to the \
             local network (Local Network privacy). Either run the packaged Voga app (pnpm tauri:build) and \
             allow the \"find devices on your local network\" prompt, or forward the port from your terminal and \
             use \"localhost\" as the server: socat TCP-LISTEN:{port},fork,reuseaddr TCP:{host}:{port}"
        );
    }

    let lan_hint = if macos && remote {
        " On macOS, also allow Local Network access for Voga in System Settings → Privacy & Security → Local Network."
    } else {
        ""
    };
    format!(
        "Cannot connect to Ollama at {endpoint}: {cause}. Is it running and reachable? \
         (a remote Ollama must listen on the network: OLLAMA_HOST=0.0.0.0){lan_hint}"
    )
}

impl OllamaEmbedder {
    pub fn new(endpoint: Option<&str>, model: &str) -> Self {
        let endpoint = normalize_endpoint(endpoint);
        Self {
            client: crate::net::client_for(&endpoint),
            endpoint,
            model: model.to_string(),
        }
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/embed", self.endpoint);
        let response = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(120))
            .json(&EmbedRequest { model: &self.model, input: texts })
            .send()
            .await
            .map_err(|e| connection_error(&self.endpoint, &e))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if status.as_u16() == 404 || body.contains("not found") {
                return Err(anyhow!(
                    "Embedding model '{}' is not available in Ollama at {}. Pull it on that machine (ollama pull {}) or set another server in Settings › AI.",
                    self.model,
                    self.endpoint,
                    self.model
                ));
            }
            return Err(anyhow!("Ollama at {} returned HTTP {}: {}", self.endpoint, status, body));
        }

        let parsed: EmbedResponse = response
            .json()
            .await
            .context("Invalid embedding response from Ollama")?;
        if parsed.embeddings.len() != texts.len() {
            return Err(anyhow!(
                "Ollama returned {} embeddings for {} inputs",
                parsed.embeddings.len(),
                texts.len()
            ));
        }
        Ok(parsed.embeddings)
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH_SIZE) {
            vectors.extend(self.embed_batch(batch).await?);
        }
        Ok(vectors)
    }
}

/// Embeds with a provider's OpenAI-compatible API (`POST <base>/embeddings`).
pub struct OpenAiEmbedder {
    client: reqwest::Client,
    base: String,
    key: String,
    model: String,
    /// "<provider>:<model>": the same model through another provider is another index
    id: String,
}

impl OpenAiEmbedder {
    pub fn new(provider: &str, base: &str, key: &str, model: &str) -> Self {
        Self {
            client: crate::net::client_for(base),
            base: base.trim_end_matches('/').to_string(),
            key: key.to_string(),
            model: model.to_string(),
            id: format!("{provider}:{model}"),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbedder {
    fn model_id(&self) -> &str {
        &self.id
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        #[derive(Deserialize)]
        struct Reply {
            data: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            embedding: Vec<f32>,
            #[serde(default)]
            index: usize,
        }
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH_SIZE) {
            let mut request = self
                .client
                .post(format!("{}/embeddings", self.base))
                .timeout(Duration::from_secs(120))
                .json(&serde_json::json!({ "model": self.model, "input": batch }));
            if !self.key.is_empty() {
                request = request.bearer_auth(&self.key);
            }
            let response = request.send().await.with_context(|| format!("{} did not answer", self.base))?;
            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow!("Embedding model '{}' at {} returned HTTP {status}: {}", self.model, self.base, body.chars().take(300).collect::<String>()));
            }
            let mut reply: Reply = response.json().await.context("Invalid embedding response")?;
            if reply.data.len() != batch.len() {
                return Err(anyhow!("{} returned {} embeddings for {} inputs", self.base, reply.data.len(), batch.len()));
            }
            reply.data.sort_by_key(|item| item.index);
            vectors.extend(reply.data.into_iter().map(|item| item.embedding));
        }
        Ok(vectors)
    }
}

/// Embeds with a built-in GGUF model through a dedicated llama-helper process.
pub struct BuiltinEmbedder {
    model: String,
    /// "builtin:<model>": vectors of the same model served by Ollama are not mixed with these
    id: String,
}

impl BuiltinEmbedder {
    pub fn new(model: &str) -> Self {
        Self { model: model.to_string(), id: format!("builtin:{model}") }
    }
}

#[async_trait]
impl EmbeddingProvider for BuiltinEmbedder {
    fn model_id(&self) -> &str {
        &self.id
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(BATCH_SIZE) {
            let embedded = crate::builtin::embed(&self.model, batch).await.map_err(|e| {
                anyhow!("Built-in embedding model '{}' failed: {e}. Download it in Settings › AI.", self.model)
            })?;
            vectors.extend(embedded);
        }
        Ok(vectors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serves one canned HTTP response and returns the raw request it received.
    async fn one_shot_server(status: &str, body: &str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let mut request = String::new();
            // Read until the JSON body has arrived
            while !request.contains("\"input\"") {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });
        (format!("http://{addr}/"), handle)
    }

    #[tokio::test]
    async fn ollama_embedder_posts_to_api_embed() {
        let (endpoint, server) = one_shot_server("200 OK", r#"{"embeddings":[[0.1,0.2],[0.3,0.4]]}"#).await;
        let embedder = OllamaEmbedder::new(Some(&endpoint), "bge-m3");
        let vectors = embedder.embed(&["a".into(), "b".into()]).await.unwrap();
        assert_eq!(vectors, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);

        let request = server.await.unwrap();
        assert!(request.starts_with("POST /api/embed "));
        assert!(request.contains(r#""model":"bge-m3""#));
        assert!(request.contains(r#""input":["a","b"]"#));
    }

    #[tokio::test]
    async fn an_openai_compatible_provider_embeds_in_its_order() {
        let (endpoint, server) =
            one_shot_server("200 OK", r#"{"data":[{"index":1,"embedding":[0.3,0.4]},{"index":0,"embedding":[0.1,0.2]}]}"#).await;
        let embedder = OpenAiEmbedder::new("gemini", &format!("{endpoint}v1"), "secret", "gemini-embedding-2");
        assert_eq!(embedder.model_id(), "gemini:gemini-embedding-2");
        let vectors = embedder.embed(&["a".into(), "b".into()]).await.unwrap();
        assert_eq!(vectors, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /v1/embeddings "));
        assert!(request.to_lowercase().contains("authorization: bearer secret"));
        assert!(request.contains(r#""model":"gemini-embedding-2""#));
    }

    #[tokio::test]
    async fn ollama_embedder_reports_missing_model() {
        let (endpoint, _server) =
            one_shot_server("404 Not Found", r#"{"error":"model \"bge-m3\" not found"}"#).await;
        let embedder = OllamaEmbedder::new(Some(&endpoint), "bge-m3");
        let error = embedder.embed(&["a".into()]).await.unwrap_err().to_string();
        assert!(error.contains("ollama pull bge-m3"), "{error}");
        assert!(error.contains(&endpoint.trim_end_matches('/').to_string()), "{error}");
    }

    #[test]
    fn macos_local_network_block_gets_actionable_message() {
        let msg = connection_error_message("http://192.168.3.16:11434", "No route to host (os error 65)", true);
        assert!(msg.contains("Local Network privacy"), "{msg}");
        assert!(msg.contains("socat TCP-LISTEN:11434,fork,reuseaddr TCP:192.168.3.16:11434"), "{msg}");

        // Same cause elsewhere, or a refused connection, gets the generic advice
        let other_os = connection_error_message("http://192.168.3.16:11434", "No route to host (os error 113)", false);
        assert!(other_os.contains("OLLAMA_HOST=0.0.0.0") && !other_os.contains("socat"));
        let refused = connection_error_message("http://192.168.3.16:11434", "Connection refused (os error 61)", true);
        assert!(refused.contains("OLLAMA_HOST=0.0.0.0") && refused.contains("Local Network"));
        let local = connection_error_message("http://localhost:11434", "Connection refused (os error 61)", true);
        assert!(!local.contains("Local Network"));
    }

    #[test]
    fn endpoints_are_normalized() {
        assert_eq!(normalize_endpoint(None), "http://localhost:11434");
        assert_eq!(normalize_endpoint(Some("  ")), "http://localhost:11434");
        assert_eq!(normalize_endpoint(Some("192.168.3.16")), "http://192.168.3.16:11434");
        assert_eq!(normalize_endpoint(Some("192.168.3.16:8080")), "http://192.168.3.16:8080");
        assert_eq!(normalize_endpoint(Some("http://192.168.3.16:11434/")), "http://192.168.3.16:11434");
        // An explicit scheme without port keeps the scheme default (e.g. behind a proxy)
        assert_eq!(normalize_endpoint(Some("https://ollama.example.com")), "https://ollama.example.com");
        assert_eq!(normalize_endpoint(Some("ollama.lan")), "http://ollama.lan:11434");
    }

    #[tokio::test]
    async fn probe_reports_models_and_availability() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = r#"{"models":[{"name":"bge-m3:latest"},{"name":"llama3.2:3b"}]}"#;
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let probe = probe_ollama(Some(&format!("127.0.0.1:{}", addr.port())), "bge-m3").await;
        assert!(probe.reachable, "{:?}", probe.error);
        assert!(probe.model_available);
        assert_eq!(probe.models.len(), 2);
        assert_eq!(probe.endpoint, format!("http://127.0.0.1:{}", addr.port()));
    }

    #[tokio::test]
    async fn probe_reports_unreachable_server() {
        // Bind then drop to get a port with nothing listening
        let port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let probe = probe_ollama(Some(&format!("127.0.0.1:{port}")), "bge-m3").await;
        assert!(!probe.reachable);
        assert!(probe.error.unwrap().contains("Cannot connect to Ollama at http://127.0.0.1"));
    }

    #[tokio::test]
    async fn ollama_embedder_rejects_count_mismatch() {
        let (endpoint, _server) = one_shot_server("200 OK", r#"{"embeddings":[[0.1]]}"#).await;
        let embedder = OllamaEmbedder::new(Some(&endpoint), "m");
        assert!(embedder.embed(&["a".into(), "b".into()]).await.is_err());
    }
}
