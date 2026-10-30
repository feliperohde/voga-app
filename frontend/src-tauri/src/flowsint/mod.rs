//! Sending an investigation's graph to flowsint (github.com/reconurge/flowsint) for deep
//! analysis there. By default flowsint runs on this computer, installed and started by Voga
//! through Docker (local.rs); a flowsint server is an option. Voga logs in, makes sure
//! flowsint knows Voga's node types (custom
//! types: Party, Bill, VoteSession, Candidacy, Promise, Statement, Record), creates one
//! flowsint investigation + sketch per Voga investigation and imports the nodes and edges.
//!
//! Only facts go: official data and the AI findings a person confirmed. Flowsint merges nodes
//! by label and edges by type, so sending again updates without duplicating.
//!
//! ponytail: what was deleted in Voga is not deleted in flowsint; add DELETE /nodes when it matters.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::HashMap;

use crate::collect::fold;
use crate::graph::{self, Graph, GraphOptions};
use crate::prefs;
use crate::state::AppState;
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub mod local;

/// local (default): flowsint on this computer, through Docker | server: one set up by the user.
const MODE: &str = "flowsint_mode";
const ENDPOINT: &str = "flowsint_endpoint";
const EMAIL: &str = "flowsint_email";
// ponytail: kept in the local database like the AI keys; move to the Keychain when Voga ships widely
const PASSWORD: &str = "flowsint_password";

/// Voga's node kinds flowsint does not have, created there as custom types.
const CUSTOM_TYPES: [(&str, &str, &[&str]); 8] = [
    ("Party", "Partido político", &["name"]),
    ("Bill", "Projeto de lei ou proposição", &["name", "ementa"]),
    ("VoteSession", "Votação nominal no Congresso", &["name", "date", "description", "result"]),
    ("Candidacy", "Candidatura registrada no TSE", &["name", "office", "party", "number", "result"]),
    ("Promise", "Promessa feita por uma pessoa", &["name", "quote", "topic", "date"]),
    ("Statement", "Declaração ou posição de uma pessoa", &["name", "quote", "topic", "date"]),
    ("Amendment", "Emenda parlamentar ao Orçamento da União", &["name", "year", "area", "paid"]),
    ("Record", "Documento coletado pelo Voga (notícia, discurso, vídeo…)", &["name", "url", "kind", "date"]),
];

fn flowsint_type(kind: &str) -> &'static str {
    match kind {
        "person" => "Individual",
        "organization" => "Organization",
        "party" => "Party",
        "bill" => "Bill",
        "vote_session" => "VoteSession",
        "amendment" => "Amendment",
        "candidacy" => "Candidacy",
        "promise" => "Promise",
        "statement" => "Statement",
        _ => "Record",
    }
}

/// "VOTED" with the vote in it, so it shows on the edge: VOTED_SIM, VOTED_NAO, VOTED_ABSTENCAO.
fn edge_label(label: &str, vote: Option<&str>) -> String {
    match vote {
        Some(vote) if label == "VOTED" => {
            let vote: String = fold(vote).chars().map(|c| if c.is_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
            format!("VOTED_{}", vote.trim_matches('_'))
        }
        _ => label.to_string(),
    }
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "sim".into() } else { "não".into() }),
        _ => None,
    }
}

/// The body of flowsint's import (`entity_mappings_json`). Labels are made unique per type,
/// since flowsint merges nodes of a type with the same label; edges only between nodes sent.
pub fn import_payload(graph: &Graph, document_urls: &HashMap<String, String>) -> Value {
    let mut seen: HashMap<(&str, String), usize> = HashMap::new();
    let mut nodes = Vec::new();
    for node in &graph.nodes {
        let kind = flowsint_type(&node.kind);
        let base: String = node.label.chars().take(140).collect();
        let count = seen.entry((kind, base.clone())).or_insert(0);
        *count += 1;
        let label = if *count == 1 { base } else { format!("{base} · {}", *count) };
        let attrs: Value = serde_json::from_str(&node.attrs).unwrap_or_default();
        let mut data = serde_json::Map::new();
        data.insert("nodeLabel".into(), json!(label));
        match kind {
            "Individual" => {
                data.insert("full_name".into(), json!(label));
            }
            _ => {
                data.insert("name".into(), json!(label));
            }
        }
        // What Voga knows about it, as text fields
        let fields: &[(&str, &str)] = match kind {
            "Organization" => &[("cnpj", "cnpj")],
            "Bill" => &[("ementa", "ementa")],
            "VoteSession" => &[("description", "description"), ("result", "result")],
            "Amendment" => &[("year", "year"), ("area", "area"), ("paid", "paid")],
            "Candidacy" => &[("office", "office"), ("party", "party"), ("number", "number"), ("result", "result")],
            "Promise" | "Statement" => &[("quote", "quote"), ("topic", "topic"), ("date", "stated_at")],
            "Individual" => &[("camara_id", "camara_id"), ("senado_id", "senado_id"), ("uf", "uf")],
            _ => &[],
        };
        for (field, attr) in fields {
            if let Some(value) = text(&attrs[*attr]) {
                data.insert((*field).into(), json!(value));
            }
        }
        if kind == "VoteSession" {
            if let Some(approved) = attrs["approved"].as_bool() {
                data.insert("result".into(), json!(if approved { "aprovada" } else { "rejeitada" }));
            }
        }
        if kind == "Record" {
            data.insert("kind".into(), json!(node.kind));
            if let Some(url) = node.document_id.as_ref().and_then(|d| document_urls.get(d)) {
                data.insert("url".into(), json!(url));
            }
        }
        nodes.push(json!({"id": node.id, "node_id": node.id, "entity_type": kind, "include": true, "nodeLabel": label, "data": data}));
    }
    let edges: Vec<Value> = graph
        .links
        .iter()
        .filter(|l| l.status == "confirmed")
        .map(|l| json!({"from_id": l.source, "to_id": l.target, "label": edge_label(&l.label, l.vote.as_deref())}))
        .collect();
    json!({"nodes": nodes, "edges": edges})
}

/// flowsint's nginx takes at most 1 MB per request (client_max_body_size): imports go in parts.
const MAX_IMPORT_BYTES: usize = 700_000;

/// Splits an import so each part stays under `max` bytes of JSON. flowsint links only nodes of
/// the same request, so each edge goes with its two nodes; nodes merge by label, so a node sent
/// again in another part is the same node. Nodes without edges fill the last parts.
pub fn import_parts(payload: &Value, max: usize) -> Vec<Value> {
    let nodes: HashMap<&str, &Value> = payload["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n["node_id"].as_str()?, n)))
        .collect();
    let size = |v: &Value| v.to_string().len() + 1;
    let mut parts = Vec::new();
    let mut part_nodes: Vec<&Value> = Vec::new();
    let mut part_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut part_edges: Vec<&Value> = Vec::new();
    let mut part_size = 0;
    let mut sent: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut flush = |part_nodes: &mut Vec<&Value>, part_ids: &mut std::collections::HashSet<&str>, part_edges: &mut Vec<&Value>, part_size: &mut usize| {
        if !part_nodes.is_empty() {
            parts.push(json!({"nodes": part_nodes, "edges": part_edges}));
        }
        part_nodes.clear();
        part_ids.clear();
        part_edges.clear();
        *part_size = 0;
    };
    for edge in payload["edges"].as_array().into_iter().flatten() {
        let ends: Vec<&str> = [&edge["from_id"], &edge["to_id"]].iter().filter_map(|id| id.as_str()).filter(|id| nodes.contains_key(id)).collect();
        if ends.len() < 2 {
            continue;
        }
        let missing: Vec<&str> = ends.iter().copied().filter(|id| !part_ids.contains(id)).collect();
        let added = size(edge) + missing.iter().map(|id| size(nodes[id])).sum::<usize>();
        if part_size + added > max && !part_nodes.is_empty() {
            flush(&mut part_nodes, &mut part_ids, &mut part_edges, &mut part_size);
        }
        for id in ends {
            if part_ids.insert(id) {
                part_nodes.push(nodes[id]);
                part_size += size(nodes[id]);
                sent.insert(id);
            }
        }
        part_edges.push(edge);
        part_size += size(edge);
    }
    for node in payload["nodes"].as_array().into_iter().flatten() {
        let Some(id) = node["node_id"].as_str() else { continue };
        if sent.contains(id) || part_ids.contains(id) {
            continue;
        }
        if part_size + size(node) > max && !part_nodes.is_empty() {
            flush(&mut part_nodes, &mut part_ids, &mut part_edges, &mut part_size);
        }
        part_ids.insert(id);
        part_nodes.push(node);
        part_size += size(node);
    }
    flush(&mut part_nodes, &mut part_ids, &mut part_edges, &mut part_size);
    parts
}

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

async fn check(response: reqwest::Response, what: &str) -> Result<Value> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&body).ok().and_then(|v| text(&v["detail"])).unwrap_or(body);
        bail!("flowsint: {what} failed (HTTP {status}): {detail}");
    }
    Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
}

impl Client {
    async fn login(endpoint: &str, email: &str, password: &str) -> Result<Client> {
        let base = endpoint.trim().trim_end_matches('/').to_string();
        let http = crate::net::client_for(&base);
        let response = http
            .post(format!("{base}/api/auth/token"))
            .form(&[("username", email), ("password", password)])
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .with_context(|| format!("Could not reach flowsint at {base}"))?;
        let body = check(response, "login").await?;
        let token = text(&body["access_token"]).ok_or_else(|| anyhow!("flowsint did not return a token"))?;
        Ok(Client { http, base, token })
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let response = self.http.get(format!("{}/api{path}", self.base)).bearer_auth(&self.token).send().await?;
        check(response, path).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        let response = self.http.post(format!("{}/api{path}", self.base)).bearer_auth(&self.token).json(&body).send().await?;
        check(response, path).await
    }

    /// Creates Voga's custom types that are missing (published, so imports can use them).
    async fn ensure_types(&self) -> Result<()> {
        let existing = self.get("/custom-types").await?;
        let names: Vec<String> = existing.as_array().map(|a| a.iter().filter_map(|t| text(&t["name"])).collect()).unwrap_or_default();
        for (name, description, fields) in CUSTOM_TYPES {
            if names.iter().any(|n| n == name) {
                continue;
            }
            let properties: serde_json::Map<String, Value> = fields.iter().map(|f| ((*f).to_string(), json!({"type": "string", "title": f}))).collect();
            self.post(
                "/custom-types",
                json!({
                    "name": name,
                    "schema": {"type": "object", "properties": properties, "required": ["name"]},
                    "description": description,
                    "status": "published",
                    "category": "Voga",
                }),
            )
            .await?;
        }
        Ok(())
    }

    async fn import(&self, sketch: &str, payload: &Value) -> Result<Value> {
        // Multipart, so the JSON goes as it is (URL-encoding would grow it toward nginx's limit)
        let form = reqwest::multipart::Form::new().text("entity_mappings_json", payload.to_string());
        let response = self
            .http
            .post(format!("{}/api/sketches/{sketch}/import/execute", self.base))
            .bearer_auth(&self.token)
            .multipart(form)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await?;
        check(response, "import").await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sync {
    pub investigation: String,
    pub sketch: String,
    pub pushed_at: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub url: String,
    pub nodes: usize,
    pub edges: usize,
    pub nodes_created: i64,
    pub errors: Vec<String>,
}

async fn uses_server(pool: &SqlitePool) -> bool {
    prefs::get(pool, MODE).await.ok().flatten().as_deref() == Some("server")
}

async fn server_credentials(pool: &SqlitePool) -> Result<(String, String, String)> {
    let get = |key| async move { prefs::get(pool, key).await.ok().flatten().filter(|v| !v.trim().is_empty()) };
    match (get(ENDPOINT).await, get(EMAIL).await, get(PASSWORD).await) {
        (Some(endpoint), Some(email), Some(password)) => Ok((endpoint, email, password)),
        _ => bail!("Set up the flowsint server in Settings › flowsint"),
    }
}

/// Reports the local flowsint's progress to the windows.
fn emitter<R: Runtime>(app: &AppHandle<R>) -> impl Fn(&local::Progress) + Send + std::marker::Sync + 'static {
    let app = app.clone();
    move |progress| {
        let _ = app.emit(local::PROGRESS_EVENT, progress);
    }
}

/// Where to send and how to log in: the flowsint on this computer (started if needed) or the server.
async fn credentials<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) -> Result<(String, String, String)> {
    if uses_server(pool).await {
        server_credentials(pool).await
    } else {
        local::ensure_running(&emitter(app), pool, &app.path().app_data_dir()?).await
    }
}

pub async fn push<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, investigation_id: &str) -> Result<PushResult> {
    let (endpoint, email, password) = credentials(app, pool).await?;
    push_to(pool, investigation_id, &endpoint, &email, &password).await
}

/// Sends the investigation to the flowsint at `endpoint`.
pub async fn push_to(pool: &SqlitePool, investigation_id: &str, endpoint: &str, email: &str, password: &str) -> Result<PushResult> {
    let client = Client::login(endpoint, email, password).await?;
    client.ensure_types().await?;

    let graph = graph::load(pool, investigation_id, GraphOptions { all_votes: true, suggested: false }).await?;
    let urls: HashMap<String, String> = sqlx::query_as::<_, (String, String)>("SELECT id, url FROM documents WHERE investigation_id = ?")
        .bind(investigation_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
    let payload = import_payload(&graph, &urls);

    let key = format!("flowsint_sync:{investigation_id}");
    let known: Option<Sync> = prefs::get(pool, &key).await?.and_then(|json| serde_json::from_str(&json).ok());
    // The same sketch on later sends (a new one if it was deleted in flowsint or the server changed)
    let reusable = match &known {
        Some(sync) if sync.url.starts_with(&client.base) => client.get(&format!("/sketches/{}", sync.sketch)).await.is_ok(),
        _ => false,
    };
    let (investigation, sketch) = match (known, reusable) {
        (Some(sync), true) => (sync.investigation, sync.sketch),
        _ => {
            let (name, description): (String, String) = sqlx::query_as("SELECT name, description FROM investigations WHERE id = ?")
                .bind(investigation_id)
                .fetch_one(pool)
                .await?;
            let description = if description.is_empty() { "Enviada pelo Voga".to_string() } else { description };
            let created = client.post("/investigations/create", json!({"name": name, "description": description})).await?;
            let investigation = text(&created["id"]).ok_or_else(|| anyhow!("flowsint did not return the investigation id"))?;
            let sketch = client
                .post("/sketches/create", json!({"title": "Voga", "description": "Grafo enviado pelo Voga", "investigation_id": investigation}))
                .await?;
            let sketch = text(&sketch["id"]).ok_or_else(|| anyhow!("flowsint did not return the sketch id"))?;
            (investigation, sketch)
        }
    };

    let mut nodes_created = 0;
    let mut errors = Vec::new();
    for part in import_parts(&payload, MAX_IMPORT_BYTES) {
        let result = client.import(&sketch, &part).await?;
        nodes_created += result["nodes_created"].as_i64().unwrap_or_default();
        errors.extend(result["errors"].as_array().map(|e| e.iter().filter_map(text).collect::<Vec<_>>()).unwrap_or_default());
    }
    let url = format!("{}/dashboard/investigations/{investigation}/graph/{sketch}", client.base);
    let sync = Sync { investigation, sketch, pushed_at: chrono::Utc::now().to_rfc3339(), url: url.clone() };
    prefs::set(pool, &key, &serde_json::to_string(&sync)?).await?;
    Ok(PushResult {
        url,
        nodes: graph.nodes.len(),
        edges: payload["edges"].as_array().map_or(0, Vec::len),
        nodes_created,
        errors,
    })
}

// ===== Commands =====

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowsintSettings {
    /// local | server
    #[serde(default)]
    pub mode: String,
    pub endpoint: String,
    pub email: String,
    /// Empty: keep the saved one. Never sent back to the UI.
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub has_password: bool,
}

#[tauri::command]
pub async fn get_flowsint_settings(state: tauri::State<'_, AppState>) -> Result<FlowsintSettings, String> {
    let pool = state.db_manager.pool();
    let get = |key| async move { prefs::get(pool, key).await.ok().flatten().unwrap_or_default() };
    let mode = if uses_server(pool).await { "server" } else { "local" }.to_string();
    Ok(FlowsintSettings { mode, endpoint: get(ENDPOINT).await, email: get(EMAIL).await, password: String::new(), has_password: !get(PASSWORD).await.is_empty() })
}

#[tauri::command]
pub async fn save_flowsint_settings(state: tauri::State<'_, AppState>, settings: FlowsintSettings) -> Result<(), String> {
    let pool = state.db_manager.pool();
    let endpoint = settings.endpoint.trim().trim_end_matches('/');
    let endpoint = if endpoint.is_empty() || endpoint.contains("://") { endpoint.to_string() } else { format!("http://{endpoint}") };
    prefs::set(pool, MODE, if settings.mode == "server" { "server" } else { "local" }).await.map_err(|e| e.to_string())?;
    prefs::set(pool, ENDPOINT, &endpoint).await.map_err(|e| e.to_string())?;
    prefs::set(pool, EMAIL, settings.email.trim()).await.map_err(|e| e.to_string())?;
    if !settings.password.is_empty() {
        prefs::set(pool, PASSWORD, &settings.password).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Logs in to the flowsint server with the saved settings: Ok when it accepts them.
#[tauri::command]
pub async fn test_flowsint(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let (endpoint, email, password) = server_credentials(state.db_manager.pool()).await.map_err(|e| e.to_string())?;
    Client::login(&endpoint, &email, &password).await.map(|_| ()).map_err(|e| e.to_string())
}

/// Creates the account on the flowsint server with the saved settings (a new server has none).
#[tauri::command]
pub async fn create_flowsint_account(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let (endpoint, email, password) = server_credentials(state.db_manager.pool()).await.map_err(|e| e.to_string())?;
    let base = endpoint.trim_end_matches('/');
    let response = crate::net::client_for(base)
        .post(format!("{base}/api/auth/register"))
        .json(&json!({"email": email, "password": password}))
        .send()
        .await
        .map_err(|e| format!("Could not reach flowsint at {base}: {e}"))?;
    check(response, "account creation").await.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn push_to_flowsint(app: AppHandle, state: tauri::State<'_, AppState>, investigation_id: String) -> Result<PushResult, String> {
    let pool = state.db_manager.pool();
    push(&app, pool, &investigation_id).await.map_err(|e| e.to_string())
}

/// When and where the investigation was last sent, if ever.
#[tauri::command]
pub async fn flowsint_sync(state: tauri::State<'_, AppState>, investigation_id: String) -> Result<Option<Sync>, String> {
    let json = prefs::get(state.db_manager.pool(), &format!("flowsint_sync:{investigation_id}")).await.map_err(|e| e.to_string())?;
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

// ===== flowsint on this computer =====

fn data_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path().app_data_dir().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn flowsint_local_status(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<local::LocalStatus, String> {
    Ok(local::status(state.db_manager.pool(), &data_dir(&app)?).await)
}

/// Downloads flowsint's images, starts it and creates Voga's account (progress: flowsint-local-progress).
#[tauri::command]
pub async fn flowsint_local_install(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let pool = state.db_manager.pool();
    local::install(&emitter(&app), pool, &data_dir(&app)?).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn flowsint_local_start(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let pool = state.db_manager.pool();
    local::start(&emitter(&app), pool, &data_dir(&app)?).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn flowsint_local_stop(app: AppHandle) -> Result<(), String> {
    local::stop(&data_dir(&app)?).await.map_err(|e| e.to_string())
}

/// Removes the local flowsint with its data and images.
#[tauri::command]
pub async fn flowsint_local_remove(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    local::remove(state.db_manager.pool(), &data_dir(&app)?).await.map_err(|e| e.to_string())
}

/// Opens Docker Desktop or OrbStack.
#[tauri::command]
pub async fn open_docker() -> Result<(), String> {
    local::open_engine().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_are_split_with_each_edge_beside_its_nodes() {
        let nodes: Vec<Value> = (0..40)
            .map(|i| json!({"id": format!("n{i}"), "node_id": format!("n{i}"), "entity_type": "Record", "include": true, "nodeLabel": format!("Nó {i}"), "data": {"name": "x".repeat(200)}}))
            .collect();
        // A chain plus a node with no edges (n39)
        let edges: Vec<Value> = (0..38).map(|i| json!({"from_id": format!("n{i}"), "to_id": format!("n{}", i + 1), "label": "ABOUT"})).collect();
        let payload = json!({"nodes": nodes, "edges": edges});
        let max = 2_000;
        let parts = import_parts(&payload, max);
        assert!(parts.len() > 3, "{}", parts.len());
        let mut edges_sent = 0;
        let mut nodes_sent = std::collections::HashSet::new();
        for part in &parts {
            assert!(part.to_string().len() <= max + 300, "part too big: {}", part.to_string().len());
            let ids: std::collections::HashSet<&str> = part["nodes"].as_array().unwrap().iter().map(|n| n["node_id"].as_str().unwrap()).collect();
            for edge in part["edges"].as_array().unwrap() {
                assert!(ids.contains(edge["from_id"].as_str().unwrap()) && ids.contains(edge["to_id"].as_str().unwrap()));
                edges_sent += 1;
            }
            nodes_sent.extend(ids);
        }
        assert_eq!(edges_sent, 38);
        assert_eq!(nodes_sent.len(), 40);
        // Small enough: one part, as before
        assert_eq!(import_parts(&payload, usize::MAX).len(), 1);
    }
    use crate::graph::{Link, Node};

    fn node(id: &str, kind: &str, label: &str, attrs: Value) -> Node {
        Node { id: id.into(), kind: kind.into(), key: id.into(), label: label.into(), attrs: attrs.to_string(), document_id: None, investigations: vec![] }
    }

    fn link(from: &str, to: &str, label: &str, status: &str, vote: Option<&str>) -> Link {
        Link {
            id: format!("{from}-{to}"),
            source: from.into(),
            target: to.into(),
            label: label.into(),
            status: status.into(),
            date: String::new(),
            props: "{}".into(),
            document_id: None,
            vote: vote.map(String::from),
        }
    }

    #[test]
    fn the_import_has_typed_nodes_unique_labels_and_only_confirmed_edges() {
        let graph = Graph {
            nodes: vec![
                node("p", "person", "Marina Exemplo", json!({"camara_id": "900001", "uf": "SP"})),
                node("v1", "vote_session", "PLP 99/2025 (02/10/2026)", json!({"approved": true, "description": "Texto-base"})),
                node("v2", "vote_session", "PLP 99/2025 (02/10/2026)", json!({"approved": false})),
                node("o", "organization", "Gráfica Exemplo", json!({"cnpj": "12345678000190"})),
                node("c", "promise", "Promete simplificar impostos.", json!({"quote": "vou votar pela simplificação", "stated_at": "2022-09-28"})),
                node("d", "article", "Notícia", json!({})),
            ],
            links: vec![
                link("p", "v1", "VOTED", "confirmed", Some("Não")),
                link("p", "v2", "VOTED", "confirmed", Some("Abstenção")),
                link("p", "c", "MADE", "confirmed", None),
                link("c", "v1", "CONTRADICTS", "suggested", None),
                link("p", "o", "SPENT_WITH", "confirmed", None),
            ],
        };
        let payload = import_payload(&graph, &HashMap::new());
        let nodes = payload["nodes"].as_array().unwrap();
        let by_id = |id: &str| nodes.iter().find(|n| n["node_id"] == id).unwrap().clone();
        assert_eq!(by_id("p")["entity_type"], "Individual");
        assert_eq!(by_id("p")["data"]["full_name"], "Marina Exemplo");
        assert_eq!(by_id("p")["data"]["camara_id"], "900001");
        assert_eq!(by_id("o")["data"]["cnpj"], "12345678000190");
        assert_eq!(by_id("c")["entity_type"], "Promise");
        assert_eq!(by_id("c")["data"]["date"], "2022-09-28");
        assert_eq!(by_id("d")["entity_type"], "Record");
        assert_eq!(by_id("v1")["data"]["result"], "aprovada");
        // Two votes with the same label stay two nodes
        assert_ne!(by_id("v1")["nodeLabel"], by_id("v2")["nodeLabel"]);
        assert_eq!(by_id("v2")["nodeLabel"], by_id("v2")["data"]["nodeLabel"]);

        let edges: Vec<&str> = payload["edges"].as_array().unwrap().iter().map(|e| e["label"].as_str().unwrap()).collect();
        assert_eq!(edges, ["VOTED_NAO", "VOTED_ABSTENCAO", "MADE", "SPENT_WITH"]);
    }
}
