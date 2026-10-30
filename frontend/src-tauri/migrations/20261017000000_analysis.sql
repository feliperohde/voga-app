-- Analysis: promises and statements read from what was collected, and what the AI finds
-- about them in other records (CONTRADICTS, FULFILLS), suggested until a person reviews it.

ALTER TABLE documents ADD COLUMN analyzed_at TEXT;
ALTER TABLE documents ADD COLUMN analysis_error TEXT;

-- Why the AI suggested a relation, in one sentence.
ALTER TABLE relations ADD COLUMN rationale TEXT;
CREATE INDEX IF NOT EXISTS idx_relations_status ON relations(investigation_id, status);
CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(investigation_id, type);
