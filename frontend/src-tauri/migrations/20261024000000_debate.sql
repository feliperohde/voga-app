-- The Debate view of live sessions: what each person affirms or denies, the interviewer's voice,
-- and checks made only when a person asks for them.

-- affirmation | denial; query: what the records are searched for (to check it later on demand)
ALTER TABLE live_checks ADD COLUMN kind TEXT NOT NULL DEFAULT 'affirmation';
ALTER TABLE live_checks ADD COLUMN query TEXT NOT NULL DEFAULT '';
-- 'interviewer': the voice asks the questions (never checked)
ALTER TABLE live_voices ADD COLUMN role TEXT;
-- 0: claims are only detected (verdict 'detected') until a person asks to check one
ALTER TABLE live_sessions ADD COLUMN auto_check INTEGER NOT NULL DEFAULT 1;
