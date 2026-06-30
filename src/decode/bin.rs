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
        // Height is inferred from the data length, NOT SAUCE's TInfo2: for an
        // uncompressed (char, attr) stream the byte count is authoritative, and ansilove
        // (what 16colo.rs renders with) computes rows = bytes/2/width, ignoring TInfo2 for
        // BIN. A stale/garbage TInfo2 would otherwise pad the canvas with blank rows or
        // clip the art — the same wrong-dimension trap as the width default. This also
        // matches every other binary decoder here (IDF/ADF/Tundra all infer from data).
        let height = pairs.len().div_ceil(width).max(1);
        if width * height > MAX_CELLS {
            return Err(DecodeError::Malformed("BIN too large".into()));
        }
        Ok(super::xbin::render_textmode(
            width,
            height,
            &pairs,
            // Raw VGA attribute bytes → the VGA-ordered palette (index 1=blue, 4=red),
            // NOT the SGR-ordered ansi::PALETTE (which would swap red↔blue).
            &super::ansi::VGA_PALETTE,
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
        // No SAUCE → width defaults to 160; two cells: a red block (VGA attr 4 = red),
        // then a blank. The attribute byte is a *VGA* index, so 4 = red (not the ANSI
        // SGR order where 1 = red) — see VGA_PALETTE.
        let data = vec![0xDB, 0x04, 0x20, 0x00];
        let img = BinDecoder.decode(&data).unwrap();
        // 160 wide × 1 row (height inferred), 8×16 cells.
        assert_eq!((img.width, img.height), (160 * 8, 16));
        assert_eq!(img.pixels[0], [170, 0, 0, 255]); // first cell red
    }

    #[test]
    fn vga_attribute_indices_are_not_ansi_order() {
        // Regression for the red↔blue swap: BIN attribute bytes are VGA-ordered, so
        // index 1 must be BLUE and index 4 RED (the bug used the SGR-ordered palette,
        // which has them the other way round). Two full-block cells, fg = 1 then 4.
        let data = vec![0xDB, 0x01, 0xDB, 0x04];
        let img = BinDecoder.decode(&data).unwrap();
        assert_eq!(img.pixels[0], [0, 0, 170, 255], "VGA index 1 = blue");
        // The second cell starts at x=8 in the same top row.
        assert_eq!(img.pixels[8], [170, 0, 0, 255], "VGA index 4 = red");
    }

    #[test]
    fn binarytext_filetype_sets_width_and_height_is_inferred() {
        // The real-world bug (33-N1.BIN): a .BIN is SAUCE DataType 5 (BinaryText), whose
        // width is FileType × 2 — not TInfo1, which char_width() used to ignore for
        // non-Character art, so it fell back to 160 and sheared. Here FileType 2 → width 4;
        // 8 pairs / 4 = 2 rows (inferred from data, NOT the bogus TInfo2 below).
        let mut data = Vec::new();
        for _ in 0..8 {
            data.extend_from_slice(&[0xDB, 0x04]); // 8 red blocks
        }
        let mut s = vec![0u8; 128];
        s[..7].copy_from_slice(b"SAUCE00");
        s[94] = 5; // BinaryText
        s[95] = 2; // FileType → width = 4
        s[98] = 99; // TInfo2 = 99 (garbage line count — must be ignored)
        data.extend_from_slice(&s);
        let img = BinDecoder.decode(&data).unwrap();
        // 4 cols × 8px wide, 2 inferred rows × 16px tall — not 99 rows, not 160 wide.
        assert_eq!((img.width, img.height), (4 * 8, 2 * 16));
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
