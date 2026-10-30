//! YouTube through yt-dlp: a video's subtitles and metadata, and video search. Channels
//! are followed through their RSS feed (no yt-dlp needed).
//!
//! yt-dlp is downloaded on first use into the app data folder and updated weekly: YouTube
//! changes often and an old copy stops working. It is a separate program (Unlicense /
//! GPLv3 binaries) the app runs, never linked in.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

use crate::prefs;

const RUN_TIMEOUT: Duration = Duration::from_secs(180);
const UPDATE_EVERY_DAYS: i64 = 7;
/// Subtitle languages, best first: Brazilian Portuguese, Portuguese, the automatic captions
/// in the language spoken. Wider patterns (pt.*) also pull auto-translations and get HTTP 429.
const SUB_LANGS: [&str; 3] = ["pt-BR", "pt", "pt-orig"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimedText {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub struct Video {
    pub url: String,
    pub title: String,
    pub channel: Option<String>,
    /// YYYY-MM-DD.
    pub published: Option<String>,
    pub description: String,
    pub segments: Vec<TimedText>,
    /// Why there are no subtitles, when there are none.
    pub note: Option<String>,
    pub thumbnail: Option<String>,
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    }
}

/// The release asset for this platform.
fn download_url() -> &'static str {
    if cfg!(target_os = "macos") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos"
    } else if cfg!(windows) {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
    } else if cfg!(target_arch = "aarch64") {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64"
    } else {
        "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux"
    }
}

/// yt-dlp in the app data folder: downloaded when missing, updated when a week old.
async fn ytdlp(pool: &SqlitePool, app_data_dir: &Path) -> Result<PathBuf> {
    let path = app_data_dir.join("bin").join(binary_name());
    if !path.exists() {
        log::info!("YouTube: downloading yt-dlp to {}", path.display());
        let bytes = reqwest::get(download_url())
            .await
            .and_then(|r| r.error_for_status())
            .context("Could not download yt-dlp")?
            .bytes()
            .await
            .context("Could not download yt-dlp")?;
        std::fs::create_dir_all(path.parent().expect("has parent"))?;
        std::fs::write(&path, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
        prefs::set(pool, "ytdlp_updated_at", &chrono::Utc::now().to_rfc3339()).await?;
        return Ok(path);
    }
    let updated = prefs::get(pool, "ytdlp_updated_at")
        .await?
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| t.with_timezone(&chrono::Utc));
    if updated.map_or(true, |t| chrono::Utc::now() - t > chrono::Duration::days(UPDATE_EVERY_DAYS)) {
        // A failed update keeps the copy there is
        match run(&path, &["-U"]).await {
            Ok(out) => log::info!("YouTube: yt-dlp -U: {}", out.lines().last().unwrap_or_default()),
            Err(e) => log::warn!("YouTube: yt-dlp update failed: {e}"),
        }
        prefs::set(pool, "ytdlp_updated_at", &chrono::Utc::now().to_rfc3339()).await?;
    }
    Ok(path)
}

/// Runs yt-dlp; Ok(stdout) when it exits successfully.
async fn run(binary: &Path, args: &[&str]) -> Result<String> {
    let mut command = Command::new(binary);
    command.args(args).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = tokio::time::timeout(RUN_TIMEOUT, command.output())
        .await
        .map_err(|_| anyhow!("yt-dlp took longer than {} s", RUN_TIMEOUT.as_secs()))??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let error = stderr.lines().rfind(|l| l.starts_with("ERROR")).unwrap_or(stderr.trim());
        bail!("yt-dlp: {error}");
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `https://www.youtube.com/watch?v=<id>` for watch, shorts and youtu.be links.
pub fn canonical_video_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?.trim_start_matches("www.").trim_start_matches("m.");
    let id = match host {
        "youtu.be" => parsed.path().trim_start_matches('/').to_string(),
        "youtube.com" => {
            let path = parsed.path();
            if let Some(id) = path.strip_prefix("/shorts/").or_else(|| path.strip_prefix("/live/")) {
                id.to_string()
            } else {
                parsed.query_pairs().find(|(k, _)| k == "v")?.1.into_owned()
            }
        }
        _ => return None,
    };
    let id = id.split('/').next().unwrap_or_default();
    (id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .then(|| format!("https://www.youtube.com/watch?v={id}"))
}

/// The RSS feed of a channel, from its /channel/UC… URL or (for @handle and /c/ URLs) from
/// the canonical link of its page.
pub async fn channel_feed_url(url: &str) -> Result<String> {
    fn channel_id(text: &str) -> Option<&str> {
        let start = text.find("/channel/UC")? + "/channel/".len();
        let id = text.get(start..start + 24)?;
        id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_').then_some(id)
    }
    if let Some(id) = channel_id(url) {
        return Ok(format!("https://www.youtube.com/feeds/videos.xml?channel_id={id}"));
    }
    let page = super::fetch::get_body(url).await?;
    let html = String::from_utf8_lossy(&page.body);
    let canonical = html
        .find("<link rel=\"canonical\"")
        .map(|at| &html[at..html.len().min(at + 200)])
        .and_then(channel_id)
        .ok_or_else(|| anyhow!("Could not find the channel of {url}"))?;
    Ok(format!("https://www.youtube.com/feeds/videos.xml?channel_id={canonical}"))
}

/// Strips WebVTT inline tags (<00:00:01.360>, <c>, </c>).
fn strip_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_tag = false;
    for c in line.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").trim().to_string()
}

fn parse_timestamp(text: &str) -> Option<f64> {
    let mut parts: Vec<&str> = text.trim().split(':').collect();
    let seconds: f64 = parts.pop()?.replace(',', ".").parse().ok()?;
    let minutes: f64 = parts.pop().map_or(Ok(0.0), str::parse).ok()?;
    let hours: f64 = parts.pop().map_or(Ok(0.0), str::parse).ok()?;
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

/// Subtitle lines with their times. YouTube's automatic captions roll: each cue repeats the
/// previous line before adding the next, so a line equal to the last one kept is skipped.
pub fn parse_vtt(vtt: &str) -> Vec<TimedText> {
    let mut lines: Vec<TimedText> = Vec::new();
    let mut cue: Option<(f64, f64)> = None;
    for raw in vtt.lines() {
        if let Some((start, rest)) = raw.split_once("-->") {
            let end = rest.split_whitespace().next().unwrap_or_default();
            cue = parse_timestamp(start).zip(parse_timestamp(end));
            continue;
        }
        // Only an empty line ends a cue: YouTube's cues hold lines of a single space
        if raw.trim_end_matches('\r').is_empty() {
            cue = None;
            continue;
        }
        let Some((start, end)) = cue else { continue };
        let text = strip_tags(raw);
        if text.is_empty() || lines.last().is_some_and(|last| last.text == text) {
            continue;
        }
        if let Some(last) = lines.last_mut() {
            last.end = last.end.min(start).max(last.start);
        }
        lines.push(TimedText { start, end, text });
    }
    lines
}

#[derive(Deserialize)]
struct Info {
    title: String,
    channel: Option<String>,
    uploader: Option<String>,
    /// YYYYMMDD
    upload_date: Option<String>,
    description: Option<String>,
    thumbnail: Option<String>,
}

/// A video's metadata and subtitles. Without subtitles (none, or YouTube refused them),
/// the video still comes back, with `note` saying why.
pub async fn fetch_video(pool: &SqlitePool, app_data_dir: &Path, url: &str) -> Result<Video> {
    let url = canonical_video_url(url).ok_or_else(|| anyhow!("Not a YouTube video: {url}"))?;
    let binary = ytdlp(pool, app_data_dir).await?;
    let dir = app_data_dir.join("tmp").join(format!("yt-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let dir_arg = dir.to_string_lossy().into_owned();
    let langs = SUB_LANGS.join(",");
    let mut args: Vec<String> = [
        "--skip-download", "--write-info-json", "--write-subs", "--write-auto-subs", "--sub-format", "vtt",
        "--sub-langs", &langs, "--extractor-args", "youtube:player_client=default,mweb", "--ignore-errors", "--no-progress",
        "-P", &dir_arg, "-o", "video.%(ext)s",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(url.clone());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    // --ignore-errors: one subtitle failing must not lose the rest; the files tell what came
    let outcome = run(&binary, &refs).await;
    let result = read_video(&dir, &url, outcome.err());
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn read_video(dir: &Path, url: &str, run_error: Option<anyhow::Error>) -> Result<Video> {
    let info_path = dir.join("video.info.json");
    let info: Info = match std::fs::read(&info_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("Unreadable yt-dlp metadata")?,
        Err(_) => bail!("{}", run_error.map_or_else(|| "yt-dlp wrote no metadata".to_string(), |e| e.to_string())),
    };
    let subtitles = SUB_LANGS.iter().find_map(|lang| std::fs::read_to_string(dir.join(format!("video.{lang}.vtt"))).ok());
    let segments = subtitles.as_deref().map(parse_vtt).unwrap_or_default();
    let note = segments.is_empty().then(|| match &run_error {
        Some(e) if e.to_string().contains("Sign in") => "O YouTube pediu login para mostrar as legendas, e o Voga não usa login: o vídeo fica com a descrição.".to_string(),
        _ => "O vídeo não tem legendas em português.".to_string(),
    });
    let published = info.upload_date.filter(|d| d.len() == 8).map(|d| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..]));
    Ok(Video {
        url: url.to_string(),
        title: info.title,
        channel: info.channel.or(info.uploader),
        published,
        description: info.description.unwrap_or_default(),
        segments,
        note,
        thumbnail: info.thumbnail,
    })
}

#[derive(Deserialize)]
struct SearchResults {
    entries: Vec<SearchEntry>,
}

#[derive(Deserialize)]
struct SearchEntry {
    id: String,
    title: Option<String>,
}

/// The first `count` videos YouTube finds for `query`.
pub async fn search(pool: &SqlitePool, app_data_dir: &Path, query: &str, count: usize) -> Result<Vec<super::feeds::Entry>> {
    let binary = ytdlp(pool, app_data_dir).await?;
    let target = format!("ytsearch{count}:{query}");
    let json = run(&binary, &["--flat-playlist", "-J", "--no-progress", &target]).await?;
    let results: SearchResults = serde_json::from_str(&json).context("Unreadable yt-dlp search results")?;
    Ok(results
        .entries
        .into_iter()
        .map(|e| super::feeds::Entry { url: format!("https://www.youtube.com/watch?v={}", e.id), title: e.title, published: None, summary: None, image: None })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start of a real automatic caption file (the lines of one space are YouTube's).
    const AUTO_CAPTIONS: &str = concat!(
        "WEBVTT\nKind: captions\nLanguage: pt\n\n",
        "00:00:01.040 --> 00:00:03.150 align:start position:0%\n \n",
        "Senhor<00:00:01.360><c> presidente,</c><00:00:02.120><c> senhoras</c><00:00:02.680><c> e</c><00:00:02.720><c> senhores</c>\n\n",
        "00:00:03.150 --> 00:00:03.160 align:start position:0%\nSenhor presidente, senhoras e senhores\n \n\n",
        "00:00:03.160 --> 00:00:05.630 align:start position:0%\nSenhor presidente, senhoras e senhores\n",
        "deputados.<00:00:04.360><c> Quero</c><00:00:04.600><c> fazer</c><00:00:05.000><c> uso</c><00:00:05.240><c> do</c><00:00:05.360><c> tempo</c>\n\n",
        "00:00:05.630 --> 00:00:05.640 align:start position:0%\ndeputados. Quero fazer uso do tempo\n \n\n",
        "00:00:05.640 --> 00:00:07.670 align:start position:0%\ndeputados. Quero fazer uso do tempo\n",
        "destinado<00:00:06.240><c> à</c><00:00:06.320><c> coordenadoria</c><00:00:07.120><c> dos</c><00:00:07.280><c> direitos</c>\n",
    );

    #[test]
    fn rolling_auto_captions_become_one_line_each() {
        let lines = parse_vtt(AUTO_CAPTIONS);
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            texts,
            ["Senhor presidente, senhoras e senhores", "deputados. Quero fazer uso do tempo", "destinado à coordenadoria dos direitos"]
        );
        assert_eq!(lines[0].start, 1.04);
        assert_eq!(lines[1].start, 3.16);
        // Each line ends where the next begins
        assert_eq!(lines[0].end, 3.15);
    }

    #[test]
    fn plain_subtitles_and_long_timestamps() {
        let lines = parse_vtt("WEBVTT\n\n01:02:03.500 --> 01:02:05.000\n<v Ana>Bom dia &amp; boa tarde\n\n01:02:05.000 --> 01:02:06.000\nSegunda linha\n");
        assert_eq!(lines[0], TimedText { start: 3723.5, end: 3725.0, text: "Bom dia & boa tarde".into() });
        assert_eq!(lines[1].text, "Segunda linha");
    }

    #[test]
    fn video_urls_are_canonical() {
        let watch = Some("https://www.youtube.com/watch?v=fFbQi8zCXyY".to_string());
        assert_eq!(canonical_video_url("https://youtu.be/fFbQi8zCXyY?t=10"), watch);
        assert_eq!(canonical_video_url("https://www.youtube.com/shorts/fFbQi8zCXyY"), watch);
        assert_eq!(canonical_video_url("https://m.youtube.com/watch?v=fFbQi8zCXyY&list=x"), watch);
        assert_eq!(canonical_video_url("https://www.youtube.com/@canal"), None);
        assert_eq!(canonical_video_url("https://vimeo.com/123"), None);
    }

    #[test]
    fn subtitles_missing_is_a_note_not_an_error() {
        let dir = std::env::temp_dir().join(format!("voga-yt-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("video.info.json"), r#"{"title":"Discurso","channel":"Canal","upload_date":"20190529","description":"d"}"#).unwrap();
        let video = read_video(&dir, "https://www.youtube.com/watch?v=fFbQi8zCXyY", Some(anyhow!("yt-dlp: ERROR: Sign in to confirm"))).unwrap();
        assert_eq!(video.published.as_deref(), Some("2019-05-29"));
        assert!(video.segments.is_empty());
        assert!(video.note.unwrap().contains("login"));

        std::fs::write(dir.join("video.pt-orig.vtt"), AUTO_CAPTIONS).unwrap();
        let video = read_video(&dir, "https://www.youtube.com/watch?v=fFbQi8zCXyY", None).unwrap();
        assert_eq!(video.segments.len(), 3);
        assert_eq!(video.note, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
