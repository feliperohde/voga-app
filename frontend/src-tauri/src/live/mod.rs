//! Live sessions (Ao vivo): a candidate heard as they speak, on this computer. The sound of a
//! microphone and/or of the computer (audio) is cut into stretches of speech (vad), written
//! down by Whisper (whisper) in lines cut at pauses (pauses), each with the voice that said
//! it (voices); what the people followed say is fact-checked against the records (check).
//! Sessions, lines, voices and checks are saved as they come (session), and a session that
//! ends becomes a `live` document of the investigation. Much of it comes from Assunta's live
//! recording (capture, VAD, Whisper, live speakers), copied and trimmed.

pub mod audio;
pub mod check;
pub mod commands;
pub mod pauses;
pub mod remote;
pub mod session;
pub mod vad;
pub mod voices;
pub mod wav;
pub mod whisper;

/// The rate everything here works at (Whisper's, Silero's and the voice model's).
pub const SAMPLE_RATE: u32 = 16_000;
