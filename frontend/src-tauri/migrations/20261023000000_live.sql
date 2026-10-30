-- Live sessions (Ao vivo): a candidate heard as they speak, transcribed on this computer, with
-- who said each line (voices) and the AI's fact checks of what the people followed say.
-- Saved line by line, so a session survives the app closing; when it ends it also becomes a
-- `live` document of the investigation (document_id).
CREATE TABLE IF NOT EXISTS live_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    target_id TEXT REFERENCES targets(id) ON DELETE SET NULL, -- the candidate
    title TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',            -- what the person typed during the session
    mic_device TEXT,                           -- the microphone heard (NULL: none)
    system_audio INTEGER NOT NULL DEFAULT 0,   -- the computer's sound was heard
    started_at TEXT NOT NULL,
    ended_at TEXT,
    status TEXT NOT NULL DEFAULT 'recording',  -- recording | done | interrupted
    audio_path TEXT,                           -- 16 kHz mono WAV in <app data>/live
    document_id TEXT REFERENCES documents(id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_live_sessions_investigation ON live_sessions(investigation_id, started_at);

-- The transcript, in seconds from the session start; voice_id is the voice heard (live_voices).
CREATE TABLE IF NOT EXISTS live_segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    start_time REAL NOT NULL,
    end_time REAL NOT NULL,
    text TEXT NOT NULL,
    voice_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_live_segments_session ON live_segments(session_id, start_time);

-- Voices told apart in a session, and who each one is: a person followed (target_id), a name
-- typed (label), or nobody (resolved: the user answered, "não sei" included; recognized: named by
-- a voiceprint, not by the user). centroid: the voice's embedding, to learn the target's voiceprint.
CREATE TABLE IF NOT EXISTS live_voices (
    session_id TEXT NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    voice_id TEXT NOT NULL,
    number INTEGER NOT NULL,                   -- "Voz N"
    target_id TEXT REFERENCES targets(id) ON DELETE SET NULL,
    label TEXT,
    centroid BLOB,
    seconds REAL NOT NULL DEFAULT 0,
    resolved INTEGER NOT NULL DEFAULT 0,
    recognized INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, voice_id)
);

-- Fact checks: a claim a person followed made (quote: their words, as transcribed), checked
-- against the records. verdict: checking | true | false | partly | unverified; status: the
-- person's review (suggested until confirmed or rejected, as every AI finding).
CREATE TABLE IF NOT EXISTS live_checks (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    target_id TEXT REFERENCES targets(id) ON DELETE SET NULL,
    start_time REAL NOT NULL,
    end_time REAL NOT NULL,
    quote TEXT NOT NULL,
    claim TEXT NOT NULL,
    topic TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT 'checking',
    confidence REAL,
    rationale TEXT,
    evidence TEXT NOT NULL DEFAULT '[]',       -- JSON [{documentId, chunkId, title, kind, date, url, excerpt, startTime, actor}]
    web_searched INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'suggested',  -- suggested | confirmed | rejected
    error TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_live_checks_session ON live_checks(session_id, start_time);

-- A person's voiceprint (mean voice embedding), learned only from the user's answers and only
-- with consent to voice recognition (prefs voice_recognition).
ALTER TABLE targets ADD COLUMN voiceprint BLOB;
ALTER TABLE targets ADD COLUMN voiceprint_samples INTEGER NOT NULL DEFAULT 0;
