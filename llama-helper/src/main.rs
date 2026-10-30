//! Voga's built-in models: llama.cpp in a process of its own, so a model that crashes or runs out
//! of memory never takes the app with it. One JSON request per line on stdin, one JSON reply per
//! line on stdout; the log goes to stderr.
//!
//!   {"op":"generate","model":"<gguf>","prompt":"…","context":32768,"max_tokens":4096,
//!    "stop":["<|im_end|>"],"sampling":{"temperature":0.5,"top_k":20,"top_p":0.8,
//!    "presence":0.3,"frequency":0.0,"repeat":1.05,"last_n":256}}      → {"text":"…"}
//!   {"op":"embed","model":"<gguf>","texts":["…"],"context":2048}     → {"vectors":[[…]]}
//!   a request that fails                                              → {"error":"…"}
//!
//! Each request names its model; the last one used for each op stays loaded. The app
//! (frontend/src-tauri/src/builtin/helper.rs) starts this on demand and stops it when idle.
//! Built with --features metal|cuda|vulkan, every layer goes to the GPU, else to the processor.

use std::io::{self, BufRead, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::TokenToStringError;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Generate {
        model: PathBuf,
        prompt: String,
        context: u32,
        max_tokens: u32,
        #[serde(default)]
        stop: Vec<String>,
        #[serde(default)]
        sampling: Sampling,
    },
    Embed {
        model: PathBuf,
        texts: Vec<String>,
        context: u32,
    },
}

/// llama.cpp's samplers. Temperature 0 picks the likeliest token every time.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
struct Sampling {
    temperature: f32,
    top_k: i32,
    top_p: f32,
    presence: f32,
    frequency: f32,
    repeat: f32,
    /// How many recent tokens the penalties look at; 0 turns them off.
    last_n: i32,
}

impl Default for Sampling {
    fn default() -> Self {
        Sampling { temperature: 0.0, top_k: 40, top_p: 1.0, presence: 0.0, frequency: 0.0, repeat: 1.0, last_n: 0 }
    }
}

impl Sampling {
    fn penalized(&self) -> bool {
        self.last_n > 0 && (self.presence != 0.0 || self.frequency != 0.0 || self.repeat != 1.0)
    }

    fn sampler(&self, seed: u32) -> LlamaSampler {
        let mut chain = Vec::new();
        if self.penalized() {
            chain.push(LlamaSampler::penalties(self.last_n, self.repeat, self.frequency, self.presence));
        }
        if self.temperature > 0.0 {
            if self.top_k > 0 {
                chain.push(LlamaSampler::top_k(self.top_k));
            }
            if self.top_p > 0.0 && self.top_p < 1.0 {
                chain.push(LlamaSampler::top_p(self.top_p, 1));
            }
            chain.push(LlamaSampler::temp(self.temperature));
            chain.push(LlamaSampler::dist(seed));
        } else {
            chain.push(LlamaSampler::greedy());
        }
        LlamaSampler::chain_simple(chain)
    }
}

struct Loaded {
    path: PathBuf,
    model: LlamaModel,
}

struct Engine {
    backend: LlamaBackend,
    chat: Option<Loaded>,
    embedder: Option<Loaded>,
    threads: i32,
}

/// The model at `path` in `slot`, loading it when the slot holds another one (or none).
fn load<'a>(backend: &LlamaBackend, slot: &'a mut Option<Loaded>, path: &Path) -> Result<&'a LlamaModel> {
    if slot.as_ref().is_some_and(|loaded| loaded.path == path) {
        return Ok(&slot.as_ref().expect("checked above").model);
    }
    *slot = None;
    if !path.is_file() {
        bail!("no model file at {}", path.display());
    }
    let started = Instant::now();
    let on_gpu = cfg!(any(feature = "metal", feature = "cuda", feature = "vulkan"));
    let model = match LlamaModel::load_from_file(backend, path, &LlamaModelParams::default().with_n_gpu_layers(if on_gpu { 999 } else { 0 })) {
        Ok(model) => model,
        // Not enough GPU memory: the processor is slower but works
        Err(error) if on_gpu => {
            eprintln!("loading {} on the GPU failed ({error}), trying the processor", path.display());
            LlamaModel::load_from_file(backend, path, &LlamaModelParams::default().with_n_gpu_layers(0))
                .with_context(|| format!("loading {}", path.display()))?
        }
        Err(error) => return Err(error).with_context(|| format!("loading {}", path.display())),
    };
    eprintln!("loaded {} in {:.1}s", path.display(), started.elapsed().as_secs_f32());
    Ok(&slot.insert(Loaded { path: path.to_path_buf(), model }).model)
}

/// The text of one token, special tokens included (a stop string can be one).
fn piece(model: &LlamaModel, token: LlamaToken) -> Result<Vec<u8>> {
    match model.token_to_piece_bytes(token, 64, true, None) {
        Err(TokenToStringError::InsufficientBufferSpace(needed)) => {
            Ok(model.token_to_piece_bytes(token, needed.unsigned_abs() as usize, true, None)?)
        }
        other => Ok(other?),
    }
}

fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

impl Engine {
    fn generate(&mut self, model: &Path, prompt: &str, context: u32, max_tokens: u32, stop: &[String], sampling: &Sampling) -> Result<String> {
        let started = Instant::now();
        let threads = self.threads;
        let model = load(&self.backend, &mut self.chat, model)?;
        let tokens = model.str_to_token(prompt, AddBos::Always)?;
        let limit = context.min(model.n_ctx_train().max(512)) as usize;
        if tokens.is_empty() || tokens.len() >= limit {
            bail!("the prompt has {} tokens; this model reads at most {limit}", tokens.len());
        }
        // A context as long as this answer can get, not the model's whole window: less memory
        let wanted = (tokens.len() + max_tokens as usize).min(limit);
        let n_ctx = wanted.div_ceil(256) * 256;
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx as u32))
            .with_n_batch(n_ctx as u32)
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let mut ctx = model.new_context(&self.backend, params)?;

        let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
        let last = tokens.len() - 1;
        for (i, token) in tokens.iter().enumerate() {
            batch.add(*token, i as i32, &[0], i == last)?;
        }
        ctx.decode(&mut batch)?;
        let read = started.elapsed();

        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let mut sampler = sampling.sampler(seed);
        let mut text: Vec<u8> = Vec::new();
        let mut position_now = tokens.len();
        let mut produced = 0;
        while produced < max_tokens as usize && position_now < wanted {
            // sample() also accepts the token, which the penalties count
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            if model.is_eog_token(token) {
                break;
            }
            let start = text.len();
            text.extend(piece(model, token)?);
            produced += 1;
            if let Some(cut) = stop.iter().filter(|s| !s.is_empty()).find_map(|s| {
                let from = start.saturating_sub(s.len() - 1);
                position(&text[from..], s.as_bytes()).map(|at| from + at)
            }) {
                text.truncate(cut);
                break;
            }
            batch.clear();
            batch.add(token, position_now as i32, &[0], true)?;
            position_now += 1;
            ctx.decode(&mut batch)?;
        }

        let writing = started.elapsed() - read;
        eprintln!(
            "generate: {} tokens read in {:.1}s, {produced} written in {:.1}s ({:.1}/s)",
            tokens.len(),
            read.as_secs_f32(),
            writing.as_secs_f32(),
            produced as f32 / writing.as_secs_f32().max(0.001),
        );
        Ok(String::from_utf8_lossy(&text).trim_end().to_string())
    }

    /// One L2-normalized vector per text; longer texts are cut at the context.
    fn embed(&mut self, model: &Path, texts: &[String], context: u32) -> Result<Vec<Vec<f32>>> {
        let started = Instant::now();
        let threads = self.threads;
        let model = load(&self.backend, &mut self.embedder, model)?;
        let n_ctx = context.min(model.n_ctx_train()).max(64);
        // Encoder models read the whole text in one micro-batch
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(n_ctx)
            .with_n_ubatch(n_ctx)
            .with_n_threads(threads)
            .with_n_threads_batch(threads)
            .with_embeddings(true);
        let mut ctx = model.new_context(&self.backend, params)?;
        let mut batch = LlamaBatch::new(n_ctx as usize, 1);

        let mut vectors = Vec::with_capacity(texts.len());
        for text in texts {
            let mut tokens = model.str_to_token(text, AddBos::Always)?;
            tokens.truncate(n_ctx as usize);
            if tokens.is_empty() {
                vectors.push(vec![0.0; model.n_embd() as usize]);
                continue;
            }
            batch.clear();
            let last = tokens.len() - 1;
            for (i, token) in tokens.into_iter().enumerate() {
                batch.add(token, i as i32, &[0], i == last)?;
            }
            ctx.clear_kv_cache();
            ctx.decode(&mut batch)?;
            let pooled = ctx.embeddings_seq_ith(0).context("the model gave no pooled embedding")?;
            let norm = pooled.iter().map(|x| x * x).sum::<f32>().sqrt().max(f32::MIN_POSITIVE);
            vectors.push(pooled.iter().map(|x| x / norm).collect());
        }
        eprintln!("embed: {} texts in {:.1}s", texts.len(), started.elapsed().as_secs_f32());
        Ok(vectors)
    }

    fn answer(&mut self, line: &str) -> Value {
        let reply = match serde_json::from_str::<Request>(line) {
            Err(error) => Err(anyhow::anyhow!("bad request: {error}")),
            Ok(Request::Generate { model, prompt, context, max_tokens, stop, sampling }) => {
                self.generate(&model, &prompt, context, max_tokens, &stop, &sampling).map(|text| json!({ "text": text }))
            }
            Ok(Request::Embed { model, texts, context }) => {
                self.embed(&model, &texts, context).map(|vectors| json!({ "vectors": vectors }))
            }
        };
        reply.unwrap_or_else(|error| json!({ "error": format!("{error:#}") }))
    }
}

fn main() -> Result<()> {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    // Leave a quarter of the processor to the app and the system
    let threads = (cores - cores / 4).max(1) as i32;
    let mut engine = Engine { backend: LlamaBackend::init()?, chat: None, embedder: None, threads };
    eprintln!("llama-helper ready ({threads} threads)");

    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        writeln!(stdout, "{}", engine.answer(&line))?;
        stdout.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_with_defaults() {
        let generate: Request = serde_json::from_str(
            r#"{"op":"generate","model":"/m.gguf","prompt":"oi","context":4096,"max_tokens":16}"#,
        )
        .unwrap();
        let Request::Generate { stop, sampling, .. } = generate else { panic!("generate expected") };
        assert!(stop.is_empty());
        assert_eq!(sampling, Sampling::default());
        assert!(!sampling.penalized());

        let embed: Request = serde_json::from_str(r#"{"op":"embed","model":"/e.gguf","texts":["a","b"],"context":2048}"#).unwrap();
        assert!(matches!(embed, Request::Embed { texts, .. } if texts == ["a", "b"]));
        assert!(serde_json::from_str::<Request>(r#"{"op":"dance"}"#).is_err());
    }

    #[test]
    fn penalties_need_a_window_and_a_strength() {
        let mut sampling: Sampling = serde_json::from_str(r#"{"temperature":0.5,"presence":0.3,"repeat":1.05,"last_n":256}"#).unwrap();
        assert!(sampling.penalized());
        sampling.last_n = 0;
        assert!(!sampling.penalized());
        sampling.last_n = 64;
        sampling.presence = 0.0;
        sampling.repeat = 1.0;
        assert!(!sampling.penalized());
    }

    #[test]
    fn stop_strings_are_found_across_pieces() {
        let text = b"answer<|im_";
        assert_eq!(position(text, b"<|im_end|>"), None);
        let text = b"answer<|im_end|>\n";
        assert_eq!(position(text, b"<|im_end|>"), Some(6));
    }
}
