//! Speech detection (Silero VAD, from Assunta's audio/vad.rs): the 16 kHz sound is cut into
//! stretches of speech, so Whisper only hears speech and never long silences or music alone.
//!
//! A debate has long turns without a pause: a stretch longer than MAX_SECONDS is cut at its
//! quietest moment of the last CUT_WINDOW_SECONDS, and the rest goes on as the next stretch,
//! so the transcript keeps up with the speaker.

use anyhow::{anyhow, Result};
use silero::{VadConfig, VadSession, VadTransition};
use std::time::Duration;

use super::SAMPLE_RATE;

/// Longest stretch handed to Whisper.
const MAX_SECONDS: f64 = 15.0;
/// Where a long stretch may be cut: the quietest 30 ms of its last seconds.
const CUT_WINDOW_SECONDS: f64 = 2.0;
const FRAME: usize = SAMPLE_RATE as usize * 30 / 1000;

/// Speech between `start` and `end`, in seconds from the session start.
#[derive(Debug, Clone)]
pub struct Speech {
    pub samples: Vec<f32>,
    pub start: f64,
    pub end: f64,
}

pub struct Vad {
    session: VadSession,
    /// Where the current speech starts, in samples from the session start.
    speech_start: Option<usize>,
    /// Samples of the current speech already handed out (a long one is cut).
    taken: usize,
}

fn seconds(samples: usize) -> f64 {
    samples as f64 / SAMPLE_RATE as f64
}

/// The quietest frame boundary in `samples[from..to]`, as an index into `samples`.
fn quietest_cut(samples: &[f32], from: usize, to: usize) -> usize {
    let to = to.min(samples.len());
    let mut best = (f32::MAX, to);
    let mut at = from;
    while at + FRAME <= to {
        let energy: f32 = samples[at..at + FRAME].iter().map(|s| s * s).sum();
        if energy < best.0 {
            best = (energy, at + FRAME / 2);
        }
        at += FRAME;
    }
    best.1
}

impl Vad {
    pub fn new() -> Result<Self> {
        // Silero's thresholds; a stretch ends after half a second of quiet (a line shows up soon),
        // keeps a little sound on both sides (Whisper hears the first and last syllables whole),
        // and a click or a cough under a quarter of a second is not speech
        let config = VadConfig {
            sample_rate: SAMPLE_RATE as usize,
            redemption_time: Duration::from_millis(500),
            pre_speech_pad: Duration::from_millis(300),
            post_speech_pad: Duration::from_millis(400),
            min_speech_time: Duration::from_millis(250),
            ..VadConfig::default()
        };
        let session = VadSession::new(config).map_err(|e| anyhow!("Failed to start speech detection: {e:?}"))?;
        Ok(Self { session, speech_start: None, taken: 0 })
    }

    /// Feeds 16 kHz sound; returns the stretches of speech it completed.
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<Speech>> {
        let mut done = Vec::new();
        for transition in self.session.process(samples).map_err(|e| anyhow!("Speech detection failed: {e}"))? {
            match transition {
                VadTransition::SpeechStart { timestamp_ms } => {
                    self.speech_start = Some(timestamp_ms * SAMPLE_RATE as usize / 1000);
                    self.taken = 0;
                }
                VadTransition::SpeechEnd { start_timestamp_ms, end_timestamp_ms, samples } => {
                    let start = start_timestamp_ms * SAMPLE_RATE as usize / 1000 + self.taken;
                    let rest = samples.get(self.taken..).unwrap_or_default();
                    if !rest.is_empty() {
                        done.push(Speech {
                            samples: rest.to_vec(),
                            start: seconds(start),
                            end: (end_timestamp_ms as f64 / 1000.0).max(seconds(start + rest.len())),
                        });
                    }
                    self.speech_start = None;
                    self.taken = 0;
                }
            }
        }
        if let Some(cut) = self.long_speech_cut() {
            done.push(cut);
        }
        Ok(done)
    }

    /// The first MAX_SECONDS of a speech that goes on, cut where it is quietest.
    fn long_speech_cut(&mut self) -> Option<Speech> {
        let start = self.speech_start?;
        let current = self.session.get_current_speech();
        let max = (MAX_SECONDS * SAMPLE_RATE as f64) as usize;
        if current.len() < self.taken + max {
            return None;
        }
        let window = (CUT_WINDOW_SECONDS * SAMPLE_RATE as f64) as usize;
        let cut = quietest_cut(current, self.taken + max - window, self.taken + max);
        let piece = Speech {
            samples: current[self.taken..cut].to_vec(),
            start: seconds(start + self.taken),
            end: seconds(start + cut),
        };
        self.taken = cut;
        Some(piece)
    }

    /// The speech still open when the session ends.
    pub fn flush(&mut self) -> Option<Speech> {
        let start = self.speech_start.take()?;
        let current = self.session.get_current_speech();
        let rest = current.get(self.taken..).filter(|rest| !rest.is_empty())?.to_vec();
        let from = start + self.taken;
        self.taken = 0;
        Some(Speech { end: seconds(from + rest.len()), start: seconds(from), samples: rest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A voice-like tone (Silero hears speech-band energy) or silence.
    fn sound(seconds: f64, voiced: bool) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE as f64) as usize;
        (0..n)
            .map(|i| {
                if !voiced {
                    return 0.0;
                }
                let t = i as f32 / SAMPLE_RATE as f32;
                // A 150 Hz voice with harmonics, its loudness changing like syllables
                let syllables = 0.6 + 0.4 * (2.0 * std::f32::consts::PI * 4.0 * t).sin().abs();
                syllables * (0.3 * (2.0 * std::f32::consts::PI * 150.0 * t).sin() + 0.15 * (2.0 * std::f32::consts::PI * 450.0 * t).sin())
            })
            .collect()
    }

    #[test]
    fn a_long_stretch_is_cut_at_its_quietest_moment() {
        let mut samples = sound(1.0, true);
        // Quiet 40 ms around 14 s: the cut lands there
        samples.extend(sound(13.0, true));
        let quiet_at = samples.len() + SAMPLE_RATE as usize * 3 / 10;
        samples.extend(sound(3.0, true));
        for s in &mut samples[quiet_at..quiet_at + FRAME * 2] {
            *s = 0.0;
        }
        let cut = quietest_cut(&samples, (13.0 * SAMPLE_RATE as f64) as usize, (15.0 * SAMPLE_RATE as f64) as usize);
        assert!(cut >= quiet_at && cut <= quiet_at + FRAME * 2, "cut at {cut}, quiet at {quiet_at}");
    }

    /// Needs Silero's model (embedded) and the ONNX Runtime: `cargo test --lib live::vad -- --ignored`.
    #[test]
    #[ignore]
    fn speech_comes_out_in_stretches_no_longer_than_the_maximum() {
        let mut vad = Vad::new().unwrap();
        let mut stretches = Vec::new();
        let mut audio = sound(1.0, false);
        audio.extend(sound(40.0, true));
        audio.extend(sound(2.0, false));
        for block in audio.chunks(1600) {
            stretches.extend(vad.push(block).unwrap());
        }
        stretches.extend(vad.flush());
        assert!(stretches.len() >= 3, "{} stretches", stretches.len());
        for s in &stretches {
            assert!(s.end - s.start <= MAX_SECONDS + 0.5, "{:.1}-{:.1}", s.start, s.end);
        }
        // They follow each other without gaps or overlaps inside the speech
        for pair in stretches.windows(2) {
            assert!((pair[1].start - pair[0].end).abs() < 0.05, "{:.2} then {:.2}", pair[0].end, pair[1].start);
        }
    }
}
