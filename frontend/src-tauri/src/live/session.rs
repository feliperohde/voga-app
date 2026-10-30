//! A live session from start to end. Three threads: audio (capture, speech detection, the WAV
//! file), transcription (Whisper, one stretch at a time, in order) and voices (voices.rs); the
//! fact checker is a task (check.rs). Lines, voices and checks are saved as they come; when the
//! session ends its transcript becomes a `live` document, with who said each line. A session
//! the app did not end (it quit, it crashed) is closed at the next start (`recover`).

use anyhow::{anyhow, bail, Result};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use super::audio::Capture;
use super::check::{self, Checker, Line};
use super::pauses::{cover, rejoin, split_at_pauses, Piece};
use super::vad::{Speech, Vad};
use super::voices::{self, VoiceChange};
use super::pauses::Word;
use super::remote::RemoteTranscriber;
use super::whisper::Transcriber;
use super::{wav, SAMPLE_RATE};
use crate::analyze;
use crate::builtin::{self, catalog::ModelKind};
use crate::collect::{self, youtube::TimedText, NewDocument};
use crate::llm::ConfiguredChatModel;
use crate::prefs;
use crate::rag::indexer;

pub const STATE_EVENT: &str = "live-state";
pub const LEVEL_EVENT: &str = "live-level";
pub const TRANSCRIPT_EVENT: &str = "live-transcript";
pub const VOICE_EVENT: &str = "live-voice";

/// "1": voices may be told apart (consent to voice recognition, LGPD).
pub const VOICE_CONSENT: &str = "voice_recognition";
/// The Whisper model chosen in Settings (empty: the best one downloaded).
pub const TRANSCRIPTION_MODEL: &str = "transcription_model";
/// Where speech is written down: "" (Whisper on this computer) or a provider (openai, groq,
/// custom-openai) with TRANSCRIPTION_REMOTE_MODEL, for computers too modest for Whisper.
pub const TRANSCRIPTION_PROVIDER: &str = "transcription_provider";
pub const TRANSCRIPTION_REMOTE_MODEL: &str = "transcription_remote_model";

/// The provider's transcription service, when one is chosen instead of Whisper here.
pub async fn remote_transcriber(pool: &SqlitePool) -> Result<Option<RemoteTranscriber>> {
    let provider = prefs::get(pool, TRANSCRIPTION_PROVIDER).await?.unwrap_or_default();
    if provider.is_empty() || provider == "builtin" {
        return Ok(None);
    }
    let settings = crate::settings::load(pool).await?;
    let base = crate::providers::openai_base(&provider, settings.custom_endpoint.as_deref())
        .ok_or_else(|| anyhow!("{provider} does not transcribe; choose another in Settings › AI"))?;
    let model = prefs::get(pool, TRANSCRIPTION_REMOTE_MODEL).await?.filter(|m| !m.trim().is_empty())
        .ok_or_else(|| anyhow!("Choose the transcription model of {provider} in Settings › AI"))?;
    let key = crate::settings::api_key(pool, &provider).await?.unwrap_or_default();
    Ok(Some(RemoteTranscriber::new(&base, &key, &model)))
}

/// Who writes the speech down: Whisper on this computer, or a provider's server.
enum Engine {
    Local(Transcriber),
    Remote(RemoteTranscriber),
}

impl Engine {
    /// The text and, from Whisper here, its words with their times (a server gives none: its
    /// stretch stays one line).
    fn transcribe(&self, samples: &[f32], hint: &str) -> Result<(String, Vec<Word>)> {
        match self {
            Engine::Local(whisper) => whisper.transcribe(samples, hint),
            Engine::Remote(remote) => Ok((tauri::async_runtime::block_on(remote.transcribe(samples, hint))?, Vec::new())),
        }
    }
}

const READ_EVERY: Duration = Duration::from_millis(50);
const LEVEL_EVERY: Duration = Duration::from_millis(100);
const WAV_SYNC_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveState {
    pub session_id: Option<String>,
    pub investigation_id: Option<String>,
    /// idle | recording | finishing (the last lines are transcribed, the document saved)
    pub phase: &'static str,
    pub started_at: Option<String>,
    /// Seconds the transcript is behind what is said.
    pub behind: f64,
    /// Voices are told apart.
    pub voices: bool,
    /// Claims are checked (an AI model is set up).
    pub checking: bool,
}

const IDLE: LiveState =
    LiveState { session_id: None, investigation_id: None, phase: "idle", started_at: None, behind: 0.0, voices: false, checking: false };
static STATE: Mutex<LiveState> = Mutex::new(IDLE);

struct Active {
    session_id: String,
    /// The models the session holds (not to be deleted meanwhile).
    models: Vec<String>,
    /// Claims are checked as found (else only detected); the session's checker reads it.
    auto: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    audio: Option<JoinHandle<()>>,
    transcription: Option<JoinHandle<()>>,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

pub fn state() -> LiveState {
    STATE.lock().map(|s| s.clone()).unwrap_or(IDLE)
}

/// A session runs, or its checks are still being made: the AI should serve it first.
pub fn busy() -> bool {
    state().phase != "idle" || check::pending() > 0
}

/// The id of the session in progress.
pub fn active() -> Option<String> {
    ACTIVE.lock().ok()?.as_ref().map(|a| a.session_id.clone())
}

/// Checks claims as they are found in the session in progress, or only detects them.
pub fn set_auto_check(session_id: &str, on: bool) {
    if let Ok(active) = ACTIVE.lock() {
        if let Some(active) = active.as_ref().filter(|a| a.session_id == session_id) {
            active.auto.store(on, Ordering::SeqCst);
        }
    }
}

pub fn model_in_use(model: &str) -> bool {
    ACTIVE.lock().is_ok_and(|a| a.as_ref().is_some_and(|a| a.models.iter().any(|m| m == model)))
}

fn update<R: Runtime>(app: &AppHandle<R>, change: impl FnOnce(&mut LiveState)) {
    if let Ok(mut state) = STATE.lock() {
        change(&mut state);
        let _ = app.emit(STATE_EVENT, state.clone());
    }
}

/// The Whisper model to use: the one chosen, else the best downloaded.
pub async fn transcription_model(pool: &SqlitePool) -> Option<String> {
    let chosen = prefs::get(pool, TRANSCRIPTION_MODEL).await.ok().flatten().filter(|m| builtin::is_downloaded(m));
    chosen.or_else(|| builtin::downloaded_of_kind(ModelKind::Transcription))
}

pub async fn voice_consent(pool: &SqlitePool) -> bool {
    prefs::get(pool, VOICE_CONSENT).await.ok().flatten().is_some_and(|v| v == "1")
}

/// A person a session follows.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Followed {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub aliases: String,
    #[serde(skip)]
    pub voiceprint: Option<Vec<u8>>,
    /// Where their records are (a debate's people come from many investigations).
    pub investigation_id: String,
}

/// The people a session follows: those of its investigation, or a debate's participants in order.
pub async fn followed(pool: &SqlitePool, session_id: &str) -> Result<Vec<Followed>> {
    Ok(sqlx::query_as(
        "SELECT t.id, t.name, t.aliases, t.voiceprint, t.investigation_id FROM targets t
         LEFT JOIN live_participants p ON p.session_id = ?1 AND p.target_id = t.id
         WHERE t.kind = 'person' AND (p.target_id IS NOT NULL OR t.investigation_id = (SELECT investigation_id FROM live_sessions WHERE id = ?1))
         ORDER BY p.position, t.name",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?)
}

/// The people a session follows, as the checker reads them.
pub async fn people(pool: &SqlitePool, session_id: &str) -> Result<Vec<analyze::claims::Person>> {
    Ok(followed(pool, session_id)
        .await?
        .into_iter()
        .map(|f| analyze::claims::Person { target_id: f.id, name: f.name, aliases: serde_json::from_str(&f.aliases).unwrap_or_default() })
        .collect())
}

/// The people a session follows, with their voiceprints.
pub async fn voice_people(pool: &SqlitePool, session_id: &str) -> Result<Vec<voices::Person>> {
    Ok(followed(pool, session_id)
        .await?
        .into_iter()
        .map(|f| voices::Person { target_id: f.id, name: f.name, voiceprint: f.voiceprint.map(|v| crate::rag::store::decode_vector(&v)) })
        .collect())
}

/// A line of the transcript, as the UI shows it.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct LineView {
    pub id: i64,
    pub session_id: String,
    #[sqlx(rename = "start_time")]
    pub start: f64,
    #[sqlx(rename = "end_time")]
    pub end: f64,
    pub text: String,
    pub voice_id: Option<String>,
}

/// What the threads of a session share.
#[derive(Clone)]
struct Session {
    id: String,
    /// The candidate: every line is theirs when voices are not told apart (a debate has none).
    candidate: Option<String>,
    voices: bool,
    /// The names Whisper is told about.
    hint: String,
}

pub struct StartOptions {
    /// None: a debate, outside every investigation.
    pub investigation_id: Option<String>,
    /// The candidate of a session of an investigation.
    pub target_id: Option<String>,
    /// A debate's people (targets of any investigation), in order.
    pub participants: Vec<String>,
    pub title: String,
    /// A microphone ("" for the default one), or None.
    pub microphone: Option<String>,
    pub system_audio: bool,
    /// Check each claim as found; off, they wait for a person to ask.
    pub auto_check: bool,
}

/// Starts a session; returns its id once the sound is being heard.
pub async fn start<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, options: StartOptions) -> Result<String> {
    if ACTIVE.lock().map_err(|_| anyhow!("live state poisoned"))?.is_some() || state().phase != "idle" {
        bail!("A live session is already running");
    }
    let inv = options.investigation_id.clone();
    let people = match &inv {
        Some(inv) => {
            let mut people = analyze::people(pool, inv).await?;
            let at = people.iter().position(|p| Some(&p.target_id) == options.target_id.as_ref()).ok_or_else(|| anyhow!("Choose a person this investigation follows"))?;
            let candidate = people.remove(at);
            people.insert(0, candidate);
            people
        }
        None => participants(pool, &options.participants).await?,
    };
    let candidate = inv.as_ref().map(|_| people[0].target_id.clone());
    // A debate is told apart by voices: without them no line has anyone to be checked against
    let voice_model = if voice_consent(pool).await { builtin::downloaded_of_kind(ModelKind::Voice) } else { None };
    if inv.is_none() && voice_model.is_none() {
        bail!("A debate needs voice recognition: turn it on and download the voice model in Settings › AI");
    }

    let mut models = Vec::new();
    let engine = match remote_transcriber(pool).await? {
        Some(remote) => Engine::Remote(remote),
        None => {
            let model = transcription_model(pool).await.ok_or_else(|| anyhow!("Download a transcription model first (Settings › AI)"))?;
            let path = builtin::model_file(&model)?;
            models.push(model);
            Engine::Local(tokio::task::spawn_blocking(move || Transcriber::load(&path)).await??)
        }
    };

    let id = format!("live-{}", uuid::Uuid::new_v4());
    let audio_path = app.path().app_data_dir()?.join("live").join(format!("{id}.wav"));
    let started_at = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO live_sessions (id, investigation_id, target_id, title, mic_device, system_audio, started_at, audio_path, auto_check)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&inv)
    .bind(&candidate)
    .bind(options.title.trim())
    .bind(&options.microphone)
    .bind(options.system_audio)
    .bind(&started_at)
    .bind(audio_path.to_string_lossy().to_string())
    .bind(options.auto_check)
    .execute(pool)
    .await?;
    for (position, person) in people.iter().enumerate().filter(|_| inv.is_none()) {
        sqlx::query("INSERT INTO live_participants (session_id, position, name, target_id) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(position as i64 + 1)
            .bind(&person.name)
            .bind(&person.target_id)
            .execute(pool)
            .await?;
    }

    // Voices, with consent and the model downloaded
    let mut voices_on = false;
    if let Some(voice_model) = voice_model {
        let people = voice_people(pool, &id).await?;
        match builtin::model_file(&voice_model).and_then(|model| voices::start(&model, people, on_voice(app.clone(), pool.clone(), id.clone()))) {
            Ok(()) => {
                voices_on = true;
                models.push(voice_model);
            }
            Err(e) if inv.is_none() => {
                sqlx::query("DELETE FROM live_sessions WHERE id = ?").bind(&id).execute(pool).await?;
                return Err(e.context("The voices of the debate could not be told apart"));
            }
            Err(e) => log::warn!("Live: voices not told apart: {e:#}"),
        }
    }

    // Checks, with a chat model and the search index
    let auto = Arc::new(AtomicBool::new(options.auto_check));
    let checker = match (ConfiguredChatModel::from_settings(pool).await, indexer::embedder(pool).await) {
        (Ok(llm), Ok(embedder)) => {
            let app = app.clone();
            let mut checker = Checker::new(
                pool.clone(),
                Arc::new(llm.deterministic()),
                Arc::from(embedder),
                id.clone(),
                people.clone(),
                started_at[..10].to_string(),
                Box::new(move |view| {
                    let _ = app.emit(check::EVENT, view);
                }),
            );
            checker.auto = auto.clone();
            Some(checker)
        }
        (llm, _) => {
            log::warn!("Live: claims are not checked: {}", llm.err().map(|e| e.to_string()).unwrap_or_else(|| "no search index".into()));
            None
        }
    };
    let checking = checker.is_some();
    check::start(checker);

    let session = Session { id: id.clone(), candidate, voices: voices_on, hint: hint(&people) };
    let stop = Arc::new(AtomicBool::new(false));
    let position = Arc::new(AtomicU64::new(0));
    let (speech_tx, speech_rx) = mpsc::channel::<Speech>();
    let (opened_tx, opened_rx) = mpsc::sync_channel::<Result<()>>(1);
    let audio = std::thread::Builder::new().name("live-audio".into()).spawn({
        let (app, stop, position) = (app.clone(), stop.clone(), position.clone());
        let microphone = options.microphone.clone();
        move || run_audio(app, microphone, options.system_audio, audio_path, stop, speech_tx, opened_tx, position)
    })?;
    let opened = tokio::task::spawn_blocking(move || opened_rx.recv().unwrap_or_else(|_| Err(anyhow!("The audio thread ended")))).await?;
    if let Err(e) = opened {
        voices::stop();
        check::finish();
        let _ = audio.join();
        sqlx::query("DELETE FROM live_sessions WHERE id = ?").bind(&id).execute(pool).await?;
        return Err(e);
    }
    let transcription = std::thread::Builder::new().name("live-transcription".into()).spawn({
        let (app, pool) = (app.clone(), pool.clone());
        move || run_transcription(app, pool, session, engine, speech_rx, position)
    })?;
    *ACTIVE.lock().map_err(|_| anyhow!("live state poisoned"))? = Some(Active {
        session_id: id.clone(),
        models,
        auto,
        stop,
        audio: Some(audio),
        transcription: Some(transcription),
    });
    update(app, |s| {
        *s = LiveState { session_id: Some(id.clone()), investigation_id: inv, phase: "recording", started_at: Some(started_at), behind: 0.0, voices: voices_on, checking }
    });
    log::info!("Live: session {id} started (voices: {voices_on}, checks: {checking})");
    Ok(id)
}

/// A debate's people, in the order given; each must be a person followed somewhere.
async fn participants(pool: &SqlitePool, ids: &[String]) -> Result<Vec<analyze::claims::Person>> {
    if ids.is_empty() {
        bail!("Add the people of the debate");
    }
    let mut people: Vec<analyze::claims::Person> = Vec::new();
    for id in ids {
        let (name, aliases): (String, String) = sqlx::query_as("SELECT name, aliases FROM targets WHERE id = ? AND kind = 'person'")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| anyhow!("A person of the debate is no longer followed"))?;
        if people.iter().any(|p| p.target_id == *id) {
            bail!("{name} is in the debate twice");
        }
        people.push(analyze::claims::Person { target_id: id.clone(), name, aliases: serde_json::from_str(&aliases).unwrap_or_default() });
    }
    Ok(people)
}

/// A person joins a debate's people, last (once).
pub async fn add_participant(pool: &SqlitePool, session_id: &str, target_id: &str) -> Result<()> {
    let joined: Option<i64> = sqlx::query_scalar("SELECT 1 FROM live_participants WHERE session_id = ? AND target_id = ?")
        .bind(session_id)
        .bind(target_id)
        .fetch_optional(pool)
        .await?;
    if joined.is_none() {
        sqlx::query(
            "INSERT INTO live_participants (session_id, position, name, target_id)
             SELECT ?1, COALESCE(MAX(position), 0) + 1, (SELECT name FROM targets WHERE id = ?2), ?2 FROM live_participants WHERE session_id = ?1",
        )
        .bind(session_id)
        .bind(target_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// The names of the people followed, the candidate first, for Whisper to spell them right.
fn hint(people: &[analyze::claims::Person]) -> String {
    let names: Vec<String> = people.iter().flat_map(|p| std::iter::once(p.name.clone()).chain(p.aliases.iter().cloned())).collect();
    format!("{}.", names.join(", "))
}

/// Ends the session in progress: the last stretches are transcribed, then the document is saved.
pub fn stop<R: Runtime>(app: &AppHandle<R>, pool: SqlitePool) -> Result<()> {
    let mut active = ACTIVE.lock().map_err(|_| anyhow!("live state poisoned"))?.take().ok_or_else(|| anyhow!("No live session is running"))?;
    active.stop.store(true, Ordering::SeqCst);
    update(app, |s| s.phase = "finishing");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let threads = (active.audio.take(), active.transcription.take());
        let _ = tokio::task::spawn_blocking(move || {
            // Audio first: it hands the last stretch over and closes the channel Whisper reads
            for thread in [threads.0, threads.1].into_iter().flatten() {
                let _ = thread.join();
            }
        })
        .await;
        for change in voices::stop() {
            if let Err(e) = keep_voice(&pool, &active.session_id, &change).await {
                log::warn!("Live: a voice was not kept: {e}");
            }
        }
        check::finish();
        if let Err(e) = close(&pool, &active.session_id, "done").await {
            log::error!("Live: closing session {} failed: {e:#}", active.session_id);
        }
        update(&app, |s| *s = IDLE);
        log::info!("Live: session {} ended", active.session_id);
    });
    Ok(())
}

/// The app quits: the sound stops and its file is closed; the rest waits for `recover`.
pub fn shutdown() {
    let Ok(mut active) = ACTIVE.lock() else { return };
    if let Some(mut active) = active.take() {
        active.stop.store(true, Ordering::SeqCst);
        if let Some(audio) = active.audio.take() {
            let _ = audio.join();
        }
    }
    voices::stop();
}

/// Sessions left open by the app quitting become `interrupted`, with their document; checks
/// left half made are marked so.
pub async fn recover(pool: &SqlitePool) -> Result<()> {
    sqlx::query("UPDATE live_checks SET verdict = 'unverified', error = 'interrupted' WHERE verdict = 'checking'").execute(pool).await?;
    let open: Vec<(String, Option<String>, String, Option<f64>)> = sqlx::query_as(
        "SELECT s.id, s.audio_path, s.started_at, MAX(g.end_time) FROM live_sessions s
         LEFT JOIN live_segments g ON g.session_id = s.id WHERE s.status = 'recording' GROUP BY s.id",
    )
    .fetch_all(pool)
    .await?;
    for (id, audio_path, started_at, last) in open {
        if let Some(path) = audio_path {
            let _ = wav::repair(std::path::Path::new(&path));
        }
        let ended_at = chrono::DateTime::parse_from_rfc3339(&started_at)
            .map(|start| (start + ChronoDuration::milliseconds((last.unwrap_or(0.0) * 1000.0) as i64)).to_rfc3339())
            .unwrap_or(started_at);
        sqlx::query("UPDATE live_sessions SET ended_at = ? WHERE id = ?").bind(ended_at).bind(&id).execute(pool).await?;
        close(pool, &id, "interrupted").await?;
        log::info!("Live: session {id} was interrupted, closed");
    }
    Ok(())
}

async fn close(pool: &SqlitePool, session_id: &str, status: &str) -> Result<()> {
    sqlx::query("UPDATE live_sessions SET status = ?, ended_at = COALESCE(ended_at, ?) WHERE id = ?")
        .bind(status)
        .bind(Utc::now().to_rfc3339())
        .bind(session_id)
        .execute(pool)
        .await?;
    save_document(pool, session_id).await?;
    Ok(())
}

/// Who a session is about, as SQL over `live_sessions s LEFT JOIN targets t ON t.id = s.target_id`:
/// its candidate, or a debate's participants ("A × B").
pub const WHO: &str = "COALESCE(t.name, (SELECT group_concat(p.name, ' × ' ORDER BY p.position) FROM live_participants p WHERE p.session_id = s.id))";

/// Saves (or updates) the session's transcript as a `live` document, each line with who said
/// it, and indexes it; a debate's goes to the investigation of each participant. Changed names
/// are read again by the analysis.
pub async fn save_document(pool: &SqlitePool, session_id: &str) -> Result<Option<String>> {
    let (inv, title, started_at, candidate, who, document_id): (Option<String>, String, String, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(&format!(
            "SELECT s.investigation_id, s.title, s.started_at, t.name, {WHO}, s.document_id FROM live_sessions s
             LEFT JOIN targets t ON t.id = s.target_id WHERE s.id = ?"
        ))
        .bind(session_id)
        .fetch_one(pool)
        .await?;
    let rows: Vec<(f64, f64, String, Option<String>, Option<String>, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT g.start_time, g.end_time, g.text, g.voice_id, t.name,
                CASE WHEN v.role = 'interviewer' THEN 'Entrevistador' ELSE v.label END, v.number FROM live_segments g
         LEFT JOIN live_voices v ON v.session_id = g.session_id AND v.voice_id = g.voice_id
         LEFT JOIN targets t ON t.id = v.target_id
         WHERE g.session_id = ? ORDER BY g.start_time",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let candidate = match &inv {
        Some(_) => candidate.unwrap_or_else(|| "Candidato".to_string()),
        None => "Outra voz".to_string(),
    };
    let segments: Vec<TimedText> = rows
        .into_iter()
        .map(|(start, end, text, voice, person, label, number)| {
            let who = match (voice, person, label, number) {
                (None, ..) => candidate.clone(),
                (_, Some(person), _, _) => person,
                (_, _, Some(label), _) => label,
                (_, _, _, Some(number)) => format!("Voz {number}"),
                _ => "Outra voz".to_string(),
            };
            TimedText { start, end, text: format!("{who}: {text}") }
        })
        .collect();
    let text = segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
    let segments = serde_json::to_string(&segments)?;
    let investigations: Vec<String> = match inv {
        Some(inv) => vec![inv],
        None => sqlx::query_scalar(
            "SELECT t.investigation_id FROM live_participants p JOIN targets t ON t.id = p.target_id
             WHERE p.session_id = ? GROUP BY t.investigation_id ORDER BY MIN(p.position)",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await?,
    };
    let url = document_url(session_id);
    let mut first = document_id;
    for (i, inv) in investigations.iter().enumerate() {
        let saved = collect::save_document(
            pool,
            NewDocument {
                investigation_id: inv,
                source_id: None,
                url: &url,
                kind: "live",
                title: &title,
                author: Some(who.as_deref().unwrap_or(&candidate)),
                site: Some("Ao vivo"),
                published_at: Some(&started_at),
                lang: Some("pt"),
                text: &text,
                segments: Some(segments.clone()),
                note: None,
            },
        )
        .await?;
        let Some(id) = saved else { continue };
        if i == 0 {
            sqlx::query("UPDATE live_sessions SET document_id = ? WHERE id = ?").bind(&id).bind(session_id).execute(pool).await?;
            first = Some(id.clone());
        }
        sqlx::query("UPDATE documents SET analyzed_at = NULL WHERE id = ?").bind(&id).execute(pool).await?;
        match indexer::embedder(pool).await {
            Ok(embedder) => {
                if let Err(e) = indexer::index_document(pool, embedder.as_ref(), &id).await {
                    log::warn!("Live: indexing the transcript failed: {e:#}");
                }
            }
            Err(e) => log::warn!("Live: no search index for the transcript: {e:#}"),
        }
    }
    Ok(first)
}

/// The address of a session's documents (one per investigation, for a debate).
pub fn document_url(session_id: &str) -> String {
    format!("voga://live/{session_id}")
}

/// Keeps what is known about a voice of a session.
pub async fn keep_voice(pool: &SqlitePool, session_id: &str, change: &VoiceChange) -> Result<()> {
    let v = &change.view;
    sqlx::query(
        "INSERT INTO live_voices (session_id, voice_id, number, target_id, label, role, centroid, seconds, resolved, recognized)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(session_id, voice_id) DO UPDATE SET number = excluded.number, target_id = excluded.target_id,
            label = excluded.label, role = excluded.role, centroid = excluded.centroid, seconds = excluded.seconds,
            resolved = excluded.resolved, recognized = excluded.recognized",
    )
    .bind(session_id)
    .bind(&v.voice_id)
    .bind(v.number as i64)
    .bind(&v.target_id)
    .bind(&v.label)
    .bind(&v.role)
    .bind(crate::rag::store::encode_vector(&change.centroid))
    .bind(v.seconds)
    .bind(v.resolved)
    .bind(v.recognized)
    .execute(pool)
    .await?;
    Ok(())
}

/// A voice of the session in progress was given to a person followed: its lines so far are checked.
pub async fn refeed(pool: &SqlitePool, session_id: &str, voice_id: &str, target_id: &str) -> Result<()> {
    let lines: Vec<(f64, f64, String)> =
        sqlx::query_as("SELECT start_time, end_time, text FROM live_segments WHERE session_id = ? AND voice_id = ? ORDER BY start_time")
            .bind(session_id)
            .bind(voice_id)
            .fetch_all(pool)
            .await?;
    for (start, end, text) in lines {
        check::heard(Line { start, end, text, target_id: target_id.to_string() });
    }
    Ok(())
}

/// Someone given a voice after the session ended: their claims are found in their lines (only
/// detected, to check on demand).
pub async fn detect_person<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, session_id: &str, target_id: &str) -> Result<()> {
    let (started_at,): (String,) = sqlx::query_as("SELECT started_at FROM live_sessions WHERE id = ?").bind(session_id).fetch_one(pool).await?;
    let lines: Vec<(f64, f64, String)> = sqlx::query_as(
        "SELECT g.start_time, g.end_time, g.text FROM live_segments g
         JOIN live_voices v ON v.session_id = g.session_id AND v.voice_id = g.voice_id
         WHERE g.session_id = ? AND v.target_id = ? ORDER BY g.start_time",
    )
    .bind(session_id)
    .bind(target_id)
    .fetch_all(pool)
    .await?;
    let llm = ConfiguredChatModel::from_settings(pool).await?.deterministic();
    let embedder = indexer::embedder(pool).await?;
    let app = app.clone();
    let mut checker = Checker::new(
        pool.clone(),
        Arc::new(llm),
        Arc::from(embedder),
        session_id.to_string(),
        people(pool, session_id).await?,
        started_at.get(..10).unwrap_or(&started_at).to_string(),
        Box::new(move |view| {
            let _ = app.emit(check::EVENT, view);
        }),
    );
    checker.auto.store(false, Ordering::SeqCst);
    for (start, end, text) in lines {
        checker.heard(Line { start, end, text, target_id: target_id.to_string() }).await;
    }
    checker.flush().await;
    Ok(())
}

/// What the voices thread says about a voice: kept, shown, and its lines checked when a
/// voiceprint named it.
fn on_voice<R: Runtime>(app: AppHandle<R>, pool: SqlitePool, session_id: String) -> impl Fn(VoiceChange) + Send + 'static {
    move |change: VoiceChange| {
        tauri::async_runtime::block_on(async {
            if let Err(e) = keep_voice(&pool, &session_id, &change).await {
                log::warn!("Live: a voice was not kept: {e}");
            }
            if let Some(target) = &change.view.target_id {
                let _ = refeed(&pool, &session_id, &change.view.voice_id, target).await;
            }
        });
        let _ = app.emit(VOICE_EVENT, json!({ "sessionId": session_id, "voice": change.view }));
    }
}

#[allow(clippy::too_many_arguments)]
fn run_audio<R: Runtime>(
    app: AppHandle<R>,
    microphone: Option<String>,
    system: bool,
    wav_path: PathBuf,
    stop: Arc<AtomicBool>,
    speech: mpsc::Sender<Speech>,
    opened: mpsc::SyncSender<Result<()>>,
    position: Arc<AtomicU64>,
) {
    let setup = (|| -> Result<_> { Ok((Vad::new()?, wav::Writer::create(&wav_path)?, Capture::open(microphone.as_deref(), system)?)) })();
    let (mut vad, mut writer, mut capture) = match setup {
        Ok(setup) => {
            let _ = opened.send(Ok(()));
            setup
        }
        Err(e) => {
            let _ = opened.send(Err(e));
            return;
        }
    };
    let hand_over = |s: Speech| {
        voices::offer(&s.samples, s.start, s.end);
        let _ = speech.send(s);
    };
    let (mut last_level, mut last_sync, mut loudest) = (Instant::now(), Instant::now(), 0.0f32);
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(READ_EVERY);
        let samples = capture.read();
        if samples.is_empty() {
            continue;
        }
        position.fetch_add(samples.len() as u64, Ordering::SeqCst);
        let _ = writer.write(&samples);
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        loudest = loudest.max(rms);
        if last_level.elapsed() >= LEVEL_EVERY {
            // -60 dB to 0 dB as 0 to 1
            let level = ((20.0 * loudest.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
            let _ = app.emit(LEVEL_EVENT, json!({ "level": level }));
            (last_level, loudest) = (Instant::now(), 0.0);
        }
        match vad.push(&samples) {
            Ok(done) => done.into_iter().for_each(hand_over),
            Err(e) => log::warn!("Live: {e:#}"),
        }
        if last_sync.elapsed() >= WAV_SYNC_EVERY {
            let _ = writer.sync();
            last_sync = Instant::now();
        }
    }
    drop(capture);
    if let Some(last) = vad.flush() {
        hand_over(last);
    }
    let _ = writer.sync();
}

fn run_transcription<R: Runtime>(app: AppHandle<R>, pool: SqlitePool, session: Session, engine: Engine, speech: mpsc::Receiver<Speech>, position: Arc<AtomicU64>) {
    while let Ok(stretch) = speech.recv() {
        match engine.transcribe(&stretch.samples, &session.hint) {
            Ok((text, _)) if text.is_empty() => {}
            Ok((text, words)) => {
                let duration = stretch.end - stretch.start;
                let pieces = split_at_pauses(&text, &words).unwrap_or_else(|| vec![Piece { text, start: 0.0, end: duration }]);
                // Cut where the voice changes, joined again where it does not
                let mut lines = rejoin(
                    pieces
                        .into_iter()
                        .map(|p| {
                            let who = voices::voice_during(stretch.start + p.start, stretch.start + p.end);
                            (p, who)
                        })
                        .collect(),
                );
                cover(&mut lines, duration);
                for (piece, voice) in lines {
                    if let Err(e) = keep_line(&app, &pool, &session, stretch.start + piece.start, stretch.start + piece.end, piece.text, voice) {
                        log::warn!("Live: a line was not kept: {e:#}");
                    }
                }
            }
            Err(e) => log::warn!("Live: transcription failed: {e:#}"),
        }
        let heard = position.load(Ordering::SeqCst) as f64 / SAMPLE_RATE as f64;
        update(&app, |s| s.behind = ((heard - stretch.end).max(0.0) * 10.0).round() / 10.0);
    }
}

fn keep_line<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool, session: &Session, start: f64, end: f64, text: String, voice: Option<String>) -> Result<()> {
    let id = tauri::async_runtime::block_on(
        sqlx::query("INSERT INTO live_segments (session_id, start_time, end_time, text, voice_id) VALUES (?, ?, ?, ?, ?)")
            .bind(&session.id)
            .bind(start)
            .bind(end)
            .bind(&text)
            .bind(&voice)
            .execute(pool),
    )?
    .last_insert_rowid();
    let _ = app.emit(TRANSCRIPT_EVENT, LineView { id, session_id: session.id.clone(), start, end, text: text.clone(), voice_id: voice.clone() });
    // Without voices every line is the candidate's; with them, only a voice given to someone followed is checked
    let person = if session.voices { voice.as_deref().and_then(voices::person_of) } else { session.candidate.clone() };
    if let Some(target_id) = person {
        check::heard(Line { start, end, text, target_id });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_session_becomes_a_document_with_who_said_each_line() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for sql in [
            "INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Teste', '2026')",
            "INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', 'Marina Exemplo', '2026')",
            "INSERT INTO live_sessions (id, investigation_id, target_id, title, started_at, status) VALUES ('s1', 'inv', 't1', 'Debate', '2026-10-22T20:00:00Z', 'recording')",
            "INSERT INTO live_voices (session_id, voice_id, number, target_id, resolved) VALUES ('s1', 'voice-1', 1, 't1', 1)",
            "INSERT INTO live_voices (session_id, voice_id, number) VALUES ('s1', 'voice-2', 2)",
            "INSERT INTO live_segments (session_id, start_time, end_time, text, voice_id) VALUES
                ('s1', 4, 9, 'eu votei contra a reforma', 'voice-1'), ('s1', 10, 12, 'isso não é verdade', 'voice-2'),
                ('s1', 0, 3, 'boa noite', NULL), ('s1', 13, 14, 'tempo', 'voice-9')",
            "INSERT INTO live_checks (id, session_id, start_time, end_time, quote, claim, created_at) VALUES ('c1', 's1', 4, 9, 'q', 'c', 'x')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        recover(&pool).await.unwrap();

        let (status, ended_at, document_id): (String, String, String) =
            sqlx::query_as("SELECT status, ended_at, document_id FROM live_sessions WHERE id = 's1'").fetch_one(&pool).await.unwrap();
        assert_eq!(status, "interrupted");
        assert!(ended_at.starts_with("2026-10-22T20:00:14"), "{ended_at}");
        let (kind, text, analyzed): (String, String, Option<String>) =
            sqlx::query_as("SELECT kind, text, analyzed_at FROM documents WHERE id = ?").bind(&document_id).fetch_one(&pool).await.unwrap();
        assert_eq!(kind, "live");
        assert_eq!(text, "Marina Exemplo: boa noite\nMarina Exemplo: eu votei contra a reforma\nVoz 2: isso não é verdade\nOutra voz: tempo");
        assert!(analyzed.is_none(), "the analysis reads it");
        let (verdict,): (String,) = sqlx::query_as("SELECT verdict FROM live_checks WHERE id = 'c1'").fetch_one(&pool).await.unwrap();
        assert_eq!(verdict, "unverified");

        // Naming voice 2 later rewrites the document
        sqlx::query("UPDATE live_voices SET label = 'Moderadora' WHERE voice_id = 'voice-2'").execute(&pool).await.unwrap();
        assert_eq!(save_document(&pool, "s1").await.unwrap(), Some(document_id.clone()));
        let (text,): (String,) = sqlx::query_as("SELECT text FROM documents WHERE id = ?").bind(&document_id).fetch_one(&pool).await.unwrap();
        assert!(text.contains("Moderadora: isso não é verdade"));

        // The interviewer, as the voices thread keeps it, is named so
        let view = |role: Option<&str>| voices::VoiceView {
            voice_id: "voice-2".into(),
            number: 2,
            target_id: None,
            label: None,
            role: role.map(String::from),
            resolved: true,
            recognized: false,
            seconds: 5.0,
            heard: Vec::new(),
            suggestions: Vec::new(),
        };
        keep_voice(&pool, "s1", &VoiceChange { view: view(Some("interviewer")), centroid: vec![0.5, 0.5] }).await.unwrap();
        let (role,): (Option<String>,) = sqlx::query_as("SELECT role FROM live_voices WHERE voice_id = 'voice-2'").fetch_one(&pool).await.unwrap();
        assert_eq!(role.as_deref(), Some("interviewer"));
        save_document(&pool, "s1").await.unwrap();
        let (text,): (String,) = sqlx::query_as("SELECT text FROM documents WHERE id = ?").bind(&document_id).fetch_one(&pool).await.unwrap();
        assert!(text.contains("Entrevistador: isso não é verdade"), "{text}");
    }

    #[tokio::test]
    async fn the_debates_migration_keeps_every_session() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        let all = sqlx::migrate!("./migrations");
        let mut before = sqlx::migrate!("./migrations");
        before.migrations = all.migrations.iter().filter(|m| m.version < 20261025000000).cloned().collect::<Vec<_>>().into();
        before.run(&pool).await.unwrap();
        for sql in [
            "INSERT INTO investigations (id, name, created_at) VALUES ('inv', 'Teste', '2026')",
            "INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES ('t1', 'inv', 'person', 'Marina Exemplo', '2026')",
            "INSERT INTO live_sessions (id, investigation_id, target_id, title, started_at, status, auto_check) VALUES ('s1', 'inv', 't1', 'Debate', '2026', 'done', 0)",
            "INSERT INTO live_voices (session_id, voice_id, number, target_id, resolved, role) VALUES ('s1', 'voice-1', 1, 't1', 1, 'interviewer')",
            "INSERT INTO live_segments (session_id, start_time, end_time, text, voice_id) VALUES ('s1', 1, 2, 'boa noite', 'voice-1')",
            "INSERT INTO live_checks (id, session_id, start_time, end_time, quote, claim, created_at, kind, query) VALUES ('c1', 's1', 1, 2, 'q', 'c', 'x', 'denial', 'reforma')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        all.run(&pool).await.unwrap();
        let (auto,): (bool,) = sqlx::query_as("SELECT auto_check FROM live_sessions WHERE id = 's1'").fetch_one(&pool).await.unwrap();
        assert!(!auto);
        let (role,): (Option<String>,) = sqlx::query_as("SELECT role FROM live_voices WHERE session_id = 's1'").fetch_one(&pool).await.unwrap();
        assert_eq!(role.as_deref(), Some("interviewer"));
        let (text,): (String,) = sqlx::query_as("SELECT text FROM live_segments WHERE session_id = 's1'").fetch_one(&pool).await.unwrap();
        assert_eq!(text, "boa noite");
        let (kind, query): (String, String) = sqlx::query_as("SELECT kind, query FROM live_checks WHERE id = 'c1'").fetch_one(&pool).await.unwrap();
        assert_eq!((kind.as_str(), query.as_str()), ("denial", "reforma"));
        // Still deleted with their investigation; a debate needs none
        sqlx::query("INSERT INTO live_sessions (id, title, started_at) VALUES ('d1', 'Debate', '2026')").execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM investigations WHERE id = 'inv'").execute(&pool).await.unwrap();
        let (left,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM live_segments").fetch_one(&pool).await.unwrap();
        assert_eq!(left, 0);
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM live_sessions").fetch_all(&pool).await.unwrap();
        assert_eq!(ids, ["d1"]);
    }

    #[tokio::test]
    async fn a_debate_follows_its_people_and_lands_in_each_ones_investigation() {
        let options = sqlx::sqlite::SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for sql in [
            "INSERT INTO investigations (id, name, created_at) VALUES ('a', 'Marina', '2026'), ('b', 'Otávio', '2026')",
            "INSERT INTO targets (id, investigation_id, kind, name, created_at) VALUES
                ('marina', 'a', 'person', 'Marina Exemplo', '2026'), ('caio', 'a', 'person', 'Caio Exemplo', '2026'), ('otavio', 'b', 'person', 'Otávio Fictício', '2026')",
            "INSERT INTO live_sessions (id, title, started_at, status) VALUES ('d1', 'Debate na TV', '2026-10-22T20:00:00Z', 'recording')",
            "INSERT INTO live_participants (session_id, position, name, target_id) VALUES ('d1', 1, 'Otávio Fictício', 'otavio'), ('d1', 2, 'Marina Exemplo', 'marina')",
            "INSERT INTO live_voices (session_id, voice_id, number, target_id, resolved) VALUES ('d1', 'voice-1', 1, 'marina', 1), ('d1', 'voice-2', 2, 'otavio', 1)",
            "INSERT INTO live_segments (session_id, start_time, end_time, text, voice_id) VALUES
                ('d1', 1, 3, 'votei contra', 'voice-1'), ('d1', 4, 6, 'mentira', 'voice-2'), ('d1', 7, 8, 'aplausos', NULL)",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        // Its people are the participants in order, not everyone their investigations follow
        let people: Vec<(String, String)> = followed(&pool, "d1").await.unwrap().into_iter().map(|f| (f.id, f.investigation_id)).collect();
        assert_eq!(people, [("otavio".to_string(), "b".to_string()), ("marina".to_string(), "a".to_string())]);
        // Someone named later joins last, once
        add_participant(&pool, "d1", "caio").await.unwrap();
        add_participant(&pool, "d1", "caio").await.unwrap();
        let joined: Vec<(i64, String)> = sqlx::query_as("SELECT position, name FROM live_participants WHERE session_id = 'd1' ORDER BY position").fetch_all(&pool).await.unwrap();
        assert_eq!(joined.last(), Some(&(3, "Caio Exemplo".to_string())));
        assert_eq!(joined.len(), 3);

        recover(&pool).await.unwrap();
        let docs: Vec<(String, String, String, String)> =
            sqlx::query_as("SELECT investigation_id, author, text, url FROM documents ORDER BY investigation_id").fetch_all(&pool).await.unwrap();
        assert_eq!(docs.len(), 2, "one per investigation");
        assert_eq!(docs[0].1, "Otávio Fictício × Marina Exemplo × Caio Exemplo");
        assert_eq!(docs[0].2, "Marina Exemplo: votei contra\nOtávio Fictício: mentira\nOutra voz: aplausos");
        assert_eq!(docs[0].2, docs[1].2);
        assert!(docs.iter().all(|d| d.3 == document_url("d1")));
        let (document_id,): (Option<String>,) = sqlx::query_as("SELECT document_id FROM live_sessions WHERE id = 'd1'").fetch_one(&pool).await.unwrap();
        let (first,): (String,) = sqlx::query_as("SELECT investigation_id FROM documents WHERE id = ?").bind(document_id).fetch_one(&pool).await.unwrap();
        assert_eq!(first, "b", "the first person's");
    }
}
