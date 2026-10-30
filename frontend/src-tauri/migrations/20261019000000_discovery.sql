-- Discovery: links followed from the sources, web search, sources the AI suggests, daily limits.

-- The pages and articles a source brings have their links followed (same site, up to the
-- investigation's depth)
ALTER TABLE sources ADD COLUMN follow_links INTEGER NOT NULL DEFAULT 0;
UPDATE sources SET follow_links = 1 WHERE origin = 'user' AND kind IN ('page', 'rss');
-- Why the AI suggested it (origin 'llm'). Sources gain the statuses 'suggested' (waiting for
-- a person to approve it) and 'rejected' (dismissed; kept so it is never suggested again).
ALTER TABLE sources ADD COLUMN note TEXT;

-- How a queued item was found (feed | search | link) and how many links away from its source
ALTER TABLE crawl_queue ADD COLUMN via TEXT NOT NULL DEFAULT 'feed';
ALTER TABLE crawl_queue ADD COLUMN depth INTEGER NOT NULL DEFAULT 0;
UPDATE crawl_queue SET via = 'search'
WHERE source_id IN (SELECT id FROM sources WHERE kind IN ('news_search', 'youtube_search'));

-- Discovery limits of each investigation: JSON {pagesPerDay, aiCallsPerDay, depth, autoApprove}
ALTER TABLE investigations ADD COLUMN discovery TEXT NOT NULL DEFAULT '{}';

-- What discovery spent on each day (local date): the limits are per day
CREATE TABLE IF NOT EXISTS discovery_usage (
    investigation_id TEXT NOT NULL REFERENCES investigations(id) ON DELETE CASCADE,
    day TEXT NOT NULL,
    pages INTEGER NOT NULL DEFAULT 0,
    ai_calls INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (investigation_id, day)
);
