//! Lines cut at short pauses. The engine still hears a whole stretch of speech
//! (short clips make Whisper invent text), and the times of its words cut the
//! text where someone paused for `LINE_PAUSE_SECONDS`: two people answering each
//! other almost at once get a line each instead of one shared line. Pieces then
//! said by the same person are joined again, so a line is a turn, not a breath.

/// A pause this long between two words starts a new piece.
pub const LINE_PAUSE_SECONDS: f64 = 0.15;

/// A word the engine heard, in seconds from the start of the clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// A part of the clip's text between pauses, in seconds from the start of the clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

fn spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `text` cut where its words pause. None when there are no words, or when they
/// do not spell `text` (the engine cleaned the text up after): the caller then
/// keeps the clip as one line.
pub fn split_at_pauses(text: &str, words: &[Word]) -> Option<Vec<Piece>> {
    let words: Vec<&Word> = words.iter().filter(|w| !w.text.trim().is_empty()).collect();
    let spelled = words.iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" ");
    // Words all at the same instant carry no times (an engine that did not compute them)
    let timed = words.first().zip(words.last()).is_some_and(|(first, last)| last.end > first.start);
    if !timed || spaced(&spelled) != spaced(text) {
        return None;
    }
    let mut pieces: Vec<Piece> = Vec::new();
    for word in words {
        match pieces.last_mut() {
            Some(piece) if word.start - piece.end < LINE_PAUSE_SECONDS => {
                piece.text.push(' ');
                piece.text.push_str(word.text.trim());
                piece.end = piece.end.max(word.end);
            }
            _ => pieces.push(Piece { text: word.text.trim().to_string(), start: word.start, end: word.end }),
        }
    }
    Some(pieces)
}

/// Stretches the first and last line to the clip's bounds (0 and `duration`), as
/// a line covered the whole clip before it was cut.
pub fn cover<T>(lines: &mut [(Piece, T)], duration: f64) {
    if let Some((first, _)) = lines.first_mut() {
        first.start = 0.0;
    }
    if let Some((last, _)) = lines.last_mut() {
        last.end = last.end.max(duration);
    }
}

/// Joins neighbouring pieces said by the same person (`who`) into one.
pub fn rejoin<T: PartialEq>(pieces: Vec<(Piece, T)>) -> Vec<(Piece, T)> {
    let mut joined: Vec<(Piece, T)> = Vec::new();
    for (piece, who) in pieces {
        match joined.last_mut() {
            Some((last, last_who)) if *last_who == who => {
                last.text.push(' ');
                last.text.push_str(&piece.text);
                last.end = piece.end;
            }
            _ => joined.push((piece, who)),
        }
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start: f64, end: f64) -> Word {
        Word { text: text.into(), start, end }
    }

    #[test]
    fn a_pause_of_150_ms_starts_a_piece_and_a_shorter_one_does_not() {
        let words = [word("Pode", 0.0, 0.3), word("ser?", 0.4, 0.7), word("Pode,", 0.85, 1.1), word("sim.", 1.2, 1.4)];
        let pieces = split_at_pauses("Pode ser? Pode, sim.", &words).unwrap();
        assert_eq!(
            pieces,
            vec![
                Piece { text: "Pode ser?".into(), start: 0.0, end: 0.7 },
                Piece { text: "Pode, sim.".into(), start: 0.85, end: 1.4 },
            ]
        );
    }

    #[test]
    fn words_that_do_not_spell_the_text_keep_it_whole() {
        assert_eq!(split_at_pauses("ok ok", &[word("ok", 0.0, 0.2)]), None);
        assert_eq!(split_at_pauses("ok", &[]), None);
        assert_eq!(split_at_pauses("sem tempo", &[word("sem", 0.0, 0.0), word("tempo", 0.0, 0.0)]), None);
    }

    #[test]
    fn the_same_person_is_joined_and_another_one_stays_apart() {
        let piece = |text: &str, start: f64, end: f64| Piece { text: text.into(), start, end };
        let lines = rejoin(vec![
            (piece("Fechou", 0.0, 0.5), "Ana"),
            (piece("o deploy?", 0.7, 1.2), "Ana"),
            (piece("Fechou.", 1.4, 1.8), "Bia"),
            (piece("Valeu.", 2.0, 2.3), "Ana"),
        ]);
        assert_eq!(
            lines,
            vec![
                (piece("Fechou o deploy?", 0.0, 1.2), "Ana"),
                (piece("Fechou.", 1.4, 1.8), "Bia"),
                (piece("Valeu.", 2.0, 2.3), "Ana"),
            ]
        );
    }
}
