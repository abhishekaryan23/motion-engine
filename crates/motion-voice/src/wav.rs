//! Minimal mono s16 WAV reader/writer (no external dependency).

use crate::{VoiceError, SAMPLE_RATE};

/// Encode mono s16 samples at [`SAMPLE_RATE`] as a canonical 44-byte-header WAV.
pub fn encode(samples: &[i16]) -> Vec<u8> {
    encode_at(samples, SAMPLE_RATE)
}

/// Encode mono s16 samples at an explicit sample rate.
pub fn encode_at(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Decoded mono s16 WAV.
#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}

impl Wav {
    pub fn duration(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

/// Decode a mono 16-bit PCM WAV (any rate). Other layouts are an error.
pub fn decode(bytes: &[u8]) -> Result<Wav, VoiceError> {
    let bad = |m: &str| VoiceError::Audio(format!("wav: {m}"));
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let mut pos = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = le32(bytes, pos + 4).ok_or_else(|| bad("truncated chunk"))? as usize;
        let body = pos + 8;
        if id == b"fmt " {
            let tag = le16(bytes, body).ok_or_else(|| bad("short fmt"))?;
            let ch = le16(bytes, body + 2).ok_or_else(|| bad("short fmt"))?;
            let rate = le32(bytes, body + 4).ok_or_else(|| bad("short fmt"))?;
            let bits = le16(bytes, body + 14).ok_or_else(|| bad("short fmt"))?;
            fmt = Some((tag, ch, rate, bits));
        } else if id == b"data" {
            let (tag, ch, rate, bits) = fmt.ok_or_else(|| bad("data before fmt"))?;
            if tag != 1 || ch != 1 || bits != 16 {
                return Err(bad("expected mono 16-bit PCM"));
            }
            let end = body.saturating_add(size).min(bytes.len());
            let samples = bytes[body..end]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect();
            return Ok(Wav {
                sample_rate: rate,
                samples,
            });
        }
        pos = body.saturating_add(size).saturating_add(size & 1);
    }
    Err(bad("no data chunk"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s: Vec<i16> = (0..1000).map(|i| ((i * 37) % 2000 - 1000) as i16).collect();
        let w = decode(&encode(&s)).unwrap();
        assert_eq!(w.sample_rate, SAMPLE_RATE);
        assert_eq!(w.samples, s);
    }

    #[test]
    fn rejects_garbage() {
        assert!(decode(b"nope").is_err());
    }
}
