//! Who is speaking (from Assunta's diarization/live.rs): each stretch of speech gets voice
//! embeddings (WeSpeaker ResNet34, in 2.5 s windows), clustered as they come. When a new voice
//! has spoken long enough it is announced ("Voz 2: quem é?"), with the people followed ranked
//! by how close their voiceprint is; a voiceprint close enough names it right away.
//!
//! Identifying people by their voice is biometric processing: none of this runs without the
//! user's consent (prefs voice_recognition), and a person's voiceprint is learned only from the
//! user's own answers, never from a guess.

use anyhow::{anyhow, Result};
use pyannote_rs::EmbeddingExtractor;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Mutex;

use super::SAMPLE_RATE;

/// Speech shorter than this carries too little voice for an embedding.
const MIN_SEGMENT_SECONDS: f64 = 1.5;
/// Long stretches are embedded in windows of about this length.
const WINDOW_SECONDS: f64 = 2.5;
/// Cosine similarity of a window to a voice's centroid to count as that voice (Assunta's,
/// a little stricter than its offline clustering since one window is noisier).
pub const LIVE_MATCH_THRESHOLD: f32 = 0.35;
/// Similarity of a voice to a person's voiceprint to name it without asking (Assunta's
/// MEMBER_MATCH_THRESHOLD).
pub const VOICEPRINT_THRESHOLD: f32 = 0.6;
/// A new voice is announced after this much speech in at least ANNOUNCE_WINDOWS windows.
const ANNOUNCE_SECONDS: f64 = 4.0;
const ANNOUNCE_WINDOWS: usize = 2;
/// Unannounced candidates heard once and not again for this long are noise.
const CANDIDATE_TTL_SECONDS: f64 = 90.0;
/// Audio kept per voice for "listen".
const SAMPLE_SECONDS: usize = 6;
/// Stretches waiting for the worker; more are dropped (this is best effort).
const QUEUE: usize = 8;
/// Windows remembered to tell who said a line (lines come after Whisper is done).
const TIMELINE_WINDOWS: usize = 2_000;

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

/// One voice heard in the session.
#[derive(Debug, Clone)]
pub struct LiveVoice {
    pub id: String,
    /// "Voz N" in the UI, in order of announcement.
    pub number: usize,
    sum: Vec<f32>,
    pub centroid: Vec<f32>,
    pub windows: usize,
    pub seconds: f64,
    pub last_heard: f64,
    /// Latest time ranges this voice spoke in (to hear what they said).
    pub heard: VecDeque<(f64, f64)>,
    pub announced: bool,
    /// Who it is: a person followed, or a name typed.
    pub target_id: Option<String>,
    pub label: Option<String>,
    /// 'interviewer': asks the questions (never checked).
    pub role: Option<String>,
    /// Answered (by the user, or named by a voiceprint).
    pub resolved: bool,
    /// Named by a voiceprint, not by the user.
    pub recognized: bool,
}

/// Online clustering of voice embeddings. Pure: no model, no clock.
#[derive(Debug, Default)]
pub struct LiveClusters {
    pub voices: Vec<LiveVoice>,
    announced: usize,
    next_id: u64,
}

impl LiveClusters {
    /// Adds one window's embedding; returns the voice it belongs to and whether that voice
    /// should be announced now.
    pub fn observe(&mut self, embedding: Vec<f32>, start: f64, end: f64) -> (usize, bool) {
        self.prune(start);
        let embedding = normalize(embedding);
        let best = self
            .voices
            .iter()
            .enumerate()
            .map(|(i, v)| (i, cosine(&embedding, &v.centroid)))
            .filter(|(_, similarity)| *similarity >= LIVE_MATCH_THRESHOLD)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        let index = match best {
            Some(i) => {
                let voice = &mut self.voices[i];
                voice.sum.iter_mut().zip(&embedding).for_each(|(s, e)| *s += e);
                voice.centroid = normalize(voice.sum.clone());
                voice.windows += 1;
                i
            }
            None => {
                self.next_id += 1;
                self.voices.push(LiveVoice {
                    id: format!("voice-{}", self.next_id),
                    number: 0,
                    sum: embedding.clone(),
                    centroid: embedding,
                    windows: 1,
                    seconds: 0.0,
                    last_heard: end,
                    heard: VecDeque::new(),
                    announced: false,
                    target_id: None,
                    label: None,
                    role: None,
                    resolved: false,
                    recognized: false,
                });
                self.voices.len() - 1
            }
        };
        let voice = &mut self.voices[index];
        voice.seconds += end - start;
        voice.last_heard = end;
        voice.heard.push_back((start, end));
        if voice.heard.len() > 5 {
            voice.heard.pop_front();
        }
        let announce = !voice.announced && voice.seconds >= ANNOUNCE_SECONDS && voice.windows >= ANNOUNCE_WINDOWS;
        if announce {
            self.announced += 1;
            voice.announced = true;
            voice.number = self.announced;
        }
        (index, announce)
    }

    /// Forgets candidates that were heard once long ago (noise, crosstalk).
    fn prune(&mut self, now: f64) {
        self.voices.retain(|v| v.announced || v.windows > 1 || now - v.last_heard < CANDIDATE_TTL_SECONDS);
    }
}

/// A person followed the voice could be.
#[derive(Debug, Clone)]
pub struct Person {
    pub target_id: String,
    pub name: String,
    pub voiceprint: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub target_id: String,
    pub name: String,
    /// Voice similarity, when the person has a voiceprint.
    pub similarity: Option<f32>,
    /// Already given to another voice of this session.
    pub taken: bool,
}

/// People for a voice, the most similar voiceprint first, and the one close enough to name it
/// without asking (never one already given to another voice).
pub fn suggestions_for(centroid: &[f32], people: &[Person], taken: &[String]) -> (Vec<Suggestion>, Option<String>) {
    let mut suggestions: Vec<Suggestion> = people
        .iter()
        .map(|p| Suggestion {
            target_id: p.target_id.clone(),
            name: p.name.clone(),
            similarity: p.voiceprint.as_ref().map(|vp| cosine(centroid, vp)),
            taken: taken.contains(&p.target_id),
        })
        .collect();
    suggestions.sort_by(|a, b| {
        b.similarity
            .unwrap_or(f32::MIN)
            .total_cmp(&a.similarity.unwrap_or(f32::MIN))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let recognized = suggestions
        .iter()
        .find(|s| !s.taken)
        .filter(|s| s.similarity.is_some_and(|sim| sim >= VOICEPRINT_THRESHOLD))
        .map(|s| s.target_id.clone());
    (suggestions, recognized)
}

/// A voice as the UI shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceView {
    pub voice_id: String,
    pub number: usize,
    pub target_id: Option<String>,
    pub label: Option<String>,
    pub role: Option<String>,
    pub resolved: bool,
    pub recognized: bool,
    pub seconds: f64,
    /// Latest time ranges (seconds from the session start) this voice spoke in.
    pub heard: Vec<(f64, f64)>,
    /// Only while it waits for an answer.
    pub suggestions: Vec<Suggestion>,
}

/// What changed about a voice: to show (view) and to keep (centroid).
#[derive(Debug, Clone)]
pub struct VoiceChange {
    pub view: VoiceView,
    pub centroid: Vec<f32>,
}

fn change(voice: &LiveVoice, people: &[Person], taken: &[String]) -> VoiceChange {
    let suggestions = if voice.resolved { Vec::new() } else { suggestions_for(&voice.centroid, people, taken).0 };
    VoiceChange {
        view: VoiceView {
            voice_id: voice.id.clone(),
            number: voice.number,
            target_id: voice.target_id.clone(),
            label: voice.label.clone(),
            role: voice.role.clone(),
            resolved: voice.resolved,
            recognized: voice.recognized,
            seconds: voice.seconds,
            heard: voice.heard.iter().copied().collect(),
            suggestions,
        },
        centroid: voice.centroid.clone(),
    }
}

// ---- The tracker of the session in progress ----

struct Stretch {
    samples: Vec<f32>,
    start: f64,
    end: f64,
}

struct State {
    clusters: LiveClusters,
    people: Vec<Person>,
    /// Latest audio of each voice, for "listen".
    audio: Vec<(String, VecDeque<i16>)>,
    /// Latest windows and the voice heard in each: (start, end, voice id).
    timeline: VecDeque<(f64, f64, String)>,
}

impl State {
    fn taken(&self) -> Vec<String> {
        self.clusters.voices.iter().filter_map(|v| v.target_id.clone()).collect()
    }
}

static SENDER: Mutex<Option<SyncSender<Stretch>>> = Mutex::new(None);
static STATE: Mutex<Option<State>> = Mutex::new(None);
/// Bumped on every start/stop, so a worker of an ended session does nothing more.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Starts following voices; `on_change` hears each voice announced or named.
pub fn start(model: &Path, people: Vec<Person>, on_change: impl Fn(VoiceChange) + Send + 'static) -> Result<()> {
    let mut extractor = EmbeddingExtractor::new(model).map_err(|e| anyhow!("Failed to load the voice model: {e}"))?;
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let (tx, rx) = sync_channel::<Stretch>(QUEUE);
    *STATE.lock().map_err(|_| anyhow!("voices poisoned"))? =
        Some(State { clusters: LiveClusters::default(), people, audio: Vec::new(), timeline: VecDeque::new() });
    *SENDER.lock().map_err(|_| anyhow!("voices poisoned"))? = Some(tx);
    std::thread::Builder::new().name("live-voices".into()).spawn(move || {
        while let Ok(stretch) = rx.recv() {
            if GENERATION.load(Ordering::SeqCst) != generation {
                break;
            }
            match process(&mut extractor, stretch) {
                Ok(changes) => changes.into_iter().for_each(&on_change),
                Err(e) => log::debug!("Live voices: stretch skipped: {e}"),
            }
        }
        log::info!("Live voices: stopped");
    })?;
    log::info!("Live voices: following voices");
    Ok(())
}

/// Stops following voices; returns every voice heard, to keep.
pub fn stop() -> Vec<VoiceChange> {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut sender) = SENDER.lock() {
        *sender = None;
    }
    let Ok(mut state) = STATE.lock() else { return Vec::new() };
    let Some(state) = state.take() else { return Vec::new() };
    let taken = state.taken();
    state.clusters.voices.iter().filter(|v| v.announced).map(|v| change(v, &state.people, &taken)).collect()
}

pub fn running() -> bool {
    SENDER.lock().is_ok_and(|s| s.is_some())
}

/// Offers a stretch of speech. Cheap when off; dropped when the worker is behind.
pub fn offer(samples: &[f32], start: f64, end: f64) {
    let Ok(guard) = SENDER.lock() else { return };
    let Some(sender) = guard.as_ref() else { return };
    if end - start < MIN_SEGMENT_SECONDS {
        return;
    }
    if let Err(TrySendError::Full(_)) = sender.try_send(Stretch { samples: samples.to_vec(), start, end }) {
        log::debug!("Live voices: busy, stretch dropped");
    }
}

fn process(extractor: &mut EmbeddingExtractor, stretch: Stretch) -> Result<Vec<VoiceChange>> {
    let samples: Vec<i16> = stretch.samples.iter().map(|&s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).collect();
    let duration = stretch.end - stretch.start;
    let pieces = ((duration / WINDOW_SECONDS).floor() as usize).max(1);
    let piece_len = samples.len() / pieces;
    let mut changes = Vec::new();
    for piece in 0..pieces {
        let from = piece * piece_len;
        let to = if piece + 1 == pieces { samples.len() } else { from + piece_len };
        let window = &samples[from..to];
        let embedding: Vec<f32> = extractor.compute(window).map_err(|e| anyhow!("embedding: {e}"))?.collect();
        let start = stretch.start + from as f64 / SAMPLE_RATE as f64;
        let end = stretch.start + to as f64 / SAMPLE_RATE as f64;
        let mut guard = STATE.lock().map_err(|_| anyhow!("voices poisoned"))?;
        let Some(state) = guard.as_mut() else { return Ok(changes) };
        let (index, announce) = state.clusters.observe(embedding, start, end);
        let voice_id = state.clusters.voices[index].id.clone();
        keep_audio(&mut state.audio, &voice_id, window);
        state.timeline.push_back((start, end, voice_id));
        if state.timeline.len() > TIMELINE_WINDOWS {
            state.timeline.pop_front();
        }
        if !announce {
            continue;
        }
        // A voiceprint close enough names the voice; the user can still change it
        let taken = state.taken();
        let (_, recognized) = suggestions_for(&state.clusters.voices[index].centroid, &state.people, &taken);
        let voice = &mut state.clusters.voices[index];
        if let Some(target_id) = recognized {
            voice.target_id = Some(target_id);
            voice.resolved = true;
            voice.recognized = true;
        }
        log::info!("Live voices: voice {} ({}){}", voice.number, voice.id, if voice.recognized { ", recognized" } else { "" });
        let voice = voice.clone();
        changes.push(change(&voice, &state.people, &taken));
    }
    Ok(changes)
}

fn keep_audio(audio: &mut Vec<(String, VecDeque<i16>)>, voice_id: &str, window: &[i16]) {
    let limit = SAMPLE_SECONDS * SAMPLE_RATE as usize;
    let buffer = match audio.iter().position(|(id, _)| id == voice_id) {
        Some(i) => &mut audio[i].1,
        None => {
            audio.push((voice_id.to_string(), VecDeque::new()));
            &mut audio.last_mut().expect("just pushed").1
        }
    };
    buffer.extend(window.iter().copied());
    while buffer.len() > limit {
        buffer.pop_front();
    }
}

/// The voice heard most during [start, end) of the session, announced or not yet.
pub fn voice_during(start: f64, end: f64) -> Option<String> {
    let guard = STATE.lock().ok()?;
    let state = guard.as_ref()?;
    let mut overlaps: Vec<(&str, f64)> = Vec::new();
    for (from, to, id) in &state.timeline {
        let overlap = to.min(end) - from.max(start);
        if overlap <= 0.0 {
            continue;
        }
        match overlaps.iter_mut().find(|(v, _)| v == id) {
            Some((_, seconds)) => *seconds += overlap,
            None => overlaps.push((id, overlap)),
        }
    }
    overlaps.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(id, _)| id.to_string())
}

/// The user's answer about a voice of the session in progress: a person followed, a name, the
/// interviewer (`role`), or neither ("I don't know"). None when the voice is not in this session any more.
pub fn resolve(voice_id: &str, target_id: Option<String>, label: Option<String>, role: Option<String>) -> Option<VoiceChange> {
    let mut guard = STATE.lock().ok()?;
    let state = guard.as_mut()?;
    let voice = state.clusters.voices.iter_mut().find(|v| v.id == voice_id)?;
    voice.resolved = true;
    voice.recognized = false;
    voice.label = label.filter(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).filter(|_| target_id.is_none() && role.is_none());
    voice.target_id = target_id.filter(|_| role.is_none());
    voice.role = role;
    let voice = voice.clone();
    let taken = state.taken();
    Some(change(&voice, &state.people, &taken))
}

/// The person followed a voice of the session in progress was given to, if any.
pub fn person_of(voice_id: &str) -> Option<String> {
    let guard = STATE.lock().ok()?;
    guard.as_ref()?.clusters.voices.iter().find(|v| v.id == voice_id)?.target_id.clone()
}

/// WAV of the latest seconds of a voice.
pub fn sample_wav(voice_id: &str) -> Option<Vec<u8>> {
    let guard = STATE.lock().ok()?;
    let (_, buffer) = guard.as_ref()?.audio.iter().find(|(id, _)| id == voice_id)?;
    let samples: Vec<f32> = buffer.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
    Some(super::wav::encode(&samples))
}

/// A person's voiceprint after one more answer: the running mean of the voices given to them.
pub fn learn(voiceprint: Option<&[f32]>, samples: i64, centroid: &[f32]) -> Vec<f32> {
    match voiceprint {
        Some(old) if old.len() == centroid.len() && samples > 0 => {
            let n = samples as f32;
            old.iter().zip(centroid).map(|(o, c)| (o * n + c) / (n + 1.0)).collect()
        }
        _ => centroid.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(a: f32, b: f32, c: f32) -> Vec<f32> {
        vec![a, b, c]
    }

    #[test]
    fn a_new_voice_is_announced_once_after_enough_speech() {
        let mut clusters = LiveClusters::default();
        let (ana, announce) = clusters.observe(voice(1.0, 0.1, 0.0), 0.0, 2.5);
        assert!(!announce); // one window is not enough
        let (again, announce) = clusters.observe(voice(0.95, 0.15, 0.05), 3.0, 5.5);
        assert_eq!(again, ana);
        assert!(announce);
        assert_eq!(clusters.voices[ana].number, 1);
        let (_, announce) = clusters.observe(voice(1.0, 0.0, 0.1), 6.0, 8.5);
        assert!(!announce); // already announced

        // A different voice becomes voice 2
        clusters.observe(voice(0.0, 1.0, 0.1), 9.0, 11.5);
        let (bia, announce) = clusters.observe(voice(0.1, 1.0, 0.0), 12.0, 14.5);
        assert!(announce);
        assert_eq!(clusters.voices[bia].number, 2);
        assert_eq!(clusters.voices.len(), 2);
    }

    #[test]
    fn stray_windows_are_forgotten() {
        let mut clusters = LiveClusters::default();
        clusters.observe(voice(0.0, 0.0, 1.0), 0.0, 2.0); // crosstalk, never heard again
        clusters.observe(voice(1.0, 0.0, 0.0), 100.0, 102.5);
        assert_eq!(clusters.voices.len(), 1);
        assert_eq!(clusters.voices[0].centroid, normalize(voice(1.0, 0.0, 0.0)));
    }

    #[test]
    fn suggestions_rank_by_voiceprint_and_never_name_a_voice_twice() {
        let people = vec![
            Person { target_id: "joao".into(), name: "João Teste".into(), voiceprint: Some(voice(0.0, 1.0, 0.0)) },
            Person { target_id: "caio".into(), name: "Caio Exemplo".into(), voiceprint: None },
            Person { target_id: "marina".into(), name: "Marina Exemplo".into(), voiceprint: Some(voice(0.9, 0.1, 0.0)) },
        ];
        let centroid = normalize(voice(1.0, 0.0, 0.0));
        let (suggestions, recognized) = suggestions_for(&centroid, &people, &[]);
        let order: Vec<&str> = suggestions.iter().map(|s| s.target_id.as_str()).collect();
        assert_eq!(order, vec!["marina", "joao", "caio"]);
        assert_eq!(recognized.as_deref(), Some("marina"));
        // Marina already has a voice in this session: nobody is named, and she is marked
        let (suggestions, recognized) = suggestions_for(&centroid, &people, &["marina".into()]);
        assert_eq!(recognized, None);
        assert!(suggestions[0].taken);
        // A voice far from every voiceprint is asked about
        assert_eq!(suggestions_for(&normalize(voice(0.0, 0.0, 1.0)), &people, &[]).1, None);
    }

    #[test]
    fn a_voiceprint_is_the_mean_of_the_answers() {
        assert_eq!(learn(None, 0, &[1.0, 0.0]), vec![1.0, 0.0]);
        assert_eq!(learn(Some(&[1.0, 0.0]), 1, &[0.0, 1.0]), vec![0.5, 0.5]);
        assert_eq!(learn(Some(&[0.5, 0.5]), 2, &[1.0, 1.0]), vec![2.0 / 3.0, 2.0 / 3.0]);
        // A voiceprint of another model's size starts over
        assert_eq!(learn(Some(&[1.0]), 5, &[0.0, 1.0]), vec![0.0, 1.0]);
    }
}
