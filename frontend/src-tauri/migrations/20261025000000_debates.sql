-- Debates: live sessions outside every investigation (investigation_id NULL). Their people are
-- the participants chosen when the debate starts, each followed in their own investigation
-- (made for them when they had none), whose records check what they say.
--
-- SQLite cannot drop a NOT NULL in place, and dropping live_sessions to rebuild it would delete
-- the lines, voices and checks that reference it (foreign keys are on): the four tables are
-- copied, rebuilt as they were (investigation_id now nullable) and filled again.
CREATE TABLE live_sessions_old AS SELECT * FROM live_sessions;
CREATE TABLE live_segments_old AS SELECT * FROM live_segments;
CREATE TABLE live_voices_old AS SELECT * FROM live_voices;
CREATE TABLE live_checks_old AS SELECT * FROM live_checks;
DROP TABLE live_checks;
DROP TABLE live_voices;
DROP TABLE live_segments;
DROP TABLE live_sessions;

CREATE TABLE live_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT REFERENCES investigations(id) ON DELETE CASCADE, -- NULL: a debate
    target_id TEXT REFERENCES targets(id) ON DELETE SET NULL, -- the candidate
    title TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    mic_device TEXT,
    system_audio INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    status TEXT NOT NULL DEFAULT 'recording',
    audio_path TEXT,
    document_id TEXT REFERENCES documents(id) ON DELETE SET NULL,
    auto_check INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX idx_live_sessions_investigation ON live_sessions(investigation_id, started_at);

CREATE TABLE live_segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    start_time REAL NOT NULL,
    end_time REAL NOT NULL,
    text TEXT NOT NULL,
    voice_id TEXT
);
CREATE INDEX idx_live_segments_session ON live_segments(session_id, start_time);

CREATE TABLE live_voices (
    session_id TEXT NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    voice_id TEXT NOT NULL,
    number INTEGER NOT NULL,
    target_id TEXT REFERENCES targets(id) ON DELETE SET NULL,
    label TEXT,
    centroid BLOB,
    seconds REAL NOT NULL DEFAULT 0,
    resolved INTEGER NOT NULL DEFAULT 0,
    recognized INTEGER NOT NULL DEFAULT 0,
    role TEXT,
    PRIMARY KEY (session_id, voice_id)
);

CREATE TABLE live_checks (
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
    evidence TEXT NOT NULL DEFAULT '[]',
    web_searched INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'suggested',
    error TEXT,
    created_at TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'affirmation',
    query TEXT NOT NULL DEFAULT ''
);
CREATE INDEX idx_live_checks_session ON live_checks(session_id, start_time);

INSERT INTO live_sessions (id, investigation_id, target_id, title, notes, mic_device, system_audio, started_at, ended_at, status, audio_path, document_id, auto_check)
    SELECT id, investigation_id, target_id, title, notes, mic_device, system_audio, started_at, ended_at, status, audio_path, document_id, auto_check FROM live_sessions_old;
INSERT INTO live_segments (id, session_id, start_time, end_time, text, voice_id)
    SELECT id, session_id, start_time, end_time, text, voice_id FROM live_segments_old;
INSERT INTO live_voices (session_id, voice_id, number, target_id, label, centroid, seconds, resolved, recognized, role)
    SELECT session_id, voice_id, number, target_id, label, centroid, seconds, resolved, recognized, role FROM live_voices_old;
INSERT INTO live_checks (id, session_id, target_id, start_time, end_time, quote, claim, topic, verdict, confidence, rationale, evidence, web_searched, status, error, created_at, kind, query)
    SELECT id, session_id, target_id, start_time, end_time, quote, claim, topic, verdict, confidence, rationale, evidence, web_searched, status, error, created_at, kind, query FROM live_checks_old;
DROP TABLE live_checks_old;
DROP TABLE live_voices_old;
DROP TABLE live_segments_old;
DROP TABLE live_sessions_old;

-- Who debates, in order ("Pessoa 1", "Pessoa 2"…): a person followed in some investigation.
-- The name stays when the person stops being followed.
CREATE TABLE live_participants (
    session_id TEXT NOT NULL REFERENCES live_sessions(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    target_id TEXT REFERENCES targets(id) ON DELETE SET NULL,
    PRIMARY KEY (session_id, position)
);
