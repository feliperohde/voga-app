//! flowsint on this computer, the default: Voga writes a Docker Compose project (compose.yml)
//! into `<app data>/flowsint` and installs, starts, stops and removes it with the docker CLI of
//! Docker Desktop, OrbStack or any Docker engine. flowsint is six services (Postgres, Redis,
//! Neo4j, its API, a worker and the web app) with no native build, so Docker is the one thing
//! the person installs; the images, the secrets and the flowsint account are Voga's job.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

use crate::prefs;

/// The flowsint release the images are pinned to.
pub const VERSION: &str = "1.2.13";
const PROJECT: &str = "voga-flowsint";
const COMPOSE: &str = include_str!("compose.yml");
const SERVICES: [&str; 6] = ["postgres", "redis", "neo4j", "api", "celery", "app"];
/// The install: its port and the flowsint account Voga made (prefs, JSON).
const CONFIG: &str = "flowsint_local";
/// The account Voga creates in its own flowsint (flowsint wants an e-mail; it is never used).
pub const EMAIL: &str = "voga@example.com";
const FIRST_PORT: u16 = 5183;
pub const PROGRESS_EVENT: &str = "flowsint-local-progress";

/// Where a long job reports what it is doing (the app emits PROGRESS_EVENT).
pub type Report = dyn Fn(&Progress) + Send + Sync;

/// One install / start / stop / removal at a time.
static BUSY: Mutex<()> = Mutex::const_new(());
/// What the running job last reported, for a page opened in the middle of it.
static LAST_PROGRESS: StdMutex<Option<Progress>> = StdMutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    port: u16,
    password: String,
    /// The account exists in this flowsint.
    #[serde(default)]
    account: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// pull | start | account
    pub stage: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// No docker CLI with Compose.
    Missing,
    /// Installed, the engine is not answering.
    Stopped,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stack {
    Absent,
    Stopped,
    Starting,
    Running,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    pub engine: Engine,
    pub stack: Stack,
    /// Voga installed flowsint here (even if Docker is off now).
    pub installed: bool,
    /// The web app, when installed.
    pub url: Option<String>,
    pub email: String,
    /// To log in to the flowsint web app; only on this computer.
    pub password: Option<String>,
    /// The Docker app Voga can open (Docker Desktop, OrbStack), if any.
    pub engine_app: Option<String>,
    /// A job is running; its last progress.
    pub busy: bool,
    pub progress: Option<Progress>,
    pub version: &'static str,
}

pub fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join("flowsint")
}

async fn config(pool: &SqlitePool) -> Option<Config> {
    prefs::get(pool, CONFIG).await.ok().flatten().and_then(|json| serde_json::from_str(&json).ok())
}

async fn save_config(pool: &SqlitePool, config: &Config) -> Result<()> {
    prefs::set(pool, CONFIG, &serde_json::to_string(config)?).await?;
    Ok(())
}

// ===== The docker CLI =====

/// Where docker CLIs usually are. A desktop app does not get the shell's PATH (on macOS it is
/// only /usr/bin:/bin:/usr/sbin:/sbin), so the usual places are tried first.
fn candidates() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut list: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        list.push("/Applications/Docker.app/Contents/Resources/bin/docker".into());
        list.push(home.join(".orbstack/bin/docker"));
        list.push(home.join(".docker/bin/docker"));
        list.push("/usr/local/bin/docker".into());
        list.push("/opt/homebrew/bin/docker".into());
    }
    if cfg!(windows) {
        list.push(r"C:\Program Files\Docker\Docker\resources\bin\docker.exe".into());
    }
    if let Some(path) = std::env::var_os("PATH") {
        let name = if cfg!(windows) { "docker.exe" } else { "docker" };
        list.extend(std::env::split_paths(&path).map(|dir| dir.join(name)));
    }
    list.push("/usr/bin/docker".into());
    let mut seen = Vec::new();
    list.retain(|p| p.is_file() && !seen.contains(p) && {
        seen.push(p.clone());
        true
    });
    list
}

fn command(docker: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(docker);
    // Credential helpers and Compose plugins are found through PATH
    let mut paths: Vec<PathBuf> = docker.parent().map(Path::to_path_buf).into_iter().collect();
    paths.extend([PathBuf::from("/usr/local/bin"), PathBuf::from("/opt/homebrew/bin")]);
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    if let Ok(joined) = std::env::join_paths(paths) {
        cmd.env("PATH", joined);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs to the end: Ok(stdout), or the error with what docker said.
async fn run(mut cmd: tokio::process::Command, limit: Duration) -> Result<String> {
    let output = tokio::time::timeout(limit, cmd.output()).await.map_err(|_| anyhow!("docker did not answer in {}s", limit.as_secs()))??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let said = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        bail!("docker failed: {said}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The first docker CLI that has Compose, remembered while it stays there.
async fn docker() -> Option<PathBuf> {
    static FOUND: StdMutex<Option<PathBuf>> = StdMutex::new(None);
    if let Some(known) = FOUND.lock().ok().and_then(|f| f.clone()).filter(|p| p.is_file()) {
        return Some(known);
    }
    for candidate in candidates() {
        let mut cmd = command(&candidate);
        cmd.args(["compose", "version"]);
        if run(cmd, Duration::from_secs(10)).await.is_ok() {
            if let Ok(mut found) = FOUND.lock() {
                *found = Some(candidate.clone());
            }
            return Some(candidate);
        }
    }
    None
}

async fn engine(docker: &Path) -> Engine {
    let mut cmd = command(docker);
    cmd.args(["info", "--format", "{{.ServerVersion}}"]);
    match run(cmd, Duration::from_secs(10)).await {
        Ok(_) => Engine::Running,
        Err(_) => Engine::Stopped,
    }
}

fn compose(docker: &Path, folder: &Path) -> tokio::process::Command {
    let mut cmd = command(docker);
    cmd.arg("compose").arg("-p").arg(PROJECT);
    let file = folder.join("compose.yml");
    if file.is_file() {
        cmd.arg("-f").arg(file).arg("--env-file").arg(folder.join(".env"));
    }
    cmd
}

/// The project's containers by service: (state, health).
fn parse_ps(output: &str) -> BTreeMap<String, (String, String)> {
    // One JSON object per line (Compose ≥ 2.21) or one JSON array (older)
    let items: Vec<Value> = match serde_json::from_str::<Value>(output.trim()) {
        Ok(Value::Array(items)) => items,
        _ => output.lines().filter_map(|l| serde_json::from_str(l).ok()).collect(),
    };
    items
        .into_iter()
        .filter_map(|item| {
            let field = |name: &str| item[name].as_str().unwrap_or_default().to_lowercase();
            let service = field("Service");
            (!service.is_empty()).then(|| (service, (field("State"), field("Health"))))
        })
        .collect()
}

fn stack_of(containers: &BTreeMap<String, (String, String)>) -> Stack {
    if containers.is_empty() {
        return Stack::Absent;
    }
    let running = |service: &str| containers.get(service).is_some_and(|(state, _)| state == "running");
    if !containers.values().any(|(state, _)| state == "running") {
        return Stack::Stopped;
    }
    let api_healthy = containers.get("api").is_some_and(|(_, health)| health == "healthy");
    if SERVICES.iter().all(|s| running(s)) && api_healthy {
        Stack::Running
    } else {
        Stack::Starting
    }
}

async fn stack(docker: &Path, folder: &Path) -> Stack {
    let mut cmd = compose(docker, folder);
    cmd.args(["ps", "-a", "--format", "json"]);
    match run(cmd, Duration::from_secs(20)).await {
        Ok(output) => stack_of(&parse_ps(&output)),
        Err(e) => {
            log::warn!("flowsint: could not list the containers: {e}");
            Stack::Absent
        }
    }
}

// ===== Setup =====

fn random_bytes(n: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; n];
    SystemRandom::new().fill(&mut bytes).map_err(|_| anyhow!("no secure random source"))?;
    Ok(bytes)
}

/// Letters and digits only (passwords that survive .env files and URLs).
fn random_token(len: usize) -> Result<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut token = String::with_capacity(len);
    while token.len() < len {
        // Rejection sampling keeps every character equally likely
        for byte in random_bytes(len * 2)? {
            if (byte as usize) < ALPHABET.len() * 4 && token.len() < len {
                token.push(ALPHABET[byte as usize % ALPHABET.len()] as char);
            }
        }
    }
    Ok(token)
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn free_port() -> u16 {
    (FIRST_PORT..FIRST_PORT + 100).find(|p| port_is_free(*p)).unwrap_or(FIRST_PORT)
}

fn parse_env(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// Writes compose.yml (the current one, so a Voga update updates flowsint) and .env: the
/// secrets made on the first install are kept, the version and port follow the config.
fn write_project(folder: &Path, config: &Config) -> Result<()> {
    std::fs::create_dir_all(folder)?;
    std::fs::write(folder.join("compose.yml"), COMPOSE)?;
    let env_path = folder.join(".env");
    let mut env = std::fs::read_to_string(&env_path).map(|t| parse_env(&t)).unwrap_or_default();
    for (key, len) in [("AUTH_SECRET", 48), ("NEO4J_PASSWORD", 24), ("POSTGRES_PASSWORD", 24)] {
        if env.get(key).map_or(true, |v| v.is_empty()) {
            env.insert(key.to_string(), random_token(len)?);
        }
    }
    if env.get("MASTER_VAULT_KEY_V1").map_or(true, |v| v.is_empty()) {
        let key = base64::engine::general_purpose::STANDARD.encode(random_bytes(32)?);
        env.insert("MASTER_VAULT_KEY_V1".into(), format!("base64:{key}"));
    }
    env.insert("FLOWSINT_VERSION".into(), VERSION.into());
    env.insert("FLOWSINT_PORT".into(), config.port.to_string());
    let text: String = env.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    std::fs::write(&env_path, format!("# Made by Voga for its local flowsint. Secrets: keep this file private.\n{text}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn report(on: &Report, stage: &str, message: &str) {
    let progress = Progress { stage: stage.into(), message: message.into() };
    on(&progress);
    if let Ok(mut last) = LAST_PROGRESS.lock() {
        *last = Some(progress);
    }
}

/// Runs a long docker command, reporting each line it prints.
async fn run_reporting(on: &Report, stage: &str, mut cmd: tokio::process::Command, limit: Duration) -> Result<()> {
    let mut child = cmd.spawn().context("could not run docker")?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no docker output"))?;
    let mut lines = BufReader::new(stderr).lines();
    let mut last = String::new();
    let mut reported = std::time::Instant::now() - Duration::from_secs(1);
    let job = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            // Downloads print many lines a second: a few reports a second are enough
            if !line.is_empty() && reported.elapsed() >= Duration::from_millis(300) {
                report(on, stage, &line);
                reported = std::time::Instant::now();
            }
            if !line.is_empty() {
                last = line;
            }
        }
        child.wait().await
    };
    let status = tokio::time::timeout(limit, job).await.map_err(|_| anyhow!("docker took more than {} minutes", limit.as_secs() / 60))??;
    if !status.success() {
        bail!("docker failed: {last}");
    }
    Ok(())
}

/// Creates Voga's account in its flowsint (once).
async fn create_account(pool: &SqlitePool, config: &mut Config) -> Result<()> {
    if config.account {
        return Ok(());
    }
    let base = endpoint(config.port);
    let response = crate::net::client_for(&base)
        .post(format!("{base}/api/auth/register"))
        .json(&json!({"email": EMAIL, "password": config.password}))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .with_context(|| format!("flowsint is not answering at {base}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() && !body.contains("already registered") {
        bail!("flowsint did not create the account (HTTP {status}): {body}");
    }
    config.account = true;
    save_config(pool, config).await
}

async fn require_engine() -> Result<PathBuf> {
    let docker = docker().await.ok_or_else(|| anyhow!("Docker is not installed. Install Docker Desktop or OrbStack to run flowsint on this computer."))?;
    if engine(&docker).await != Engine::Running {
        bail!("Docker is not running. Open Docker Desktop (or OrbStack) and try again.");
    }
    Ok(docker)
}

/// Installs (downloading the images) or starts flowsint and waits until it answers.
async fn up(on: &Report, pool: &SqlitePool, data_dir: &Path, pull: bool) -> Result<Config> {
    let docker = require_engine().await?;
    let folder = folder(data_dir);
    let mut config = match config(pool).await {
        Some(config) => config,
        None => {
            // A first install: data left by an earlier one (its passwords are gone) would not open
            let mut cmd = compose(&docker, &folder);
            cmd.args(["down", "--volumes", "--remove-orphans"]);
            if let Err(e) = run(cmd, Duration::from_secs(300)).await {
                log::warn!("flowsint: could not clear an earlier install: {e}");
            }
            let _ = std::fs::remove_file(folder.join(".env"));
            let config = Config { port: free_port(), password: random_token(24)?, account: false };
            save_config(pool, &config).await?;
            config
        }
    };
    write_project(&folder, &config)?;
    if pull {
        report(on, "pull", "");
        let mut cmd = compose(&docker, &folder);
        cmd.args(["--progress", "plain", "pull"]);
        run_reporting(on, "pull", cmd, Duration::from_secs(60 * 60)).await?;
    }
    report(on, "start", "");
    let mut cmd = compose(&docker, &folder);
    cmd.args(["up", "-d", "--wait", "--wait-timeout", "600", "--pull", "missing"]);
    run_reporting(on, "start", cmd, Duration::from_secs(15 * 60)).await?;
    report(on, "account", "");
    create_account(pool, &mut config).await?;
    Ok(config)
}

/// Clears the last progress when a job ends, however it ends.
struct JobDone;
impl Drop for JobDone {
    fn drop(&mut self) {
        if let Ok(mut last) = LAST_PROGRESS.lock() {
            *last = None;
        }
    }
}

async fn job<T>(work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    let _busy = BUSY.try_lock().map_err(|_| anyhow!("flowsint is already being set up; wait for it to finish"))?;
    let _done = JobDone;
    work.await
}

/// For sending: the local flowsint's address and account, starting it if needed.
pub async fn ensure_running(on: &Report, pool: &SqlitePool, data_dir: &Path) -> Result<(String, String, String)> {
    let Some(config) = config(pool).await else {
        bail!("flowsint is not installed on this computer yet: install it in Settings › flowsint");
    };
    let docker = require_engine().await?;
    let config = if config.account && stack(&docker, &folder(data_dir)).await == Stack::Running {
        config
    } else {
        job(up(on, pool, data_dir, false)).await?
    };
    Ok((endpoint(config.port), EMAIL.to_string(), config.password))
}

pub async fn status(pool: &SqlitePool, data_dir: &Path) -> LocalStatus {
    let config = config(pool).await;
    let docker = docker().await;
    let engine = match &docker {
        Some(docker) => engine(docker).await,
        None => Engine::Missing,
    };
    let stack = match (&docker, engine) {
        (Some(docker), Engine::Running) => stack(docker, &folder(data_dir)).await,
        _ => Stack::Absent,
    };
    LocalStatus {
        engine,
        stack,
        installed: config.is_some(),
        url: config.as_ref().map(|c| endpoint(c.port)),
        email: EMAIL.to_string(),
        password: config.map(|c| c.password),
        engine_app: engine_app().map(|(name, _)| name.to_string()),
        busy: BUSY.try_lock().is_err(),
        progress: LAST_PROGRESS.lock().ok().and_then(|p| p.clone()),
        version: VERSION,
    }
}

pub async fn install(on: &Report, pool: &SqlitePool, data_dir: &Path) -> Result<()> {
    job(up(on, pool, data_dir, true)).await.map(|_| ())
}

pub async fn start(on: &Report, pool: &SqlitePool, data_dir: &Path) -> Result<()> {
    job(up(on, pool, data_dir, false)).await.map(|_| ())
}

pub async fn stop(data_dir: &Path) -> Result<()> {
    job(async {
        let docker = require_engine().await?;
        let mut cmd = compose(&docker, &folder(data_dir));
        cmd.arg("stop");
        run(cmd, Duration::from_secs(180)).await.map(|_| ())
    })
    .await
}

/// Removes the containers, their data and the images; the next install starts from scratch.
pub async fn remove(pool: &SqlitePool, data_dir: &Path) -> Result<()> {
    job(async {
        let docker = require_engine().await?;
        let folder = folder(data_dir);
        let mut cmd = compose(&docker, &folder);
        cmd.args(["down", "--volumes", "--rmi", "all", "--remove-orphans"]);
        run(cmd, Duration::from_secs(600)).await?;
        if folder.exists() {
            std::fs::remove_dir_all(&folder)?;
        }
        sqlx::query("DELETE FROM prefs WHERE key = ?").bind(CONFIG).execute(pool).await?;
        Ok(())
    })
    .await
}

/// The Docker app on this computer that Voga can open: (name, how to open it).
fn engine_app() -> Option<(&'static str, PathBuf)> {
    let apps: &[(&str, &str)] = if cfg!(target_os = "macos") {
        &[("Docker Desktop", "/Applications/Docker.app"), ("OrbStack", "/Applications/OrbStack.app")]
    } else if cfg!(windows) {
        &[("Docker Desktop", r"C:\Program Files\Docker\Docker\Docker Desktop.exe")]
    } else {
        &[]
    };
    apps.iter().map(|(name, path)| (*name, PathBuf::from(path))).find(|(_, path)| path.exists())
}

/// Opens Docker Desktop or OrbStack; the engine takes a little while to answer.
pub fn open_engine() -> Result<()> {
    let (name, path) = engine_app().ok_or_else(|| anyhow!("Start the Docker engine (on Linux: sudo systemctl start docker)"))?;
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg("-a").arg(&path).spawn()
    } else {
        std::process::Command::new(&path).spawn()
    };
    result.map(|_| ()).with_context(|| format!("could not open {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_list_in_both_compose_formats() {
        let lines = concat!(
            r#"{"Service":"api","State":"running","Health":"healthy"}"#,
            "\n",
            r#"{"Service":"neo4j","State":"running","Health":"starting"}"#,
            "\n"
        );
        let parsed = parse_ps(lines);
        assert_eq!(parsed["api"], ("running".to_string(), "healthy".to_string()));
        assert_eq!(parsed.len(), 2);
        let array = r#"[{"Service":"app","State":"exited","Health":""}]"#;
        assert_eq!(parse_ps(array)["app"].0, "exited");
        assert!(parse_ps("").is_empty());
    }

    #[test]
    fn stack_state_from_containers() {
        let all = |state: &str, api: &str| -> BTreeMap<String, (String, String)> {
            SERVICES
                .iter()
                .map(|s| (s.to_string(), (state.to_string(), if *s == "api" { api.to_string() } else { String::new() })))
                .collect()
        };
        assert_eq!(stack_of(&BTreeMap::new()), Stack::Absent);
        assert_eq!(stack_of(&all("exited", "")), Stack::Stopped);
        assert_eq!(stack_of(&all("running", "starting")), Stack::Starting);
        assert_eq!(stack_of(&all("running", "healthy")), Stack::Running);
        let mut missing_app = all("running", "healthy");
        missing_app.remove("app");
        assert_eq!(stack_of(&missing_app), Stack::Starting);
    }

    #[test]
    fn project_files_keep_their_secrets() {
        let dir = std::env::temp_dir().join(format!("voga-flowsint-test-{}", std::process::id()));
        let config = Config { port: 5190, password: "x".into(), account: false };
        write_project(&dir, &config).unwrap();
        let first = parse_env(&std::fs::read_to_string(dir.join(".env")).unwrap());
        assert_eq!(first["FLOWSINT_PORT"], "5190");
        assert_eq!(first["FLOWSINT_VERSION"], VERSION);
        assert_eq!(first["AUTH_SECRET"].len(), 48);
        assert!(first["MASTER_VAULT_KEY_V1"].starts_with("base64:"));
        assert!(std::fs::read_to_string(dir.join("compose.yml")).unwrap().contains("127.0.0.1:${FLOWSINT_PORT}:8080"));

        // A new port keeps the secrets
        write_project(&dir, &Config { port: 5191, ..config }).unwrap();
        let second = parse_env(&std::fs::read_to_string(dir.join(".env")).unwrap());
        assert_eq!(second["FLOWSINT_PORT"], "5191");
        assert_eq!(second["AUTH_SECRET"], first["AUTH_SECRET"]);
        assert_eq!(second["NEO4J_PASSWORD"], first["NEO4J_PASSWORD"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tokens_are_alphanumeric() {
        let token = random_token(32).unwrap();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(token, random_token(32).unwrap());
    }
}
