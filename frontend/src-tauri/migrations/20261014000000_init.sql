-- Voga 0.1.0: settings and the trial/license state.

-- Settings › AI (single row). api_keys: JSON {provider: key}, never sent to the UI.
CREATE TABLE IF NOT EXISTS ai_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    provider TEXT NOT NULL DEFAULT 'ollama',
    model TEXT NOT NULL DEFAULT '',
    ollama_endpoint TEXT,
    custom_endpoint TEXT,
    embedding_model TEXT NOT NULL DEFAULT 'bge-m3',
    api_keys TEXT NOT NULL DEFAULT '{}'
);
INSERT OR IGNORE INTO ai_settings (id) VALUES (1);

-- Keys entered by the user (verified with the app's public key on every read).
CREATE TABLE IF NOT EXISTS entitlement_keys (
    id TEXT PRIMARY KEY NOT NULL,       -- the key's own id
    features TEXT NOT NULL,             -- JSON array of feature ids
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    install_id TEXT,                    -- the installation it was issued for, if any
    key TEXT NOT NULL,
    activated_at TEXT NOT NULL
);

-- This installation's id, when it first ran (start of the trial) and the latest time
-- the app has seen, so turning the clock back does not reopen features.
CREATE TABLE IF NOT EXISTS entitlement_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    install_id TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    installed_at TEXT NOT NULL
);
