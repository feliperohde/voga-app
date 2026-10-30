//! The pictures news items, pages, videos and posts came with, kept on this computer when the
//! user wants them (prefs `news_images`): each fetched once, politely (fetch.rs: robots.txt, ≥2 s
//! per host), while the worker is idle and within its investigation's schedule.

use std::path::{Path, PathBuf};

use anyhow::Result;
use sqlx::SqlitePool;

use super::{fetch, schedule};
use crate::prefs;

/// url | image | both: how pictures are shown.
pub const PREF: &str = "news_images";
/// Pictures fetched per worker step.
const PER_STEP: i64 = 10;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const TRIES: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Only the address: the app loads the picture from its site.
    Url,
    /// Only the copy kept here.
    Image,
    /// The copy kept here, else the site's.
    Both,
}

impl Mode {
    pub fn parse(value: &str) -> Option<Mode> {
        match value {
            "url" => Some(Mode::Url),
            "image" => Some(Mode::Image),
            "both" => Some(Mode::Both),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Url => "url",
            Mode::Image => "image",
            Mode::Both => "both",
        }
    }

    pub fn keeps(self) -> bool {
        self != Mode::Url
    }

    pub fn shows_remote(self) -> bool {
        self != Mode::Image
    }
}

pub async fn mode(pool: &SqlitePool) -> Mode {
    prefs::get(pool, PREF).await.ok().flatten().and_then(|v| Mode::parse(&v)).unwrap_or(Mode::Both)
}

pub fn dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("thumbs")
}

fn extension(content_type: &str) -> Option<&'static str> {
    match content_type.split(';').next().unwrap_or_default().trim() {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/avif" => Some("avif"),
        _ => None,
    }
}

/// Fetches pictures not kept yet (newest documents first). Returns how many were kept.
pub async fn step(pool: &SqlitePool, app_data_dir: &Path) -> Result<usize> {
    if !mode(pool).await.keeps() {
        return Ok(0);
    }
    let gate = schedule::gate(pool).await?;
    let pending: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, image_url FROM documents WHERE image_url IS NOT NULL AND image_path IS NULL AND image_tries < ?2
           AND investigation_id NOT IN (SELECT value FROM json_each(?1))
         ORDER BY fetched_at DESC LIMIT ?3",
    )
    .bind(&gate.closed)
    .bind(TRIES)
    .bind(PER_STEP)
    .fetch_all(pool)
    .await?;
    let mut kept = 0;
    for (id, url) in pending {
        match keep(app_data_dir, &id, &url).await {
            Ok(path) => {
                sqlx::query("UPDATE documents SET image_path = ? WHERE id = ?").bind(path.to_string_lossy()).bind(&id).execute(pool).await?;
                kept += 1;
            }
            Err(e) => {
                log::info!("Thumbs: {url} not kept: {e}");
                sqlx::query("UPDATE documents SET image_tries = image_tries + 1 WHERE id = ?").bind(&id).execute(pool).await?;
            }
        }
    }
    Ok(kept)
}

async fn keep(app_data_dir: &Path, document_id: &str, url: &str) -> Result<PathBuf> {
    let (content_type, bytes) = fetch::get_image(url, MAX_BYTES).await?;
    let ext = extension(&content_type).ok_or_else(|| anyhow::anyhow!("unsupported picture type {content_type}"))?;
    let dir = dir(app_data_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{document_id}.{ext}"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Deletes the pictures kept for documents about to be deleted (`where_clause` picks them from
/// `documents`, with `bind` as its only parameter).
pub async fn forget(pool: &SqlitePool, where_clause: &str, bind: &str) {
    let paths: Vec<(String,)> = sqlx::query_as(&format!("SELECT image_path FROM documents WHERE image_path IS NOT NULL AND {where_clause}"))
        .bind(bind)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    for (path,) in paths {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_picture_types() {
        assert_eq!(Mode::parse("image"), Some(Mode::Image));
        assert!(Mode::Both.keeps() && Mode::Both.shows_remote());
        assert!(!Mode::Url.keeps() && !Mode::Image.shows_remote());
        assert_eq!(extension("image/jpeg; charset=binary"), Some("jpg"));
        // An SVG can carry scripts: not kept
        assert_eq!(extension("image/svg+xml"), None);
    }
}
