//! Audio support (MVP): pure-Rust DECODING + metadata via `symphonia` (no audio device —
//! safe in headless CI). Every audio file gets a tile: a real **waveform** for formats
//! symphonia can decode (mp3/wav/ogg/flac/…), or a **music-note icon** for the rest
//! (trackers xm/it/s3m/mod, voc/au, midi). The Details pane shows duration / sample rate /
//! channels / codec. In-app *playback* (rodio → ALSA) is a deferred, default-off feature so
//! `cargo test` stays device-free — for now the "Open in default app" button + associations
//! launch the user's audio editor.

use super::cp437_font::CP437_8X16;
use super::{DecodeError, Decoder};
use crate::image_types::PixImage;
use std::io::Cursor;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

/// Every audio extension we claim. The decoder tries a waveform first (symphonia), then
/// falls back to the icon tile — so trackers/voc/au/midi list + open externally too.
pub const AUDIO_EXTS: &[&str] = &[
    // symphonia-decodable → waveform
    "mp3", "wav", "wave", "ogg", "oga", "flac", "ape", "mka", // icon-only → external open
    "voc", "au", "snd", "aiff", "aif", "aifc", "m4a", "aac", "opus", "wma", "ra", "mid", "midi",
    "rmi", "xm", "it", "s3m", "mod", "rad", "mtm", "669", "far", "okt", "stm", "ult", "med", "amf",
    "mptm",
];

/// Parsed audio metadata for the Details pane.
#[derive(Clone, Default)]
pub struct AudioInfo {
    pub duration_secs: f32,
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: String,
    pub bits_per_sample: Option<u32>, // None when the codec has no fixed bit depth (e.g. MP3)
    pub frames: u64,                  // decoded sample frames (per channel), 0 if unknown
}

/// A byte-slice media source (symphonia impls `MediaSource` only for `File`).
struct BytesSource(Cursor<Vec<u8>>);

impl std::io::Read for BytesSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl std::io::Seek for BytesSource {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}
impl MediaSource for BytesSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.get_ref().len() as u64)
    }
}

fn make_reader(bytes: &[u8], ext: &str) -> Option<Box<dyn symphonia::core::formats::FormatReader>> {
    let mss = MediaSourceStream::new(
        Box::new(BytesSource(Cursor::new(bytes.to_vec()))),
        Default::default(),
    );
    let mut hint = Hint::new();
    hint.with_extension(ext);
    symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .ok()
}

/// Best-effort metadata (duration / sample rate / channels / codec). `None` for formats
/// symphonia can't parse (trackers, voc/au, midi).
pub fn audio_info(bytes: &[u8], ext: &str) -> Option<AudioInfo> {
    let reader = make_reader(bytes, ext)?;
    let track = reader.default_track(TrackType::Audio)?;
    let params = track.codec_params.as_ref()?.audio()?;
    let sample_rate = params.sample_rate.unwrap_or(0);
    let channels = params
        .channels
        .as_ref()
        .map(|c| c.count() as u16)
        .unwrap_or(0);
    // Duration = playable frames / sample rate (frames excludes delay/padding).
    let frames = track.num_frames.unwrap_or(0);
    let duration_secs = if sample_rate > 0 {
        frames as f32 / sample_rate as f32
    } else {
        0.0
    };
    let codec = codec_label(ext);
    let bits_per_sample = params.bits_per_sample;
    Some(AudioInfo {
        duration_secs,
        sample_rate,
        channels,
        codec,
        bits_per_sample,
        frames,
    })
}

fn codec_label(ext: &str) -> String {
    ext.to_ascii_uppercase()
}

fn fmt_duration(secs: f32) -> String {
    let s = secs.round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

// ---- Rendering ---------------------------------------------------------------------------

const BG: [u8; 4] = [16, 18, 24, 255];
const AXIS: [u8; 4] = [40, 44, 54, 255];
const LABEL: [u8; 4] = [210, 210, 216, 255];
// Waveform/note colors come from `crate::format_color::color(ext)` (see `accent`).

const WAVE_W: usize = 900; // crisp waveform tile
const WAVE_H: usize = 200;
const MAX_FRAMES: u64 = 12_000_000; // ~4.5 min @ 44.1k — bounds thumbnail-worker time
const WINDOW: usize = 512; // frames per peak sample (finer envelope)

/// Decode a peak envelope (0..1 per column) for the file, or `None` if undecodable.
fn peaks(bytes: &[u8], ext: &str) -> Option<Vec<f32>> {
    let mut reader = make_reader(bytes, ext)?;
    let track = reader.default_track(TrackType::Audio)?;
    let track_id = track.id;
    let params = track.codec_params.as_ref()?.audio()?.clone();
    let mut dec = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .ok()?;

    let mut env: Vec<f32> = Vec::new(); // one peak per WINDOW frames
    let mut cur_peak = 0f32;
    let mut cur_count = 0usize;
    let mut total: u64 = 0;
    let mut scratch: Vec<f32> = Vec::new();

    'outer: while let Ok(Some(packet)) = reader.next_packet() {
        if packet.track_id != track_id {
            continue;
        }
        let buf = match dec.decode(&packet) {
            Ok(b) => b,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let frames = buf.frames();
        let chans = buf.spec().channels().count().max(1);
        if frames == 0 {
            continue;
        }
        scratch.clear();
        scratch.resize(frames * chans, 0.0);
        buf.copy_to_slice_interleaved(&mut scratch[..]);
        for f in 0..frames {
            // Peak across channels for this frame.
            let mut amp = 0f32;
            for c in 0..chans {
                amp = amp.max(scratch[f * chans + c].abs());
            }
            cur_peak = cur_peak.max(amp);
            cur_count += 1;
            if cur_count >= WINDOW {
                env.push(cur_peak);
                cur_peak = 0.0;
                cur_count = 0;
            }
            total += 1;
            if total >= MAX_FRAMES {
                break 'outer;
            }
        }
    }
    if cur_count > 0 {
        env.push(cur_peak);
    }
    if env.is_empty() {
        return None;
    }
    // Resample the envelope to WAVE_W columns (max within each bucket) + normalize.
    let mut cols = vec![0f32; WAVE_W];
    let n = env.len();
    for (i, col) in cols.iter_mut().enumerate() {
        let a = i * n / WAVE_W;
        let b = ((i + 1) * n / WAVE_W).max(a + 1).min(n);
        *col = env[a..b].iter().copied().fold(0.0, f32::max);
    }
    let peak = cols.iter().copied().fold(0.0, f32::max).max(1e-4);
    for c in cols.iter_mut() {
        *c = (*c / peak).clamp(0.0, 1.0);
    }
    Some(cols)
}

/// Render a waveform tile (with a duration/format caption).
/// The per-format waveform colors: a full-strength accent + a dimmer version for the bar body.
fn accent(ext: &str) -> ([u8; 4], [u8; 4]) {
    let [r, g, b] = crate::format_color::color(ext);
    let dim = |v: u8| (v as u16 * 55 / 100) as u8;
    ([r, g, b, 255], [dim(r), dim(g), dim(b), 255])
}

fn render_waveform(cols: &[f32], ext: &str, info: &Option<AudioInfo>) -> PixImage {
    let cap_h = 22usize;
    let w = WAVE_W;
    let h = WAVE_H + cap_h;
    let mut px = vec![BG; w * h];
    let (wave, wave_dim) = accent(ext); // colored by file format

    // Center axis.
    let mid = WAVE_H / 2;
    for x in 0..w {
        set(&mut px, w, x, mid, AXIS);
    }
    // Waveform bars, mirrored around the axis.
    for (x, &c) in cols.iter().enumerate().take(w) {
        let half = ((c * (WAVE_H as f32 * 0.46)).round() as usize).max(1);
        for dy in 0..half {
            let col = if dy > half * 3 / 4 { wave } else { wave_dim };
            set(&mut px, w, x, mid.saturating_sub(dy), col);
            set(&mut px, w, x, (mid + dy).min(WAVE_H - 1), col);
        }
        set(&mut px, w, x, mid, wave);
    }
    // Caption: "EXT · m:ss · 44.1k · stereo".
    let mut caption = ext.to_ascii_uppercase();
    if let Some(a) = info {
        if a.duration_secs > 0.0 {
            caption.push_str(&format!("  {}", fmt_duration(a.duration_secs)));
        }
        if a.sample_rate > 0 {
            caption.push_str(&format!("  {:.1}k", a.sample_rate as f32 / 1000.0));
        }
        caption.push_str(match a.channels {
            1 => "  mono",
            2 => "  stereo",
            _ => "",
        });
    }
    blit_text(&mut px, w, 6, WAVE_H + 4, &caption, LABEL, 1);
    PixImage::from_rgba(w as u32, h as u32, px)
}

/// A music-note icon tile for formats we can't decode (trackers, voc/au, midi). A clean ♫ drawn
/// from filled ellipse note-heads + stems + a beam — nicer than the old blocky CP437 glyph, and it
/// stays legible when the tile is downscaled (the thumbnailer area-averages). The format label sits
/// directly under the note (the old "audio - open in your player" line was removed — it overlapped
/// the label).
fn render_icon(ext: &str) -> PixImage {
    let (w, h) = (320usize, 240usize);
    let mut px = vec![BG; w * h];
    let (note, dim) = accent(ext); // colored by file format
    // Two beamed note-heads (symmetric ♫). A one-row `dim` under-edge gives the heads a little
    // roundness/depth against the dark background.
    fill_ellipse(&mut px, w, 116, 151, 34, 23, dim);
    fill_ellipse(&mut px, w, 204, 151, 34, 23, dim);
    fill_ellipse(&mut px, w, 116, 149, 34, 23, note);
    fill_ellipse(&mut px, w, 204, 149, 34, 23, note);
    // Stems up from each head's right edge, joined by a beam across their tops.
    fill_rect(&mut px, w, 143, 58, 154, 150, note);
    fill_rect(&mut px, w, 231, 58, 242, 150, note);
    fill_rect(&mut px, w, 143, 46, 242, 66, note);
    // Format label, centered directly under the note (the only text — legible when downscaled).
    let label = ext.to_ascii_uppercase();
    let tscale = 3usize;
    let tw = label.len() * 8 * tscale;
    blit_text(&mut px, w, (w.saturating_sub(tw)) / 2, 184, &label, note, tscale);
    PixImage::from_rgba(w as u32, h as u32, px)
}

/// Fill an axis-aligned rectangle `[x0,x1) × [y0,y1)` (signed coords; off-canvas pixels clipped).
fn fill_rect(px: &mut [[u8; 4]], w: usize, x0: i32, y0: i32, x1: i32, y1: i32, c: [u8; 4]) {
    for y in y0.max(0)..y1.max(0) {
        for x in x0.max(0)..x1.max(0) {
            set(px, w, x as usize, y as usize, c);
        }
    }
}

/// Fill an axis-aligned ellipse centered at `(cx,cy)` with radii `(rx,ry)`.
fn fill_ellipse(px: &mut [[u8; 4]], w: usize, cx: i32, cy: i32, rx: i32, ry: i32, c: [u8; 4]) {
    for y in (cy - ry)..=(cy + ry) {
        for x in (cx - rx)..=(cx + rx) {
            if x < 0 || y < 0 || rx == 0 || ry == 0 {
                continue;
            }
            let dx = (x - cx) as f32 / rx as f32;
            let dy = (y - cy) as f32 / ry as f32;
            if dx * dx + dy * dy <= 1.0 {
                set(px, w, x as usize, y as usize, c);
            }
        }
    }
}

fn set(px: &mut [[u8; 4]], w: usize, x: usize, y: usize, c: [u8; 4]) {
    if x < w {
        let i = y * w + x;
        if i < px.len() {
            px[i] = c;
        }
    }
}

fn blit_text(
    px: &mut [[u8; 4]],
    w: usize,
    x0: usize,
    y0: usize,
    s: &str,
    c: [u8; 4],
    scale: usize,
) {
    for (i, ch) in s.chars().enumerate() {
        let byte = if (0x20..0x7f).contains(&(ch as u32)) {
            ch as u8
        } else {
            b'?'
        };
        blit_glyph(px, w, x0 + i * 8 * scale, y0, byte, c, scale);
    }
}

fn blit_glyph(
    px: &mut [[u8; 4]],
    w: usize,
    x0: usize,
    y0: usize,
    ch: u8,
    c: [u8; 4],
    scale: usize,
) {
    let glyph = &CP437_8X16[ch as usize];
    for (ry, &bits) in glyph.iter().enumerate() {
        for rx in 0..8 {
            if (bits >> (7 - rx)) & 1 == 1 {
                for sy in 0..scale {
                    for sx in 0..scale {
                        set(px, w, x0 + rx * scale + sx, y0 + ry * scale + sy, c);
                    }
                }
            }
        }
    }
}

pub struct SoundDecoder;

impl Decoder for SoundDecoder {
    fn name(&self) -> &'static str {
        "audio"
    }

    fn extensions(&self) -> &'static [&'static str] {
        AUDIO_EXTS
    }

    fn sniff(&self, _header: &[u8]) -> bool {
        false // dispatch by extension (audio magics overlap; keep it simple)
    }

    fn decode(&self, _bytes: &[u8]) -> Result<PixImage, DecodeError> {
        // Needs the extension for the format hint — the registry routes to `decode_ext`.
        Err(DecodeError::Unsupported)
    }
}

impl SoundDecoder {
    /// Extension-aware decode: a waveform if symphonia can decode it, else the icon tile.
    pub fn decode_ext(bytes: &[u8], ext: &str) -> Result<PixImage, DecodeError> {
        let info = audio_info(bytes, ext);
        match peaks(bytes, ext) {
            Some(cols) => Ok(render_waveform(&cols, ext, &info)),
            None => Ok(render_icon(ext)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny valid mono 8-bit PCM WAV (a few samples) that symphonia can decode.
    fn mini_wav() -> Vec<u8> {
        let sample_rate = 8000u32;
        let data: Vec<u8> = (0..800).map(|i| ((i * 7) % 256) as u8).collect();
        let mut w = Vec::new();
        let chunk = 36 + data.len() as u32;
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&chunk.to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&1u16.to_le_bytes()); // mono
        w.extend_from_slice(&sample_rate.to_le_bytes());
        w.extend_from_slice(&sample_rate.to_le_bytes()); // byte rate (1ch*1byte)
        w.extend_from_slice(&1u16.to_le_bytes()); // block align
        w.extend_from_slice(&8u16.to_le_bytes()); // bits
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        w
    }

    #[test]
    fn wav_metadata_parses() {
        let info = audio_info(&mini_wav(), "wav").expect("wav parses");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert!(info.duration_secs > 0.0);
    }

    #[test]
    fn wav_renders_a_waveform() {
        let img = SoundDecoder::decode_ext(&mini_wav(), "wav").unwrap();
        assert_eq!(img.width, WAVE_W as u32);
        assert!(img.height > WAVE_H as u32);
    }

    #[test]
    fn undecodable_falls_back_to_icon() {
        // A .mod we can't decode → icon tile (fixed 320×240), never an error.
        let img = SoundDecoder::decode_ext(b"not really a module", "mod").unwrap();
        assert_eq!((img.width, img.height), (320, 240));
    }

    #[test]
    fn duration_formats_mm_ss() {
        assert_eq!(fmt_duration(83.0), "1:23");
        assert_eq!(fmt_duration(5.0), "0:05");
    }
}
