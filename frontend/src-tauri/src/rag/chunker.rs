//! Splits documents into retrieval chunks.
//!
//! Timed text (video subtitles) is chunked in time windows of consecutive segments
//! (with a one-segment overlap) so each chunk keeps enough surrounding speech to be
//! understood on its own, and carries start/end times for citations. Articles and
//! pages are split on Markdown structure.

/// One timed piece of text (a subtitle line).
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub speaker: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkDraft {
    pub text: String,
    pub start_time: Option<f64>,
    pub end_time: Option<f64>,
    pub speakers: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ChunkOptions {
    /// Target window length in seconds for timed transcripts.
    pub target_seconds: f64,
    /// Hard cap on chunk size in characters (also the only limit for untimed text).
    pub max_chars: usize,
    /// Segments repeated at the start of the next chunk.
    pub overlap_segments: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            target_seconds: 90.0,
            max_chars: 1500,
            overlap_segments: 1,
        }
    }
}

pub fn chunk_transcript(segments: &[Segment], opts: ChunkOptions) -> Vec<ChunkDraft> {
    let segments: Vec<&Segment> = segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < segments.len() {
        let mut end = start; // exclusive
        let mut chars = 0;
        while end < segments.len() {
            let seg_chars = segments[end].text.trim().chars().count() + 1;
            let window_full = end > start
                && (chars + seg_chars > opts.max_chars
                    || window_seconds(&segments[start..end]) >= opts.target_seconds);
            if window_full {
                break;
            }
            chars += seg_chars;
            end += 1;
        }

        let window = &segments[start..end];
        chunks.push(build_chunk(window));

        if end >= segments.len() {
            break;
        }
        // Overlap, but always make progress
        start = end.saturating_sub(opts.overlap_segments).max(start + 1);
    }
    chunks
}

fn window_seconds(window: &[&Segment]) -> f64 {
    let first = window.iter().find_map(|s| s.start_time);
    let last = window.iter().rev().find_map(|s| s.end_time.or(s.start_time));
    match (first, last) {
        (Some(a), Some(b)) if b >= a => b - a,
        _ => 0.0,
    }
}

/// Joins segment texts; when speakers are known, each change of speaker is marked
/// inline ("Ana: …") so both keyword search and the answer model see who said what.
fn build_chunk(window: &[&Segment]) -> ChunkDraft {
    let mut text = String::new();
    let mut current: Option<&str> = None;
    for segment in window {
        if !text.is_empty() {
            text.push(' ');
        }
        if let Some(speaker) = segment.speaker.as_deref() {
            if current != Some(speaker) {
                text.push_str(speaker);
                text.push_str(": ");
                current = Some(speaker);
            }
        }
        text.push_str(segment.text.trim());
    }
    let mut speakers: Vec<String> = Vec::new();
    for speaker in window.iter().filter_map(|s| s.speaker.as_ref()) {
        if !speakers.contains(speaker) {
            speakers.push(speaker.clone());
        }
    }
    ChunkDraft {
        text,
        start_time: window.iter().find_map(|s| s.start_time),
        end_time: window.iter().rev().find_map(|s| s.end_time.or(s.start_time)),
        speakers,
    }
}

/// Splits markdown into chunks of at most `max_chars`, preferring to break at
/// headings, then blank lines, then line/sentence boundaries. A heading stays
/// attached to the content that follows it.
pub fn chunk_markdown(markdown: &str, max_chars: usize) -> Vec<String> {
    // Group into sections that start at headings
    let mut sections: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with('#') && !current.trim().is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        sections.push(current);
    }

    let mut chunks = Vec::new();
    let mut buffer = String::new();
    for section in sections {
        for piece in split_to_fit(section.trim(), max_chars) {
            if !buffer.is_empty() && buffer.chars().count() + piece.chars().count() + 2 > max_chars {
                chunks.push(std::mem::take(&mut buffer));
            }
            if !buffer.is_empty() {
                buffer.push_str("\n\n");
            }
            buffer.push_str(&piece);
        }
    }
    if !buffer.trim().is_empty() {
        chunks.push(buffer);
    }
    chunks
}

/// Breaks text that exceeds `max_chars` at paragraph, line, sentence, then word boundaries.
fn split_to_fit(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    for separator in ["\n\n", "\n", ". ", " "] {
        let parts: Vec<&str> = text.split(separator).filter(|p| !p.trim().is_empty()).collect();
        if parts.len() > 1 {
            let mut out = Vec::new();
            let mut buffer = String::new();
            for part in parts {
                let candidate_len = buffer.chars().count() + separator.len() + part.chars().count();
                if !buffer.is_empty() && candidate_len > max_chars {
                    out.push(std::mem::take(&mut buffer));
                }
                if !buffer.is_empty() {
                    buffer.push_str(separator);
                }
                buffer.push_str(part);
            }
            if !buffer.is_empty() {
                out.push(buffer);
            }
            return out.into_iter().flat_map(|p| split_to_fit(&p, max_chars)).collect();
        }
    }
    // A single unbreakable token: hard split by characters
    text.chars()
        .collect::<Vec<_>>()
        .chunks(max_chars)
        .map(|c| c.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, start: f64, end: f64) -> Segment {
        Segment {
            text: text.to_string(),
            start_time: Some(start),
            end_time: Some(end),
            speaker: None,
        }
    }

    #[test]
    fn transcript_windows_follow_time_and_overlap() {
        // 10 segments of 20s each = 200s
        let segments: Vec<Segment> = (0..10)
            .map(|i| seg(&format!("fala {i}"), i as f64 * 20.0, i as f64 * 20.0 + 20.0))
            .collect();
        let chunks = chunk_transcript(&segments, ChunkOptions::default());

        assert!(chunks.len() >= 2);
        // Every window stays near the target (one segment of slack)
        for chunk in &chunks {
            let span = chunk.end_time.unwrap() - chunk.start_time.unwrap();
            assert!(span <= 90.0 + 20.0, "span {span}");
        }
        // Consecutive chunks overlap by one segment
        assert_eq!(chunks[0].end_time, Some(chunks[1].start_time.unwrap() + 20.0));
        // All content is covered
        assert!(chunks.last().unwrap().text.contains("fala 9"));
        assert!(chunks[0].text.starts_with("fala 0"));
    }

    #[test]
    fn untimed_segments_split_by_size() {
        let segments: Vec<Segment> = (0..50)
            .map(|i| Segment {
                text: format!("segmento numero {i} com algum texto"),
                start_time: None,
                end_time: None,
                speaker: None,
            })
            .collect();
        let opts = ChunkOptions { max_chars: 200, ..ChunkOptions::default() };
        let chunks = chunk_transcript(&segments, opts);
        assert!(chunks.len() > 5);
        assert!(chunks.iter().all(|c| c.text.chars().count() <= 200));
        assert!(chunks.iter().all(|c| c.start_time.is_none()));
    }

    #[test]
    fn empty_and_blank_segments_are_skipped() {
        assert!(chunk_transcript(&[], ChunkOptions::default()).is_empty());
        let chunks = chunk_transcript(&[seg("  ", 0.0, 1.0), seg("oi", 1.0, 2.0)], ChunkOptions::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "oi");
        assert_eq!(chunks[0].start_time, Some(1.0));
    }

    #[test]
    fn speakers_are_deduplicated_in_order() {
        let mut a = seg("a", 0.0, 1.0);
        a.speaker = Some("Ana".into());
        let mut b = seg("b", 1.0, 2.0);
        b.speaker = Some("Bruno".into());
        let mut c = seg("c", 2.0, 3.0);
        c.speaker = Some("Ana".into());
        let mut d = seg("d", 3.0, 4.0);
        d.speaker = Some("Ana".into());
        let chunks = chunk_transcript(&[a, b, c, d], ChunkOptions::default());
        assert_eq!(chunks[0].speakers, vec!["Ana".to_string(), "Bruno".to_string()]);
        assert_eq!(chunks[0].text, "Ana: a Bruno: b Ana: c d");
    }

    #[test]
    fn markdown_keeps_headings_with_content_and_respects_size() {
        let md = "# Resumo\nReunião sobre deploy.\n\n## Decisões\n- Adiar release\n\n## Ações\n- Ana abre ticket ABC-123\n";
        let chunks = chunk_markdown(md, 1000);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("## Decisões\n- Adiar release"));

        let long = format!("# A\n{}\n\n# B\n{}", "palavra ".repeat(300), "outra ".repeat(300));
        let chunks = chunk_markdown(&long, 500);
        assert!(chunks.len() >= 4);
        assert!(chunks.iter().all(|c| c.chars().count() <= 500));
        assert!(chunks[0].starts_with("# A"));
    }
}
