//! Speech to text with whisper.cpp (whisper-rs), in this process, with each word's time and
//! without what Whisper invents (clean). Portuguese only; the names of the people followed go in as the initial prompt so
//! Whisper spells them right. Metal on macOS (CoreML would need an encoder model per
//! Whisper model), CPU elsewhere unless the build asked for CUDA or Vulkan.

use anyhow::{anyhow, Result};
use std::path::Path;
use std::sync::Once;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::pauses::Word;
use super::SAMPLE_RATE;
use crate::collect::fold;

/// Lines Whisper writes from silence, music or noise (from the subtitles it was trained on),
/// as `fold` writes them. A line that is only one of these is dropped.
const HALLUCINATIONS: [&str; 14] = [
    "legendas pela comunidade amara.org",
    "legendas pela comunidade amara org",
    "obrigado por assistir",
    "obrigada por assistir",
    "obrigado por assistirem",
    "inscreva-se no canal",
    "inscreva se no canal",
    "se inscreva no canal",
    "deixe seu like",
    "ative o sininho",
    "musica",
    "aplausos",
    "risos",
    "legenda adriana zanotto",
];

const GPU: bool = cfg!(any(target_os = "macos", feature = "whisper-cuda", feature = "whisper-vulkan"));

pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    pub fn load(path: &Path) -> Result<Self> {
        static LOGS: Once = Once::new();
        // whisper.cpp's own messages go to the app log instead of the terminal
        LOGS.call_once(whisper_rs::install_whisper_log_trampoline);
        let params = WhisperContextParameters { use_gpu: GPU, ..Default::default() };
        let ctx = WhisperContext::new_with_params(&path.to_string_lossy(), params)
            .map_err(|e| anyhow!("Failed to load the transcription model {}: {e}", path.display()))?;
        log::info!("Live: transcription model loaded from {} (GPU: {GPU})", path.display());
        Ok(Self { ctx })
    }

    /// The text of a stretch of 16 kHz speech, and its words with their times in seconds from
    /// the start of the stretch (estimates: good for pauses, not for exact timing).
    pub fn transcribe(&self, samples: &[f32], hint: &str) -> Result<(String, Vec<Word>)> {
        // Whisper needs at least a second; a shorter stretch is padded with silence
        let mut audio = samples.to_vec();
        if audio.len() < SAMPLE_RATE as usize + SAMPLE_RATE as usize / 10 {
            audio.resize(SAMPLE_RATE as usize + SAMPLE_RATE as usize / 10, 0.0);
        }
        let strategy = if GPU {
            SamplingStrategy::BeamSearch { beam_size: 5, patience: 1.0 }
        } else {
            SamplingStrategy::Greedy { best_of: 1 }
        };
        let mut params = FullParams::new(strategy);
        params.set_language(Some("pt"));
        if !hint.is_empty() {
            params.set_initial_prompt(hint);
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
        params.set_n_threads(threads as i32);
        // Where these differ from whisper.cpp's defaults: no timestamp tokens (they make it skip
        // whole stretches) but token times (for the pauses), no sound effects written as text, a
        // stretch counted as silence a little sooner (against invented lines), nothing printed
        params.set_no_timestamps(true);
        params.set_token_timestamps(true);
        params.set_suppress_non_speech_tokens(true);
        params.set_no_speech_thold(0.55);
        params.set_print_progress(false);
        params.set_print_timestamps(false);

        let mut state = self.ctx.create_state()?;
        state.full(params, &audio)?;
        let eot = self.ctx.token_eot();
        let mut text = String::new();
        let mut words: Vec<Word> = Vec::new();
        for i in 0..state.full_n_segments()? {
            let Ok(segment) = state.full_get_segment_text_lossy(i) else { continue };
            // A token starting with a space starts a word; times are in hundredths of a second.
            // Segments are joined with a space: each one starts a word
            let mut first = true;
            for j in 0..state.full_n_tokens(i).unwrap_or(0) {
                let (Ok(data), Ok(token)) = (state.full_get_token_data(i, j), state.full_get_token_text_lossy(i, j)) else { continue };
                if data.id >= eot {
                    continue; // special tokens (end, timestamps…) are not words
                }
                let (start, end) = (data.t0 as f64 / 100.0, data.t1 as f64 / 100.0);
                let new_word = std::mem::take(&mut first) || token.starts_with(' ');
                match words.last_mut() {
                    Some(word) if !new_word => {
                        word.text.push_str(&token);
                        word.end = end.max(word.end);
                    }
                    _ => words.push(Word { text: token.trim().to_string(), start, end }),
                }
            }
            let segment = segment.trim();
            if !segment.is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(segment);
            }
        }
        let cleaned = clean(&text);
        // Cleaning changed the words: they no longer spell the text, so the stretch stays one line
        if cleaned != text {
            words.clear();
        }
        Ok((cleaned, words))
    }
}

/// `text` without what Whisper invents: a line that is only a known hallucination, a word or
/// phrase (up to five words) said again right after itself (kept once), and a line that is
/// mostly such repeats (a loop such as "sim não sim não…").
pub fn clean(text: &str) -> String {
    let text = text.trim();
    let bare = fold(text).trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    if bare.is_empty() || HALLUCINATIONS.iter().any(|h| bare == *h) || bare.contains("amara.org") {
        return String::new();
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let kept = collapse_repeats(&words);
    if words.len() >= 6 && kept.len() * 5 <= words.len() * 2 {
        return String::new();
    }
    if kept.len() == words.len() {
        text.to_string()
    } else {
        kept.join(" ")
    }
}

/// The words in order, except that whenever the last words kept end in the same run of one to
/// five words twice, the second run is dropped. Every prefix of `kept` was once all of it, so
/// one check per word is enough.
fn collapse_repeats<'a>(words: &[&'a str]) -> Vec<&'a str> {
    let mut kept: Vec<&str> = Vec::with_capacity(words.len());
    for &word in words {
        kept.push(word);
        let len = kept.len();
        if let Some(run) = (1..=5.min(len / 2)).find(|&run| kept[len - run..] == kept[len - 2 * run..len - run]) {
            kept.truncate(len - run);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hallucinations_and_repetition_are_removed() {
        assert_eq!(clean("Legendas pela comunidade Amara.org"), "");
        assert_eq!(clean("Obrigado por assistir!"), "");
        assert_eq!(clean("[Música]"), "");
        assert_eq!(clean("   "), "");
        // Repeated words and phrases are kept once
        assert_eq!(clean("eu eu vou vou construir creches"), "eu vou construir creches");
        assert_eq!(clean("a saúde pública a saúde pública é prioridade"), "a saúde pública é prioridade");
        // Said three times, still once
        assert_eq!(clean("vamos vamos vamos fazer"), "vamos fazer");
        assert_eq!(clean("é isso é isso é isso que importa"), "é isso que importa");
        // Mostly repetition is noise
        assert_eq!(clean("sim não sim não sim não sim não sim"), "");
        // A short emphasis is speech
        assert_eq!(clean("não não não não"), "não");
        // Real speech that only mentions music stays
        assert_eq!(clean("a música brasileira precisa de incentivo"), "a música brasileira precisa de incentivo");
    }
}
