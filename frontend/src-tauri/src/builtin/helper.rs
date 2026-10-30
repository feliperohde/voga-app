//! The llama-helper processes (llama-helper/src/main.rs): one for chat and one for embeddings, so
//! indexing never waits behind an answer. Each starts with its first request, answers one request
//! at a time, and is killed after five idle minutes, when a request runs out of time (llama.cpp
//! cannot be stopped any other way) and when Voga quits.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex as SyncMutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

const IDLE: Duration = Duration::from_secs(5 * 60);

pub static CHAT: LazyLock<Helper> = LazyLock::new(|| Helper::new("chat"));
pub static EMBEDDINGS: LazyLock<Helper> = LazyLock::new(|| Helper::new("embeddings"));

struct Pipes {
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

pub struct Helper {
    name: &'static str,
    /// Held for a whole request: one at a time
    pipes: Mutex<Option<Pipes>>,
    /// Apart from the pipes, so it can be killed while a request waits on it
    child: SyncMutex<Option<Child>>,
    last_used: SyncMutex<Instant>,
    reaper: AtomicBool,
}

impl Helper {
    fn new(name: &'static str) -> Self {
        Helper { name, pipes: Mutex::new(None), child: SyncMutex::new(None), last_used: SyncMutex::new(Instant::now()), reaper: AtomicBool::new(false) }
    }

    /// Sends one request and returns its reply, or the error it reports.
    pub async fn call(&'static self, request: &Value, timeout: Duration) -> Result<Value> {
        let mut pipes = self.pipes.lock().await;
        if pipes.is_none() || self.child.lock().unwrap().is_none() {
            *pipes = Some(self.start()?);
        }
        let running = pipes.as_mut().expect("started above");
        let line = format!("{request}\n");
        let exchange = async {
            running.stdin.write_all(line.as_bytes()).await?;
            running.stdin.flush().await?;
            running.stdout.next_line().await
        };
        let reply = tokio::time::timeout(timeout, exchange).await;
        *self.last_used.lock().unwrap() = Instant::now();

        let reply = match reply {
            Ok(Ok(Some(reply))) => reply,
            outcome => {
                *pipes = None;
                self.kill();
                return Err(match outcome {
                    Err(_) => anyhow!("the built-in model gave no answer in {} minutes", timeout.as_secs() / 60),
                    Ok(Err(error)) => anyhow!("the built-in model stopped: {error}"),
                    Ok(Ok(_)) => anyhow!("the built-in model stopped (out of memory?)"),
                });
            }
        };
        let reply: Value = serde_json::from_str(&reply).with_context(|| format!("llama-helper replied {reply}"))?;
        match reply.get("error").and_then(Value::as_str) {
            Some(error) => bail!("{error}"),
            None => Ok(reply),
        }
    }

    fn start(&'static self) -> Result<Pipes> {
        let binary = binary()?;
        // Below the app's priority: the window stays responsive while a model works
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("nice");
            command.arg("-n").arg("10").arg(&binary);
            command
        };
        #[cfg(windows)]
        let mut command = {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
            let mut command = Command::new(&binary);
            command.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
            command
        };
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", binary.display()))?;
        let pipes = Pipes {
            stdin: child.stdin.take().context("llama-helper has no stdin")?,
            stdout: BufReader::new(child.stdout.take().context("llama-helper has no stdout")?).lines(),
        };
        *self.child.lock().unwrap() = Some(child);
        log::info!("built-in models: {} helper started ({})", self.name, binary.display());
        if !self.reaper.swap(true, Ordering::SeqCst) {
            tokio::spawn(self.reap());
        }
        Ok(pipes)
    }

    /// Kills the process once it sits idle (a request in progress holds the pipes).
    async fn reap(&'static self) {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let Ok(mut pipes) = self.pipes.try_lock() else { continue };
            if pipes.is_some() && self.last_used.lock().unwrap().elapsed() > IDLE {
                *pipes = None;
                self.kill();
                log::info!("built-in models: {} helper stopped after {} idle minutes", self.name, IDLE.as_secs() / 60);
            }
        }
    }

    /// Kills the process now, even in the middle of a request (which then fails).
    pub fn kill(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.start_kill();
        }
    }
}

/// VOGA_LLAMA_HELPER, else next to the app's executable (where the bundle and `tauri dev` put
/// the sidecar, without its target triple), or one folder up (tests run from target/*/deps).
fn binary() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("VOGA_LLAMA_HELPER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Ok(path);
    }
    let name = format!("llama-helper{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().context("finding the app's executable")?;
    exe.ancestors()
        .skip(1)
        .take(2)
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
        .ok_or_else(|| anyhow!("{name} is missing next to {}: build it with `cargo build -p llama-helper --release` or set VOGA_LLAMA_HELPER", exe.display()))
}
