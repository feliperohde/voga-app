//! The session's sound, kept as a 16 kHz mono 16-bit WAV so a check's moment can be heard
//! again. The header's sizes are rewritten every few seconds: if the app stops abruptly the
//! file still plays up to then, and `repair` fixes the rest from the file's length.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::SAMPLE_RATE;

const HEADER_BYTES: u64 = 44;

fn header(data_bytes: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

/// A whole WAV in memory (a voice's sample, a clip).
pub fn encode(samples: &[f32]) -> Vec<u8> {
    let mut wav = Vec::with_capacity(44 + samples.len() * 2);
    wav.extend_from_slice(&header((samples.len() * 2) as u32));
    for &sample in samples {
        wav.extend_from_slice(&to_i16(sample).to_le_bytes());
    }
    wav
}

/// Appends the session's sound to its file.
pub struct Writer {
    file: File,
    data_bytes: u64,
}

impl Writer {
    pub fn create(path: &Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut file = File::create(path)?;
        file.write_all(&header(0))?;
        Ok(Self { file, data_bytes: 0 })
    }

    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        let bytes: Vec<u8> = samples.iter().flat_map(|&s| to_i16(s).to_le_bytes()).collect();
        self.file.write_all(&bytes)?;
        self.data_bytes += bytes.len() as u64;
        Ok(())
    }

    /// Rewrites the header's sizes, so the file plays up to here.
    pub fn sync(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&header(self.data_bytes.min(u32::MAX as u64 - 36) as u32))?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }
}

/// Sets the header's sizes from the file's length (a session the app did not end).
pub fn repair(path: &Path) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let data_bytes = file.metadata()?.len().saturating_sub(HEADER_BYTES);
    file.write_all(&header(data_bytes.min(u32::MAX as u64 - 36) as u32))?;
    Ok(())
}

/// The sound between `start` and `end` seconds of a session file. Reads by position, so it
/// works whatever the header says.
pub fn read_range(path: &Path, start: f64, end: f64) -> io::Result<Vec<f32>> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len().saturating_sub(HEADER_BYTES);
    let at = |seconds: f64| ((seconds.max(0.0) * SAMPLE_RATE as f64) as u64 * 2).min(length);
    let (from, to) = (at(start), at(end));
    if to <= from {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(HEADER_BYTES + from))?;
    let mut bytes = vec![0u8; (to - from) as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_file_reads_back_by_time_even_before_it_is_repaired() {
        let dir = std::env::temp_dir().join(format!("voga-wav-{}", uuid::Uuid::new_v4()));
        let path = dir.join("s.wav");
        let mut writer = Writer::create(&path).unwrap();
        // One second of silence, then one of a constant level
        writer.write(&vec![0.0; SAMPLE_RATE as usize]).unwrap();
        writer.write(&vec![0.5; SAMPLE_RATE as usize]).unwrap();
        drop(writer); // never synced: the header still says 0 bytes

        let second = read_range(&path, 1.0, 2.0).unwrap();
        assert_eq!(second.len(), SAMPLE_RATE as usize);
        assert!((second[0] - 0.5).abs() < 0.001);
        assert_eq!(read_range(&path, 1.5, 9.0).unwrap().len(), SAMPLE_RATE as usize / 2);

        repair(&path).unwrap();
        let mut bytes = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), SAMPLE_RATE * 2 * 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_clip_is_a_whole_wav() {
        let wav = encode(&[0.0, 1.0, -1.0]);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes([wav[46], wav[47]]), i16::MAX);
    }
}
