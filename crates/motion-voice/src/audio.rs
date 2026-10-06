//! ffmpeg-based normalisation of provider audio to 48 kHz mono s16 WAV.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{wav, VoiceError, SAMPLE_RATE};

/// Where ffmpeg reads the source audio from.
pub enum AudioInput<'a> {
    Bytes(&'a [u8]),
    Path(&'a Path),
    /// (0.10 Q) Headerless mono s16le PCM at `rate` (Gemini TTS responses).
    RawPcm {
        bytes: &'a [u8],
        rate: u32,
    },
}

/// Decode `input` with ffmpeg to 48 kHz mono s16 PCM samples.
pub fn decode_to_pcm(input: AudioInput<'_>) -> Result<Vec<i16>, VoiceError> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin"]);
    if let AudioInput::RawPcm { rate, .. } = &input {
        cmd.args(["-f", "s16le", "-ar", &rate.to_string(), "-ac", "1"]);
    }
    cmd.arg("-i");
    match &input {
        AudioInput::Bytes(_) | AudioInput::RawPcm { .. } => {
            // `-nostdin` only disables interactive keys; pipe:0 is still readable.
            cmd.arg("pipe:0");
        }
        AudioInput::Path(p) => {
            cmd.arg(p);
        }
    }
    cmd.args([
        "-vn",
        "-f",
        "s16le",
        "-acodec",
        "pcm_s16le",
        "-ar",
        &SAMPLE_RATE.to_string(),
        "-ac",
        "1",
        "pipe:1",
    ]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(match input {
        AudioInput::Bytes(_) | AudioInput::RawPcm { .. } => Stdio::piped(),
        AudioInput::Path(_) => Stdio::null(),
    });
    let mut child = cmd
        .spawn()
        .map_err(|e| VoiceError::Audio(format!("cannot run ffmpeg: {e}")))?;

    // Feed stdin from a thread so a full stdout pipe cannot deadlock us.
    let feeder = match (&input, child.stdin.take()) {
        (AudioInput::Bytes(b) | AudioInput::RawPcm { bytes: b, .. }, Some(mut stdin)) => {
            let data = b.to_vec();
            Some(std::thread::spawn(move || {
                let _ = stdin.write_all(&data);
            }))
        }
        _ => None,
    };
    let mut stderr = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let mut raw = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_end(&mut raw)?;
    }
    let status = child.wait()?;
    if let Some(f) = feeder {
        let _ = f.join();
    }
    let err = err_thread.join().unwrap_or_default();
    if !status.success() {
        let msg: String = err.lines().take(3).collect::<Vec<_>>().join(" | ");
        return Err(VoiceError::Audio(format!("ffmpeg failed: {msg}")));
    }
    if raw.len() < 2 {
        return Err(VoiceError::Audio("ffmpeg produced no audio".into()));
    }
    Ok(raw
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect())
}

/// Convert any ffmpeg-readable audio to a 48 kHz mono s16 WAV file image.
pub fn to_wav_48k_mono(input: AudioInput<'_>) -> Result<Vec<u8>, VoiceError> {
    Ok(wav::encode(&decode_to_pcm(input)?))
}

/// Whether an `ffmpeg` binary can be started (tests skip when it cannot).
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
