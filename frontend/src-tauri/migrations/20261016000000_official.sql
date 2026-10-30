-- Official sources (Câmara, Senado, TSE): the facts they state become entities and relations,
-- the nodes and edges of the investigation's graph.

-- Who and what the facts are about. `key` is stable across runs (e.g. 'target:<id>' for the
-- person a target is, 'party:PSB', 'camara:prop:2647228', 'cnpj:12345678000190').
CREATE TABLE IF NOT EXISTS entities (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    type TEXT NOT NULL,          -- person | party | organization | bill | vote_session | candidacy
    key TEXT NOT NULL,
    label TEXT NOT NULL,
    attrs TEXT NOT NULL DEFAULT '{}',
    document_id TEXT REFERENCES documents(id) ON DELETE SET NULL,  -- where it is described
    origin TEXT NOT NULL DEFAULT 'structured',                      -- structured | llm | user
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, key)
);

-- What connects them: MEMBER_OF, AUTHORED, VOTED {vote}, SPENT_WITH {amount}, CANDIDATE_IN.
-- Facts from official data are 'confirmed'; what an AI infers will start as 'suggested'.
CREATE TABLE IF NOT EXISTS relations (
    id TEXT PRIMARY KEY NOT NULL,
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    from_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    to_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    props TEXT NOT NULL DEFAULT '{}',
    date TEXT NOT NULL DEFAULT '',  -- '' when timeless; part of the identity (a monthly expense)
    document_id TEXT REFERENCES documents(id) ON DELETE SET NULL,
    origin TEXT NOT NULL DEFAULT 'structured',
    status TEXT NOT NULL DEFAULT 'confirmed',  -- confirmed | suggested | rejected
    confidence REAL NOT NULL DEFAULT 1.0,
    created_at TEXT NOT NULL,
    UNIQUE (investigation_id, from_id, to_id, label, date)
);
CREATE INDEX IF NOT EXISTS idx_relations_from ON relations(from_id);
CREATE INDEX IF NOT EXISTS idx_relations_to ON relations(to_id);

-- Open-data dumps kept on disk (votes of a year, TSE candidates), refreshed with conditional GETs.
CREATE TABLE IF NOT EXISTS bulk_files (
    url TEXT PRIMARY KEY NOT NULL,
    path TEXT NOT NULL,
    etag TEXT,
    last_modified TEXT,
    checked_at TEXT NOT NULL
);

-- Where each official source stopped (JSON per stream, e.g. {"speeches": "2026-07-07"}), so a
-- long history is collected over several runs.
ALTER TABLE sources ADD COLUMN cursor TEXT NOT NULL DEFAULT '{}';
