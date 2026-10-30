-- Investigations of any interest (a disease's treatment, a technology, a company), not only
-- politics: they collect and index the same way, without official data or claim analysis.
ALTER TABLE investigations ADD COLUMN kind TEXT NOT NULL DEFAULT 'politics'; -- politics | interest

-- The Feed: what was read, starred and liked or disliked (-1 | 0 | 1), and how close each
-- document is to its investigation's interest (NULL: to be scored, feed::score_step).
ALTER TABLE documents ADD COLUMN read_at TEXT;
ALTER TABLE documents ADD COLUMN starred INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN feedback INTEGER NOT NULL DEFAULT 0;
ALTER TABLE documents ADD COLUMN relevance REAL;
CREATE INDEX documents_unscored ON documents(investigation_id) WHERE relevance IS NULL;
