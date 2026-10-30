-- Voga 0.2.0: investigations, what they follow, where it comes from, what was collected,
-- and the search index over it.

-- A subject of investigation: a politician, a mandate, a theme.
CREATE TABLE IF NOT EXISTS investigations (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

-- Who or what an investigation follows. Names and aliases drive the searches and tell
-- whether a found page is about it.
CREATE TABLE IF NOT EXISTS targets (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT 'person',      -- person | organization | topic
    name TEXT NOT NULL,
    aliases TEXT NOT NULL DEFAULT '[]',       -- JSON array of other names
    external_ids TEXT NOT NULL DEFAULT '{}',  -- JSON {camara, senado, tse}
    created_at TEXT NOT NULL
);

-- Where content comes from, run on a schedule. For news_search and youtube_search,
-- url holds the query.
CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    target_id TEXT REFERENCES targets(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,                       -- page | rss | youtube_video | youtube_channel | news_search | youtube_search
    url TEXT NOT NULL,
    title TEXT,
    every_minutes INTEGER NOT NULL DEFAULT 360,
    next_run_at TEXT NOT NULL,
    last_run_at TEXT,
    last_error TEXT,
    status TEXT NOT NULL DEFAULT 'active',    -- active | paused
    origin TEXT NOT NULL DEFAULT 'user',      -- user | target (made for a target's searches)
    etag TEXT,
    last_modified TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, kind, url)
);

-- What was collected: an article, a page or a video (its subtitles).
CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    source_id TEXT REFERENCES sources(id) ON DELETE SET NULL,
    url TEXT NOT NULL,
    kind TEXT NOT NULL,                       -- article | page | video
    title TEXT NOT NULL,
    author TEXT,
    site TEXT,
    published_at TEXT,
    fetched_at TEXT NOT NULL,
    lang TEXT,
    text TEXT NOT NULL,                       -- Markdown; for a video, its subtitles
    segments TEXT,                            -- JSON [{start, end, text}] for videos
    note TEXT,                                -- why the text is incomplete (e.g. no subtitles)
    content_hash TEXT NOT NULL,
    indexed_at TEXT,
    index_error TEXT,
    UNIQUE (investigation_id, url)
);
CREATE INDEX IF NOT EXISTS idx_documents_investigation ON documents(investigation_id, fetched_at);

-- Pages and videos found by feeds and searches, waiting to be fetched.
CREATE TABLE IF NOT EXISTS crawl_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    source_id TEXT REFERENCES sources(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'article',     -- article | video
    title TEXT,                               -- hints from the feed or search result
    published_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending',   -- pending | done | failed | skipped
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL,
    error TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, url)
);
CREATE INDEX IF NOT EXISTS idx_crawl_queue_pending ON crawl_queue(status, next_attempt_at);

-- Search index: passages of each document with an optional embedding (f32 LE BLOB)
-- and a keyword (FTS5) entry.
CREATE TABLE IF NOT EXISTS rag_chunks (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    text TEXT NOT NULL,
    start_time REAL,                          -- seconds into a video
    end_time REAL,
    doc_date TEXT NOT NULL,                   -- published_at, else fetched_at
    embedding BLOB,
    embedding_model TEXT,
    dims INTEGER,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_investigation ON rag_chunks(investigation_id);
CREATE INDEX IF NOT EXISTS idx_rag_chunks_document ON rag_chunks(document_id);

CREATE VIRTUAL TABLE IF NOT EXISTS rag_chunks_fts USING fts5(
    chunk_id UNINDEXED,
    title,
    text,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Chunks also go when their document or investigation is deleted (foreign key cascade)
CREATE TRIGGER IF NOT EXISTS rag_chunks_fts_delete AFTER DELETE ON rag_chunks BEGIN
    DELETE FROM rag_chunks_fts WHERE chunk_id = old.id;
END;

-- Questions asked on the Ask page, with the answer as returned.
CREATE TABLE IF NOT EXISTS ask_history (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    answer_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Small app preferences (key → value), e.g. the browser whose YouTube login yt-dlp uses.
CREATE TABLE IF NOT EXISTS prefs (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
