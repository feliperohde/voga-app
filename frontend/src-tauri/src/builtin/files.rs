//! The models' files in <app data>/models: their state, downloads (resumed where a partial file
//! stopped, with `builtin-ai-download-progress` events for the UI) and deletion.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

use super::catalog::Model;

const EVENT: &str = "builtin-ai-download-progress";
/// What a cancelled download fails with; the UI does not report it as an error.
pub const CANCELLED: &str = "CANCELLED";
const MB: f64 = 1024.0 * 1024.0;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Status {
    NotDownloaded,
    Downloading { progress: u8 },
    Available,
    /// A file of the wrong size: a download that stopped halfway (downloading resumes it)
    Corrupted,
}

struct Active {
    progress: u8,
    cancel: CancellationToken,
}

static ACTIVE: LazyLock<Mutex<HashMap<&'static str, Active>>> = LazyLock::new(Default::default);

pub fn path(model: &Model) -> Result<PathBuf> {
    Ok(super::data_dir()?.join("models").join(model.file))
}

/// The catalog's sizes are rounded: within 10% is the whole file.
fn whole(bytes: u64, model: &Model) -> bool {
    let mb = bytes as f64 / MB;
    let expected = model.size_mb as f64;
    (expected * 0.9..=expected * 1.1).contains(&mb)
}

pub fn status(model: &Model) -> Status {
    if let Some(active) = ACTIVE.lock().unwrap().get(model.name) {
        return Status::Downloading { progress: active.progress };
    }
    match path(model).and_then(|p| Ok(std::fs::metadata(p)?.len())) {
        Ok(bytes) if whole(bytes, model) => Status::Available,
        Ok(_) => Status::Corrupted,
        Err(_) => Status::NotDownloaded,
    }
}

/// Downloads `model`, resuming a partial file. Fails with CANCELLED when cancelled (the partial
/// file is kept for next time).
pub async fn download<R: Runtime>(app: &AppHandle<R>, model: &'static Model) -> Result<(), String> {
    let cancel = CancellationToken::new();
    {
        let mut active = ACTIVE.lock().unwrap();
        if active.contains_key(model.name) {
            return Err(format!("{} is already downloading", model.label));
        }
        active.insert(model.name, Active { progress: 0, cancel: cancel.clone() });
    }
    let emit = |payload: serde_json::Value| {
        let _ = app.emit(EVENT, payload);
    };
    let result = match path(model) {
        Ok(file) => fetch(model, &file, &cancel, &emit).await,
        Err(error) => Err(error),
    };
    ACTIVE.lock().unwrap().remove(model.name);

    let (status, error) = match &result {
        Ok(()) => ("completed", None),
        Err(_) if cancel.is_cancelled() => ("cancelled", None),
        Err(error) => ("error", Some(format!("{error:#}"))),
    };
    let progress = if status == "completed" { 100 } else { 0 };
    emit(json!({ "model": model.name, "progress": progress, "status": status, "error": error }));
    match (result, cancel.is_cancelled()) {
        (Ok(()), _) => Ok(()),
        (Err(_), true) => Err(CANCELLED.to_string()),
        (Err(error), false) => Err(format!("{error:#}")),
    }
}

/// Downloads `model` into `file`, sending progress to `emit`.
async fn fetch(model: &Model, file: &Path, cancel: &CancellationToken, emit: &(dyn Fn(serde_json::Value) + Sync)) -> Result<()> {
    tokio::fs::create_dir_all(file.parent().context("models folder")?).await?;
    let mut have = tokio::fs::metadata(file).await.map(|m| m.len()).unwrap_or(0);
    if whole(have, model) {
        return check(file, model).await;
    }
    if have as f64 / MB > model.size_mb as f64 * 1.1 {
        tokio::fs::remove_file(file).await?;
        have = 0;
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("Voga/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let mut request = client.get(model.url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let response = request.send().await.with_context(|| format!("downloading {}", model.url))?;
    let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() {
        bail!("{} answered {}", model.url, response.status());
    }
    if !resumed {
        have = 0;
    }
    let total = have + response.content_length().unwrap_or(0);
    let out = tokio::fs::OpenOptions::new().create(true).write(true).append(resumed).truncate(!resumed).open(file).await?;
    let mut out = BufWriter::with_capacity(1 << 20, out);

    let mut body = response.bytes_stream();
    let (mut window_start, mut window_bytes, mut shown) = (Instant::now(), 0u64, u8::MAX);
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => None,
            next = tokio::time::timeout(Duration::from_secs(60), body.next()) => {
                Some(next.map_err(|_| anyhow!("the download stalled: nothing arrived for a minute"))?)
            }
        };
        let Some(chunk) = chunk else {
            out.flush().await?;
            bail!(CANCELLED);
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk?;
        out.write_all(&chunk).await?;
        have += chunk.len() as u64;
        window_bytes += chunk.len() as u64;

        let progress = if total > 0 { (have * 100 / total).min(100) as u8 } else { 0 };
        let elapsed = window_start.elapsed();
        if progress != shown || elapsed >= Duration::from_millis(500) {
            if let Some(active) = ACTIVE.lock().unwrap().get_mut(model.name) {
                active.progress = progress;
            }
            let speed = window_bytes as f64 / MB / elapsed.as_secs_f64().max(0.001);
            emit(json!({
                "model": model.name,
                "progress": progress,
                "downloaded_mb": have as f64 / MB,
                "total_mb": total as f64 / MB,
                "speed_mbps": speed,
                "status": "downloading",
            }));
            (window_start, window_bytes, shown) = (Instant::now(), 0, progress);
        }
    }
    out.flush().await?;
    drop(out);
    if !whole(have, model) {
        // Kept: the next download resumes it
        bail!("the download ended early, at {:.0} of {} MB", have as f64 / MB, model.size_mb);
    }
    if let Err(error) = check(file, model).await {
        let _ = tokio::fs::remove_file(file).await;
        return Err(error);
    }
    Ok(())
}

/// The file starts like files of its kind (an HTML error page saved as a model does not).
async fn check(file: &Path, model: &Model) -> Result<()> {
    let Some(accepted) = model.kind.magic() else { return Ok(()) };
    let mut start = [0u8; 4];
    tokio::fs::File::open(file).await?.read_exact(&mut start).await?;
    if accepted.contains(&start) {
        Ok(())
    } else {
        bail!("{} is not a {:?} model file (it starts with {:?})", model.file, model.kind, String::from_utf8_lossy(&start))
    }
}

pub fn cancel(name: &str) {
    if let Some(active) = ACTIVE.lock().unwrap().get(name) {
        active.cancel.cancel();
    }
}

pub async fn delete(model: &Model) -> Result<()> {
    cancel(model.name);
    match tokio::fs::remove_file(path(model)?).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::catalog;

    #[test]
    fn a_whole_file_is_within_ten_percent_of_the_catalog() {
        let model = catalog::find("bge-m3-q8").unwrap();
        let mb = |n: f64| (n * MB) as u64;
        assert!(whole(mb(605.0), model));
        assert!(whole(mb(560.0), model));
        assert!(!whole(mb(300.0), model), "half a download");
        assert!(!whole(mb(700.0), model));
    }

    /// Real download (139 MB from Hugging Face): cancelled halfway, then resumed where it stopped.
    ///   cargo test --lib builtin::files::tests::downloads_resume_after_a_cancel -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn downloads_resume_after_a_cancel() {
        let model = catalog::find("nomic-embed-text-v1.5-q8").unwrap();
        let dir = std::env::temp_dir().join(format!("voga-download-{}", std::process::id()));
        let file = dir.join("models").join(model.file);
        let events = Mutex::new(Vec::new());
        let cancel = CancellationToken::new();
        // Cancelled once 20 MB are in
        let emit = |event: serde_json::Value| {
            if event["downloaded_mb"].as_f64().unwrap_or(0.0) >= 20.0 {
                cancel.cancel();
            }
            events.lock().unwrap().push(event);
        };
        let error = fetch(model, &file, &cancel, &emit).await.unwrap_err().to_string();
        assert_eq!(error, CANCELLED);
        let partial = std::fs::metadata(&file).unwrap().len();
        assert!(partial > 0 && !whole(partial, model), "{partial} bytes kept");

        events.lock().unwrap().clear();
        let record = |event: serde_json::Value| events.lock().unwrap().push(event);
        fetch(model, &file, &CancellationToken::new(), &record).await.unwrap();
        let first = events.lock().unwrap()[0]["downloaded_mb"].as_f64().unwrap();
        assert!(first >= partial as f64 / MB, "resumed at {first} MB");
        assert!(whole(std::fs::metadata(&file).unwrap().len(), model));
        println!("cancelled at {:.1} MB, resumed and finished", partial as f64 / MB);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn status_reports_the_ui_shape() {
        assert_eq!(serde_json::to_value(Status::Downloading { progress: 42 }).unwrap(), json!({"type": "downloading", "progress": 42}));
        assert_eq!(serde_json::to_value(Status::NotDownloaded).unwrap(), json!({"type": "not_downloaded"}));
        assert_eq!(serde_json::to_value(Status::Corrupted).unwrap(), json!({"type": "corrupted"}));
    }
}
