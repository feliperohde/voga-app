-- When each investigation collects (collect::schedule): paused or not, each group of sources'
-- pace, and the local hours and days it may go online. '{}' collects as each source says, any time.
ALTER TABLE investigations ADD COLUMN schedule TEXT NOT NULL DEFAULT '{}';
