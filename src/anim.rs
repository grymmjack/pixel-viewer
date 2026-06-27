//! Animated-image (GIF) frame decoding — frames are fully composited onto the
//! canvas by the `image` crate, so each is a complete width×height RGBA image.

use crate::image_types::Rgba;
use image::AnimationDecoder;

pub struct AnimFrames {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<Vec<Rgba>>,
    pub delays_ms: Vec<u16>,
}

#[cfg(test)]
impl AnimFrames {
    /// Total loop duration in milliseconds. (Test-only; the viewer computes this
    /// inline from `AnimState`.)
    pub fn total_ms(&self) -> u32 {
        self.delays_ms.iter().map(|&d| u32::from(d)).sum()
    }
}

/// Decode a GIF's frames + per-frame delays. Returns `None` for a non-GIF, a
/// decode error, or a single-frame GIF (handled as a static image elsewhere).
pub fn decode_gif(bytes: &[u8]) -> Option<AnimFrames> {
    let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let frames = dec.into_frames().collect_frames().ok()?;
    if frames.len() <= 1 {
        return None;
    }
    let (width, height) = {
        let b = frames[0].buffer();
        (b.width(), b.height())
    };
    let mut out = Vec::with_capacity(frames.len());
    let mut delays_ms = Vec::with_capacity(frames.len());
    for f in &frames {
        let (num, den) = f.delay().numer_denom_ms();
        let ms = num.checked_div(den).unwrap_or(100);
        // Floor very short delays (many GIFs say 0/10ms but browsers clamp).
        delays_ms.push(ms.clamp(20, u32::from(u16::MAX)) as u16);
        let px: Vec<Rgba> = f
            .buffer()
            .as_raw()
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        out.push(px);
    }
    Some(AnimFrames {
        width,
        height,
        frames: out,
        delays_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny 1×1, 2-frame GIF (red then blue) with the image crate.
    fn two_frame_gif() -> Vec<u8> {
        use image::{codecs::gif::GifEncoder, Delay, Frame, RgbaImage};
        let mut buf = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut buf);
            let red = RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
            let blue = RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 255, 255]));
            let d = Delay::from_numer_denom_ms(100, 1);
            enc.encode_frame(Frame::from_parts(red, 0, 0, d)).unwrap();
            enc.encode_frame(Frame::from_parts(blue, 0, 0, d)).unwrap();
        }
        buf
    }

    #[test]
    fn decodes_multi_frame_gif() {
        let af = decode_gif(&two_frame_gif()).expect("animated");
        assert_eq!((af.width, af.height), (1, 1));
        assert_eq!(af.frames.len(), 2);
        assert_eq!(af.delays_ms.len(), 2);
        assert!(af.total_ms() >= 200);
    }

    #[test]
    fn single_frame_is_not_animated() {
        use image::{codecs::gif::GifEncoder, Delay, Frame, RgbaImage};
        let mut buf = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut buf);
            let img = RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
            enc.encode_frame(Frame::from_parts(
                img,
                0,
                0,
                Delay::from_numer_denom_ms(100, 1),
            ))
            .unwrap();
        }
        assert!(decode_gif(&buf).is_none());
    }
}
