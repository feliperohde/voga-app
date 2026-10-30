-- Questions asked about several investigations together: the set of their ids (a sorted JSON
-- array); NULL for a question about investigation_id alone.
ALTER TABLE ask_history ADD COLUMN scope TEXT;
