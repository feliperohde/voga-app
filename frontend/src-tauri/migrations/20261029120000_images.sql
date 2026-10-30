-- The picture a news item, page, video or post came with (og:image, the feed's thumbnail, the
-- video's), and the copy kept in <app data>/thumbs (prefs news_images), fetched once.
ALTER TABLE documents ADD COLUMN image_url TEXT;
ALTER TABLE documents ADD COLUMN image_path TEXT;
ALTER TABLE documents ADD COLUMN image_tries INTEGER NOT NULL DEFAULT 0;
CREATE INDEX documents_image_pending ON documents(fetched_at) WHERE image_url IS NOT NULL AND image_path IS NULL;
