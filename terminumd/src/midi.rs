// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! MIDI files rendered into WAV for View's player (AC-64): browsers play no MIDI. The page sends
//! the file's bytes (it has them, from this Mac or a host); terminumd renders them with a General
//! MIDI SoundFont of the user's (settings.json's `midi.soundfont`, else `~/.terminum/soundfont.sf2`;
//! none is shipped) through rustysynth, the bank read once and kept while its file is unchanged.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use parking_lot::Mutex;
use rustysynth::{MidiFile, MidiFileSequencer, SoundFont, Synthesizer, SynthesizerSettings};

use crate::app::Shared;
use crate::http::err;

/// A MIDI file's bytes at most.
pub const MAX_BYTES: usize = 4 << 20;
const RATE: i32 = 44100;
/// Seconds rendered at most, and of quiet after the last event (its notes' release).
const LONGEST: f64 = 10.0 * 60.0;
const TAIL: f64 = 2.0;

/// The SoundFont last read: its path, modification time and bank.
static BANK: Mutex<Option<(PathBuf, SystemTime, Arc<SoundFont>)>> = Mutex::new(None);

/// Where the SoundFont is: settings.json's `midi.soundfont` (`~/` the home), else the default.
pub fn soundfont_path(named: Option<&str>) -> PathBuf {
    match named {
        Some(p) if p.starts_with("~/") => crate::config::home().join(&p[2..]),
        Some(p) => PathBuf::from(p),
        None => crate::config::tm_home().join("soundfont.sf2"),
    }
}

fn bank(path: &PathBuf) -> Result<Arc<SoundFont>, String> {
    let shown = path.display().to_string().replace(&crate::config::home().display().to_string(), "~");
    let stamp = std::fs::metadata(path).and_then(|m| m.modified())
        .map_err(|_| format!("No SoundFont for MIDI: put a General MIDI .sf2 at {shown} or name one in settings.json (midi.soundfont)"))?;
    if let Some((p, t, b)) = BANK.lock().as_ref() && p == path && *t == stamp {
        return Ok(b.clone());
    }
    let mut file = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| format!("{shown}: {e}"))?);
    let b = Arc::new(SoundFont::new(&mut file).map_err(|e| format!("{shown} is not a SoundFont: {e:?}"))?);
    *BANK.lock() = Some((path.clone(), stamp, b.clone()));
    Ok(b)
}

/// The MIDI file `bytes` rendered with `font`: a 16-bit stereo WAV.
pub fn render(font: &Arc<SoundFont>, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let midi = Arc::new(MidiFile::new(&mut Cursor::new(bytes)).map_err(|e| format!("Not a MIDI file this can play: {e:?}"))?);
    let length = midi.get_length();
    if length > LONGEST {
        return Err(format!("It lasts more than {} minutes", LONGEST as u32 / 60));
    }
    let synth = Synthesizer::new(font, &SynthesizerSettings::new(RATE)).map_err(|e| format!("The SoundFont cannot play: {e:?}"))?;
    let mut seq = MidiFileSequencer::new(synth);
    seq.play(&midi, false);
    // a block at a time into the WAV: the samples are never all held as floats
    let frames = ((length + TAIL) * RATE as f64) as usize;
    let mut out = wav_head(frames);
    let (mut l, mut r) = (vec![0f32; 4096], vec![0f32; 4096]);
    for at in (0..frames).step_by(4096) {
        let n = 4096.min(frames - at);
        seq.render(&mut l[..n], &mut r[..n]);
        put(&mut out, &l[..n], &r[..n]);
    }
    Ok(out)
}

#[cfg(test)]
fn wav(left: &[f32], right: &[f32]) -> Vec<u8> {
    let mut out = wav_head(left.len());
    put(&mut out, left, right);
    out
}

/// A 16-bit stereo WAV's head for `frames` frames, room made for its data.
fn wav_head(frames: usize) -> Vec<u8> {
    let data = frames * 4;
    let mut out = Vec::with_capacity(44 + data);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes(), [1, 0, 2, 0], (RATE as u32).to_le_bytes(), (RATE as u32 * 4).to_le_bytes(), [4, 0, 16, 0]] {
        out.extend_from_slice(&v);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    out
}

fn put(out: &mut Vec<u8>, left: &[f32], right: &[f32]) {
    for (l, r) in left.iter().zip(right) {
        for s in [l, r] {
            out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
}

/// `POST /api/midi`: the file's bytes in, its WAV out (`x-tm-soundfont`: the bank's name).
pub async fn post(State(app): State<Shared>, body: Bytes) -> Response {
    let path = soundfont_path(app.looks.settings.read().midi_soundfont.as_deref());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let done = tokio::task::spawn_blocking(move || bank(&path).and_then(|b| render(&b, &body))).await;
    match done {
        Ok(Ok(wav)) => {
            let mut res = wav.into_response();
            res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
            if let Ok(v) = HeaderValue::from_str(&name) {
                res.headers_mut().insert("x-tm-soundfont", v);
            }
            res
        }
        Ok(Err(e)) => err(StatusCode::UNPROCESSABLE_ENTITY, "midi", e),
        Err(_) => err(StatusCode::INTERNAL_SERVER_ERROR, "midi", "Rendering failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A format-0 file: tempo 120, a kick and a snare on channel 10, then a piano C4, 1.5 s in all.
    fn song() -> Vec<u8> {
        let mut t = vec![0x00, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20, 0x00, 0x99, 36, 100, 0x60, 0x89, 36, 0, 0x00, 0x99, 38, 100, 0x60, 0x89, 38, 0,
                         0x00, 0x90, 60, 90, 0x83, 0x00, 0x80, 60, 0, 0x00, 0xff, 0x2f, 0x00];
        let mut f = b"MThd\0\0\0\x06\0\0\0\x01\x01\xe0MTrk".to_vec();
        f.extend_from_slice(&(t.len() as u32).to_be_bytes());
        f.append(&mut t);
        f
    }

    #[test]
    fn the_soundfont_is_the_settings_one_or_the_default() {
        assert_eq!(soundfont_path(Some("/x/gm.sf2")), PathBuf::from("/x/gm.sf2"));
        assert_eq!(soundfont_path(Some("~/gm.sf2")), crate::config::home().join("gm.sf2"));
        assert!(soundfont_path(None).ends_with("soundfont.sf2"));
    }

    #[test]
    fn no_soundfont_says_where_to_put_one() {
        let e = bank(&PathBuf::from("/nonexistent/terminum/soundfont.sf2")).err().unwrap();
        assert!(e.starts_with("No SoundFont for MIDI: put a General MIDI .sf2 at /nonexistent/terminum/soundfont.sf2"), "{e}");
    }

    #[test]
    fn a_wav_is_riff_stereo_16_bit() {
        let w = wav(&[0.5, -1.0], &[0.0, 2.0]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(w.len(), 44 + 8);
        assert_eq!(i16::from_le_bytes([w[44], w[45]]), 16383);
        assert_eq!(i16::from_le_bytes([w[50], w[51]]), 32767, "clamped");
    }

    /// With a SoundFont named in TM_TEST_SOUNDFONT: a song renders to sound, not silence.
    #[test]
    fn a_song_renders_with_a_real_soundfont() {
        let Ok(path) = std::env::var("TM_TEST_SOUNDFONT") else { return };
        let font = bank(&PathBuf::from(path)).unwrap();
        let w = render(&font, &song()).unwrap();
        let loud = w[44..].chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max().unwrap();
        assert!(w.len() > 44 + 4 * RATE as usize && loud > 1000, "{} bytes, peak {loud}", w.len());
        assert!(render(&font, b"not midi").unwrap_err().starts_with("Not a MIDI file"));
    }
}
