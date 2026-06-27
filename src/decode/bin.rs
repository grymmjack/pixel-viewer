//! Raw BIN — the simplest scene text-mode format: a headerless stream of
//! (character, attribute) pairs. There's no width in the data, so it comes from
//! the SAUCE record (TInfo1) or the 160-column scene default; iCE colors and the
//! line count likewise come from SAUCE. Rendered with the default VGA font/palette.

use super::{DecodeError, Decoder};
use crate::image_types::PixImage;

pub struct BinDecoder;

const DEFAULT_WIDTH: usize = 160; // the scene default for header-less BIN
const MAX_CELLS: usize = 250_000;

impl Decoder for BinDecoder {
    fn name(&self) -> &'static str {
        "bin"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["bin"]
    }

    fn sniff(&self, _header: &[u8]) -> bool {
        false // headerless — dispatched by the .bin extension only
    }

    fn decode(&self, bytes: &[u8]) -> Result<PixImage, DecodeError> {
        let sauce = crate::sauce::parse(bytes);
        let ice = sauce.as_ref().map(|s| s.ice).unwrap_or(true);
        let width = sauce
            .as_ref()
            .and_then(|s| s.char_width())
            .unwrap_or(DEFAULT_WIDTH)
            .clamp(1, 1000);
        let data = crate::sauce::strip(bytes);
        let pairs: Vec<(u8, u8)> = data.chunks_exact(2).map(|c| (c[0], c[1])).collect();
        if pairs.is_empty() {
            return Err(DecodeError::Malformed("empty BIN".into()));
        }
        // Prefer SAUCE's line count; otherwise infer from the data length.
        let height = sauce
            .as_ref()
            .map(|s| s.tinfo2 as usize)
            .filter(|&h| h > 0)
            .unwrap_or_else(|| pairs.len().div_ceil(width))
            .max(1);
        if width * height > MAX_CELLS {
            return Err(DecodeError::Malformed("BIN too large".into()));
        }
        Ok(super::xbin::render_textmode(
            width,
            height,
            &pairs,
            &super::ansi::PALETTE,
            None,
            16,
            ice,
            false,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_bin_row() {
        // No SAUCE → width defaults to 160; two cells: red block, then a blank.
        let data = vec![0xDB, 0x01, 0x20, 0x00];
        let img = BinDecoder.decode(&data).unwrap();
        // 160 wide × 1 row (height inferred), 8×16 cells.
        assert_eq!((img.width, img.height), (160 * 8, 16));
        assert_eq!(img.pixels[0], [170, 0, 0, 255]); // first cell red
    }

    #[test]
    fn sauce_width_sets_the_canvas() {
        // 4 cells with a SAUCE width of 2 → a 2×2 canvas (16×32 px).
        let mut data = vec![0xDB, 0x01, 0xDB, 0x02, 0xDB, 0x04, 0xDB, 0x06];
        let mut s = vec![0u8; 128];
        s[..7].copy_from_slice(b"SAUCE00");
        s[94] = 1; // Character
        s[96] = 2; // TInfo1 width = 2
        data.extend_from_slice(&s);
        let img = BinDecoder.decode(&data).unwrap();
        assert_eq!((img.width, img.height), (16, 32));
    }
}
