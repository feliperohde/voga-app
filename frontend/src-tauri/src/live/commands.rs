//! Tauri commands of live sessions (the Ao vivo and Debate pages, and Settings › AI).

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};

use super::audio::{self, Microphone};
use super::check::{self, CheckView};
use super::session::{self, Followed, LineView, LiveState, StartOptions};
use super::voices::{self, VoiceView};
use super::wav;
use crate::llm::ConfiguredChatModel;
use crate::prefs;
use crate::rag::store::{decode_vector, encode_vector};
use crate::state::AppState;

type CommandResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Most of a voice played to tell who it is.
const SAMPLE_SECONDS: f64 = 6.0;
/// Longest clip of a session's sound served.
const MAX_CLIP_SECONDS: f64 = 30.0;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStatus {
    pub state: LiveState,
    /// The Whisper model a session would use here (None: none downloaded).
    pub transcription_model: Option<String>,
    /// "builtin" (Whisper here) or the provider that transcribes, with its model.
    pub transcription_provider: String,
    pub transcription_remote_model: String,
    pub voice_model: bool,
    pub voice_consent: bool,
    pub system_audio: bool,
    pub microphones: Vec<Microphone>,
    /// An AI model is set up: claims can be checked.
    pub chat_model: bool,
}

#[tauri::command]
pub async fn live_status(state: tauri::State<'_, AppState>) -> CommandResult<LiveStatus> {
    let pool = state.db_manager.pool();
    Ok(LiveStatus {
        state: session::state(),
        transcription_model: session::transcription_model(pool).await,
        transcription_provider: prefs::get(pool, session::TRANSCRIPTION_PROVIDER).await.ok().flatten().filter(|p| !p.is_empty()).unwrap_or_else(|| "builtin".into()),
        transcription_remote_model: prefs::get(pool, session::TRANSCRIPTION_REMOTE_MODEL).await.ok().flatten().unwrap_or_default(),
        voice_model: crate::builtin::downloaded_of_kind(crate::builtin::catalog::ModelKind::Voice).is_some(),
        voice_consent: session::voice_consent(pool).await,
        system_audio: audio::system_audio_available(),
        microphones: audio::microphones(),
        chat_model: ConfiguredChatModel::from_settings(pool).await.is_ok(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSettings {
    pub voice_consent: bool,
    /// The Whisper model to use ("" or None: the best downloaded).
    pub transcription_model: Option<String>,
    /// "builtin", or a provider transcribing instead (left as is when None).
    #[serde(default)]
    pub transcription_provider: Option<String>,
    #[serde(default)]
    pub transcription_remote_model: Option<String>,
}

#[tauri::command]
pub async fn live_save_settings(state: tauri::State<'_, AppState>, settings: LiveSettings) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    prefs::set(pool, session::VOICE_CONSENT, if settings.voice_consent { "1" } else { "0" }).await.map_err(err)?;
    if !settings.voice_consent {
        // Consent withdrawn: the voices learned go with it (LGPD)
        sqlx::query("UPDATE targets SET voiceprint = NULL, voiceprint_samples = 0").execute(pool).await.map_err(err)?;
        sqlx::query("UPDATE live_voices SET centroid = NULL").execute(pool).await.map_err(err)?;
    }
    prefs::set(pool, session::TRANSCRIPTION_MODEL, settings.transcription_model.as_deref().unwrap_or_default()).await.map_err(err)?;
    if let Some(provider) = settings.transcription_provider {
        prefs::set(pool, session::TRANSCRIPTION_PROVIDER, &provider).await.map_err(err)?;
    }
    if let Some(model) = settings.transcription_remote_model {
        prefs::set(pool, session::TRANSCRIPTION_REMOTE_MODEL, model.trim()).await.map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn live_start<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    investigation_id: String,
    target_id: String,
    title: String,
    microphone: Option<String>,
    system_audio: bool,
    auto_check: bool,
) -> CommandResult<String> {
    let pool = state.db_manager.pool();
    let options = StartOptions { investigation_id: Some(investigation_id), target_id: Some(target_id), participants: Vec::new(), title, microphone, system_audio, auto_check };
    session::start(&app, pool, options).await.map_err(|e| format!("{e:#}"))
}

/// A person of a new debate: one followed in some investigation, or a name nobody follows yet.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantInput {
    pub target_id: Option<String>,
    #[serde(default)]
    pub name: String,
}

/// Starts a debate: a session outside every investigation, whose people are checked each in
/// their own investigation. A person nobody follows gets an investigation of their own first.
#[tauri::command]
pub async fn debate_start<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    title: String,
    participants: Vec<ParticipantInput>,
    microphone: Option<String>,
    system_audio: bool,
    auto_check: bool,
) -> CommandResult<String> {
    let pool = state.db_manager.pool();
    // Not after making investigations for nothing
    if session::state().phase != "idle" {
        return Err("A live session is already running".into());
    }
    let mut ids = Vec::new();
    for participant in participants {
        ids.push(match participant.target_id {
            Some(id) => id,
            None if participant.name.trim().is_empty() => return Err("Give each person of the debate a name".into()),
            None => crate::investigations::follow_person(pool, &participant.name).await?,
        });
    }
    let options = StartOptions { investigation_id: None, target_id: None, participants: ids, title, microphone, system_audio, auto_check };
    session::start(&app, pool, options).await.map_err(|e| format!("{e:#}"))
}

/// Someone named in a debate going on or over joins its people: followed in their own
/// investigation (made when nobody follows them).
#[tauri::command]
pub async fn debate_add_participant(state: tauri::State<'_, AppState>, session_id: String, name: String) -> CommandResult<String> {
    let pool = state.db_manager.pool();
    let target_id = crate::investigations::follow_person(pool, &name).await?;
    session::add_participant(pool, &session_id, &target_id).await.map_err(err)?;
    Ok(target_id)
}

/// A person followed somewhere, to pick for a debate.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownPerson {
    pub target_id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub investigation_id: String,
    pub investigation_name: String,
}

/// Everyone followed, as `investigations::follow_person` prefers them (the investigation with
/// the most documents first): the debate form links a name the same way.
#[tauri::command]
pub async fn debate_people(state: tauri::State<'_, AppState>) -> CommandResult<Vec<KnownPerson>> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT t.id, t.name, t.aliases, i.id, i.name FROM targets t JOIN investigations i ON i.id = t.investigation_id
         WHERE t.kind = 'person' ORDER BY (SELECT COUNT(*) FROM documents d WHERE d.investigation_id = t.investigation_id) DESC, t.created_at",
    )
    .fetch_all(state.db_manager.pool())
    .await
    .map_err(err)?;
    Ok(rows
        .into_iter()
        .map(|(target_id, name, aliases, investigation_id, investigation_name)| KnownPerson {
            target_id,
            name,
            aliases: serde_json::from_str(&aliases).unwrap_or_default(),
            investigation_id,
            investigation_name,
        })
        .collect())
}

#[tauri::command]
pub async fn live_stop<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> CommandResult<()> {
    session::stop(&app, state.db_manager.pool().clone()).map_err(err)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    /// None: a debate.
    pub investigation_id: Option<String>,
    pub target_id: Option<String>,
    /// The candidate, or a debate's people ("A × B").
    pub target_name: Option<String>,
    pub title: String,
    pub notes: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// recording | done | interrupted
    pub status: String,
    pub document_id: Option<String>,
    pub has_audio: bool,
    /// Claims are checked as found (else only detected).
    pub auto_check: bool,
    pub lines: i64,
    pub true_count: i64,
    pub false_count: i64,
    pub partly_count: i64,
    pub unverified_count: i64,
}

const SUMMARY: &str = "SELECT s.id, s.investigation_id, s.target_id, {WHO} AS target_name, s.title, s.notes, s.started_at, s.ended_at,
        s.status, s.document_id, s.audio_path IS NOT NULL AS has_audio, s.auto_check,
        (SELECT COUNT(*) FROM live_segments g WHERE g.session_id = s.id) AS lines,
        (SELECT COUNT(*) FROM live_checks c WHERE c.session_id = s.id AND c.verdict = 'true') AS true_count,
        (SELECT COUNT(*) FROM live_checks c WHERE c.session_id = s.id AND c.verdict = 'false') AS false_count,
        (SELECT COUNT(*) FROM live_checks c WHERE c.session_id = s.id AND c.verdict = 'partly') AS partly_count,
        (SELECT COUNT(*) FROM live_checks c WHERE c.session_id = s.id AND c.verdict = 'unverified') AS unverified_count
    FROM live_sessions s LEFT JOIN targets t ON t.id = s.target_id";

fn summary() -> String {
    SUMMARY.replace("{WHO}", session::WHO)
}

#[tauri::command]
pub async fn live_list_sessions(state: tauri::State<'_, AppState>, investigation_id: String) -> CommandResult<Vec<SessionSummary>> {
    sqlx::query_as(&format!("{} WHERE s.investigation_id = ? ORDER BY s.started_at DESC", summary()))
        .bind(investigation_id)
        .fetch_all(state.db_manager.pool())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn debate_list(state: tauri::State<'_, AppState>) -> CommandResult<Vec<SessionSummary>> {
    sqlx::query_as(&format!("{} WHERE s.investigation_id IS NULL ORDER BY s.started_at DESC", summary()))
        .fetch_all(state.db_manager.pool())
        .await
        .map_err(err)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session: SessionSummary,
    /// Who the session follows (a debate's people in order), and where each one's records are.
    pub people: Vec<Followed>,
    pub lines: Vec<LineView>,
    pub voices: Vec<VoiceView>,
    pub checks: Vec<CheckView>,
}

#[tauri::command]
pub async fn live_get_session(state: tauri::State<'_, AppState>, id: String) -> CommandResult<SessionDetail> {
    let pool = state.db_manager.pool();
    let session: SessionSummary = sqlx::query_as(&format!("{} WHERE s.id = ?", summary())).bind(&id).fetch_one(pool).await.map_err(err)?;
    let lines: Vec<LineView> =
        sqlx::query_as("SELECT id, session_id, start_time, end_time, text, voice_id FROM live_segments WHERE session_id = ? ORDER BY start_time")
            .bind(&id)
            .fetch_all(pool)
            .await
            .map_err(err)?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, i64, Option<String>, Option<String>, Option<String>, Option<Vec<u8>>, f64, bool, bool)> = sqlx::query_as(
        "SELECT voice_id, number, target_id, label, role, centroid, seconds, resolved, recognized FROM live_voices WHERE session_id = ? ORDER BY number",
    )
    .bind(&id)
    .fetch_all(pool)
    .await
    .map_err(err)?;
    // Voices still waiting for an answer get their suggestions again (after a reload)
    let followed = session::followed(pool, &id).await.map_err(err)?;
    let people = session::voice_people(pool, &id).await.map_err(err)?;
    let taken: Vec<String> = rows.iter().filter_map(|r| r.2.clone()).collect();
    let voices = rows
        .into_iter()
        .map(|(voice_id, number, target_id, label, role, centroid, seconds, resolved, recognized)| VoiceView {
            suggestions: match (&centroid, resolved) {
                (Some(centroid), false) => voices::suggestions_for(&decode_vector(centroid), &people, &taken).0,
                _ => Vec::new(),
            },
            voice_id,
            number: number as usize,
            target_id,
            label,
            role,
            resolved,
            recognized,
            seconds,
            heard: Vec::new(),
        })
        .collect();
    let checks = check::list(pool, &id).await.map_err(err)?;
    Ok(SessionDetail { session, people: followed, lines, voices, checks })
}

/// Renames a session, keeps the notes typed in it, or turns its automatic checks on or off.
#[tauri::command]
pub async fn live_update_session(
    state: tauri::State<'_, AppState>,
    id: String,
    title: Option<String>,
    notes: Option<String>,
    auto_check: Option<bool>,
) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    sqlx::query("UPDATE live_sessions SET title = COALESCE(?, title), notes = COALESCE(?, notes), auto_check = COALESCE(?, auto_check) WHERE id = ?")
        .bind(title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
        .bind(notes)
        .bind(auto_check)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(err)?;
    if let Some(on) = auto_check {
        session::set_auto_check(&id, on);
    }
    let ended: Option<(String,)> = sqlx::query_as("SELECT id FROM live_sessions WHERE id = ? AND document_id IS NOT NULL").bind(&id).fetch_optional(pool).await.map_err(err)?;
    if ended.is_some() {
        session::save_document(pool, &id).await.map_err(err)?;
    }
    Ok(())
}

/// Deletes a session: its lines, voices, checks, sound and documents.
#[tauri::command]
pub async fn live_delete_session(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    if session::active().is_some_and(|active| active == id) {
        return Err("Stop the session before deleting it".into());
    }
    let pool = state.db_manager.pool();
    let (audio_path,): (Option<String>,) = sqlx::query_as("SELECT audio_path FROM live_sessions WHERE id = ?").bind(&id).fetch_one(pool).await.map_err(err)?;
    if let Some(path) = audio_path {
        let _ = std::fs::remove_file(path);
    }
    sqlx::query("DELETE FROM documents WHERE url = ?").bind(session::document_url(&id)).execute(pool).await.map_err(err)?;
    sqlx::query("DELETE FROM live_sessions WHERE id = ?").bind(&id).execute(pool).await.map_err(err)?;
    Ok(())
}

/// Who a voice is: a person followed (target_id), a name (label), the interviewer (role), or neither ("não sei"). The
/// answer teaches the person's voiceprint, with consent; in a session going on, the voice's
/// lines so far are checked; after it, the document gets the new name.
#[tauri::command]
pub async fn live_resolve_voice(
    state: tauri::State<'_, AppState>,
    session_id: String,
    voice_id: String,
    target_id: Option<String>,
    label: Option<String>,
    role: Option<String>,
) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    let role = role.filter(|r| r == "interviewer");
    let target_id = target_id.filter(|_| role.is_none());
    let (previous, centroid): (Option<String>, Option<Vec<u8>>) =
        sqlx::query_as("SELECT target_id, centroid FROM live_voices WHERE session_id = ? AND voice_id = ?")
            .bind(&session_id)
            .bind(&voice_id)
            .fetch_one(pool)
            .await
            .map_err(err)?;
    let label = label.map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && target_id.is_none() && role.is_none());
    let live = session::active().is_some_and(|active| active == session_id);
    let change = if live { voices::resolve(&voice_id, target_id.clone(), label.clone(), role.clone()) } else { None };
    match &change {
        Some(change) => session::keep_voice(pool, &session_id, change).await.map_err(err)?,
        None => {
            sqlx::query("UPDATE live_voices SET target_id = ?, label = ?, role = ?, resolved = 1, recognized = 0 WHERE session_id = ? AND voice_id = ?")
                .bind(&target_id)
                .bind(&label)
                .bind(&role)
                .bind(&session_id)
                .bind(&voice_id)
                .execute(pool)
                .await
                .map_err(err)?;
        }
    }
    let changed = target_id.is_some() && target_id != previous;
    let centroid = change.map(|c| c.centroid).or_else(|| centroid.map(|c| decode_vector(&c)));
    if let (true, Some(target), Some(centroid)) = (changed && session::voice_consent(pool).await, &target_id, centroid) {
        let (voiceprint, samples): (Option<Vec<u8>>, i64) =
            sqlx::query_as("SELECT voiceprint, voiceprint_samples FROM targets WHERE id = ?").bind(target).fetch_one(pool).await.map_err(err)?;
        let learned = voices::learn(voiceprint.map(|v| decode_vector(&v)).as_deref(), samples, &centroid);
        sqlx::query("UPDATE targets SET voiceprint = ?, voiceprint_samples = voiceprint_samples + 1 WHERE id = ?")
            .bind(encode_vector(&learned))
            .bind(target)
            .execute(pool)
            .await
            .map_err(err)?;
    }
    match (live, &target_id) {
        (true, Some(target)) if changed => session::refeed(pool, &session_id, &voice_id, target).await.map_err(err)?,
        (true, _) => {}
        (false, _) => {
            session::save_document(pool, &session_id).await.map_err(err)?;
        }
    }
    Ok(())
}

/// Says a line is another voice's (the voices were mixed up there).
#[tauri::command]
pub async fn live_set_line_voice(state: tauri::State<'_, AppState>, line_id: i64, voice_id: Option<String>) -> CommandResult<()> {
    let pool = state.db_manager.pool();
    let (session_id,): (String,) = sqlx::query_as("SELECT session_id FROM live_segments WHERE id = ?").bind(line_id).fetch_one(pool).await.map_err(err)?;
    sqlx::query("UPDATE live_segments SET voice_id = ? WHERE id = ?").bind(&voice_id).bind(line_id).execute(pool).await.map_err(err)?;
    if !session::active().is_some_and(|active| active == session_id) {
        session::save_document(pool, &session_id).await.map_err(err)?;
    }
    Ok(())
}

/// A few seconds of a voice, as WAV, to hear who it is.
#[tauri::command]
pub async fn live_voice_sample(state: tauri::State<'_, AppState>, session_id: String, voice_id: String) -> CommandResult<tauri::ipc::Response> {
    if session::active().is_some_and(|active| active == session_id) {
        if let Some(sample) = voices::sample_wav(&voice_id) {
            return Ok(tauri::ipc::Response::new(sample));
        }
    }
    // After the session: its longest line
    let line: (f64, f64) = sqlx::query_as(
        "SELECT start_time, end_time FROM live_segments WHERE session_id = ? AND voice_id = ? ORDER BY end_time - start_time DESC LIMIT 1",
    )
    .bind(&session_id)
    .bind(&voice_id)
    .fetch_one(state.db_manager.pool())
    .await
    .map_err(|_| "No sound for this voice".to_string())?;
    clip(state.db_manager.pool(), &session_id, line.0, line.1.min(line.0 + SAMPLE_SECONDS)).await
}

/// The session's sound between two moments (seconds), as WAV.
#[tauri::command]
pub async fn live_clip(state: tauri::State<'_, AppState>, session_id: String, start: f64, end: f64) -> CommandResult<tauri::ipc::Response> {
    clip(state.db_manager.pool(), &session_id, start, end).await
}

async fn clip(pool: &sqlx::SqlitePool, session_id: &str, start: f64, end: f64) -> CommandResult<tauri::ipc::Response> {
    let (path,): (Option<String>,) = sqlx::query_as("SELECT audio_path FROM live_sessions WHERE id = ?").bind(session_id).fetch_one(pool).await.map_err(err)?;
    let path = path.ok_or("This session kept no sound")?;
    let (start, end) = ((start - 0.3).max(0.0), end + 0.3);
    let samples = wav::read_range(std::path::Path::new(&path), start, end.min(start + MAX_CLIP_SECONDS)).map_err(err)?;
    Ok(tauri::ipc::Response::new(wav::encode(&samples)))
}

/// Checks one claim now: during the session or after it.
#[tauri::command]
pub async fn live_check_now<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    let pool = state.db_manager.pool().clone();
    let llm = ConfiguredChatModel::from_settings(&pool).await.map_err(|e| format!("{e:#}"))?.deterministic();
    let embedder = crate::rag::indexer::embedder(&pool).await.map_err(err)?;
    tauri::async_runtime::spawn(async move {
        let emitter = app.clone();
        let on_check = Box::new(move |view: &CheckView| {
            let _ = emitter.emit(check::EVENT, view);
        });
        if let Err(e) = check::check_now(&pool, &id, std::sync::Arc::new(llm), std::sync::Arc::from(embedder), on_check).await {
            log::warn!("Live: checking {id} failed: {e:#}");
            // The card stops waiting
            let _ = sqlx::query("UPDATE live_checks SET verdict = 'detected', error = ? WHERE id = ? AND verdict = 'checking'")
                .bind(e.to_string())
                .bind(&id)
                .execute(&pool)
                .await;
            if let Ok(view) = check::load(&pool, &id).await {
                let _ = app.emit(check::EVENT, view);
            }
        }
    });
    Ok(())
}

/// Finds the claims of someone given a voice after the session ended (to check on demand).
#[tauri::command]
pub async fn live_detect_person<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>, session_id: String, target_id: String) -> CommandResult<()> {
    let pool = state.db_manager.pool().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = session::detect_person(&app, &pool, &session_id, &target_id).await {
            log::warn!("Live: finding the claims of {target_id} failed: {e:#}");
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn live_set_check_status(state: tauri::State<'_, AppState>, id: String, status: String) -> CommandResult<()> {
    check::set_status(state.db_manager.pool(), &id, &status).await.map_err(err)
}
