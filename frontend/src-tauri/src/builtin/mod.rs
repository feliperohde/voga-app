//! The built-in models: Voga's default for answers, analysis and the search index, so it works
//! without any server. GGUF models are downloaded into <app data>/models (files.rs) and run by
//! llama.cpp in the llama-helper sidecar (helper.rs); Whisper and the voice model are run by
//! src/live in this process.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

pub mod catalog;
pub mod commands;
pub mod files;
mod helper;

use catalog::ModelKind;

/// Built-in embedding model of a new installation (multilingual, good for Portuguese).
pub const DEFAULT_EMBEDDING_MODEL: &str = "bge-m3-q8";

/// Longest answer, in tokens.
const MAX_TOKENS: u32 = 4096;
const GENERATE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const EMBED_TIMEOUT: Duration = Duration::from_secs(5 * 60);

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The app data folder, whose `models/` holds the downloaded models. Set at startup.
pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

pub fn data_dir() -> Result<&'static PathBuf> {
    DATA_DIR.get().ok_or_else(|| anyhow!("The built-in models folder is not set"))
}

/// Whether a built-in model's whole file is on this computer.
pub fn is_downloaded(model: &str) -> bool {
    catalog::find(model).is_some_and(|model| files::status(model) == files::Status::Available)
}

/// The first model of a kind downloaded, recommended first.
pub fn downloaded_of_kind(kind: ModelKind) -> Option<String> {
    catalog::of_kind(kind).find(|model| is_downloaded(model.name)).map(|model| model.name.to_string())
}

/// The best built-in chat model downloaded, if any.
pub fn downloaded_chat_model() -> Option<String> {
    downloaded_of_kind(ModelKind::Summary)
}

/// Where a built-in model's file is.
pub fn model_file(model: &str) -> Result<PathBuf> {
    files::path(catalog::find(model).ok_or_else(|| anyhow!("Unknown built-in model: {model}"))?)
}

fn ready(name: &str, kind: ModelKind) -> Result<&'static catalog::Model> {
    let model = catalog::find(name).filter(|m| m.kind == kind).ok_or_else(|| anyhow!("Unknown built-in model: {name}"))?;
    if !is_downloaded(name) {
        bail!("The built-in model {} is not downloaded. Download it in Settings › AI.", model.label);
    }
    Ok(model)
}

/// One answer from a built-in chat model. `temperature` overrides the model's (0: the same
/// answer every time).
pub async fn generate(name: &str, system: &str, user: &str, temperature: Option<f32>) -> Result<String> {
    let model = ready(name, ModelKind::Summary)?;
    let chat = model.chat.context("not a chat model")?;
    let mut sampling = chat.sampling;
    if let Some(temperature) = temperature {
        sampling.temperature = temperature;
    }
    let request = json!({
        "op": "generate",
        "model": files::path(model)?,
        "prompt": chat.template.render(system, user),
        "context": model.context,
        "max_tokens": MAX_TOKENS,
        "stop": [chat.template.stop()],
        "sampling": sampling,
    });
    let reply = helper::CHAT.call(&request, GENERATE_TIMEOUT).await?;
    reply.get("text").and_then(Value::as_str).map(str::to_string).ok_or_else(|| anyhow!("llama-helper gave no text"))
}

/// One L2-normalized vector per text, from a built-in embedding model.
pub async fn embed(name: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let model = ready(name, ModelKind::Embedding)?;
    let request = json!({ "op": "embed", "model": files::path(model)?, "texts": texts, "context": model.context });
    let reply = helper::EMBEDDINGS.call(&request, EMBED_TIMEOUT).await?;
    let vectors: Vec<Vec<f32>> = serde_json::from_value(reply.get("vectors").cloned().unwrap_or_default()).context("llama-helper's vectors")?;
    if vectors.len() != texts.len() {
        bail!("llama-helper gave {} vectors for {} texts", vectors.len(), texts.len());
    }
    Ok(vectors)
}

/// Stops the sidecars (Voga is quitting).
pub fn stop_helpers() {
    helper::CHAT.kill();
    helper::EMBEDDINGS.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real run: VOGA_LLAMA_HELPER (cargo build -p llama-helper --release) and VOGA_DATA_DIR, an
    /// app data folder whose models/ has bge-m3-Q8_0.gguf and Qwen3.5-4B-Q4_K_M.gguf.
    ///   cargo test --lib builtin::tests::built_in_models_answer -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn built_in_models_answer() {
        set_data_dir(std::env::var("VOGA_DATA_DIR").expect("VOGA_DATA_DIR").into());
        let texts = [
            "O deputado votou contra a reforma da previdência.",
            "Na votação, o parlamentar rejeitou a reforma previdenciária.",
            "Receita de bolo de cenoura com cobertura de chocolate.",
        ]
        .map(String::from);
        let vectors = embed(DEFAULT_EMBEDDING_MODEL, &texts).await.unwrap();
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(dot(&vectors[0], &vectors[1]) > dot(&vectors[0], &vectors[2]) + 0.2);

        let answer = generate("qwen3.5:4b", "Responda com uma palavra.", "Qual é a capital do Brasil?", Some(0.0)).await.unwrap();
        println!("{answer}");
        assert!(answer.contains("Brasília"), "{answer}");
        stop_helpers();
    }
}
