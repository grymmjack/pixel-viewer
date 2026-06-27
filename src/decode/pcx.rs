use super::{DecodeError, Decoder};
use crate::image_types::PixImage;

/// Hand-written PCX decoder. PCX is a great first "exotic" format: simple RLE,
/// genuinely palette-based, and everywhere in DOS-era pixel art. This is the
/// template to copy for IFF/ILBM, LBM, or anything the `image` crate lacks.
///
/// Handles the two common variants:
///   * 8bpp / 1 plane  -> indexed, 256-color VGA palette at end of file
///   * 8bpp / 3 planes -> truecolor RGB, stored plane-by-plane per scanline
pub struct PcxDecoder;

impl Decoder for PcxDecoder {
    fn name(&self) -> &'static str {
        "pcx"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["pcx"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.first() == Some(&0x0A)
    }

    fn decode(&self, bytes: &[u8]) -> Result<PixImage, DecodeError> {
        if bytes.len() < 128 || bytes[0] != 0x0A {
            return Err(DecodeError::Malformed("not a PCX file".into()));
        }
        let rd_u16 = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);

        let encoding = bytes[2];
        let bits_per_pixel = bytes[3];
        let xmin = rd_u16(4) as i32;
        let ymin = rd_u16(6) as i32;
        let xmax = rd_u16(8) as i32;
        let ymax = rd_u16(10) as i32;
        let n_planes = bytes[65] as usize;
        let bytes_per_line = rd_u16(66) as usize;

        if encoding != 1 {
            return Err(DecodeError::Malformed(
                "only RLE-encoded PCX supported".into(),
            ));
        }
        let width = (xmax - xmin + 1).max(0) as usize;
        let height = (ymax - ymin + 1).max(0) as usize;
        if width == 0 || height == 0 || bytes_per_line == 0 {
            return Err(DecodeError::Malformed("zero-sized image".into()));
        }
        let total_per_line = n_planes * bytes_per_line;

        // RLE-decode exactly height * total_per_line bytes from the body.
        let body = &bytes[128..];
        let mut scan = vec![0u8; height * total_per_line];
        let mut si = 0usize; // source index into body
        let mut di = 0usize; // dest index into scan
        while di < scan.len() {
            let b = *body
                .get(si)
                .ok_or_else(|| DecodeError::Malformed("truncated RLE stream".into()))?;
            si += 1;
            if b & 0xC0 == 0xC0 {
                let count = (b & 0x3F) as usize;
                let val = *body
                    .get(si)
                    .ok_or_else(|| DecodeError::Malformed("truncated RLE run".into()))?;
                si += 1;
                for _ in 0..count {
                    if di >= scan.len() {
                        break;
                    }
                    scan[di] = val;
                    di += 1;
                }
            } else {
                scan[di] = b;
                di += 1;
            }
        }

        // 8bpp, single plane => indexed with a 256-color VGA palette at EOF.
        if bits_per_pixel == 8 && n_planes == 1 {
            let mut palette = vec![[0u8, 0, 0, 255]; 256];
            if bytes.len() >= 769 && bytes[bytes.len() - 769] == 0x0C {
                let pal = &bytes[bytes.len() - 768..];
                for i in 0..256 {
                    palette[i] = [pal[i * 3], pal[i * 3 + 1], pal[i * 3 + 2], 255];
                }
            }
            let mut indices = vec![0u8; width * height];
            for y in 0..height {
                let row = &scan[y * total_per_line..y * total_per_line + bytes_per_line];
                for x in 0..width {
                    indices[y * width + x] = row[x];
                }
            }
            return Ok(PixImage::from_indexed(
                width as u32,
                height as u32,
                indices,
                palette,
            ));
        }

        // 8bpp, three planes => truecolor RGB, one plane after another per line.
        if bits_per_pixel == 8 && n_planes == 3 {
            let mut pixels = vec![[0u8, 0, 0, 255]; width * height];
            for y in 0..height {
                let line = &scan[y * total_per_line..(y + 1) * total_per_line];
                for x in 0..width {
                    let r = line[x];
                    let g = line[bytes_per_line + x];
                    let b = line[2 * bytes_per_line + x];
                    pixels[y * width + x] = [r, g, b, 255];
                }
            }
            return Ok(PixImage::from_rgba(width as u32, height as u32, pixels));
        }

        Err(DecodeError::Malformed(format!(
            "unsupported PCX variant: {bits_per_pixel}bpp x {n_planes} planes"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::Decoder;

    /// A minimal 2×2, 8bpp/1-plane (indexed) RLE PCX with a 256-color VGA palette:
    /// indices [0,1,2,3], colors black / red / green / blue.
    fn sample_pcx() -> Vec<u8> {
        let mut header = vec![0u8; 128];
        header[0] = 0x0A; // magic
        header[1] = 5; // version
        header[2] = 1; // RLE encoding
        header[3] = 8; // bits per pixel
        header[8] = 1; // xmax = 1 -> width 2
        header[10] = 1; // ymax = 1 -> height 2
        header[65] = 1; // n_planes
        header[66] = 2; // bytes_per_line
        let body = vec![0u8, 1, 2, 3]; // 2 scanlines × 2 px, all literals (< 0xC0)
        let mut pal = vec![0u8; 769];
        pal[0] = 0x0C; // palette marker
        let colors = [[0u8, 0, 0], [255, 0, 0], [0, 255, 0], [0, 0, 255]];
        for (i, c) in colors.iter().enumerate() {
            pal[1 + i * 3..1 + i * 3 + 3].copy_from_slice(c);
        }
        [header, body, pal].concat()
    }

    #[test]
    fn decodes_indexed_pcx_with_palette() {
        let img = PcxDecoder.decode(&sample_pcx()).expect("decode");
        assert_eq!((img.width, img.height), (2, 2));
        let idx = img.indexed.expect("indexed");
        assert_eq!(idx.indices, vec![0, 1, 2, 3]);
        assert_eq!(idx.palette[1], [255, 0, 0, 255]);
        assert_eq!(img.pixels[3], [0, 0, 255, 255]);
    }

    #[test]
    fn sniffs_magic_and_rejects_junk() {
        assert!(PcxDecoder.sniff(&[0x0A, 5, 1, 8]));
        assert!(!PcxDecoder.sniff(&[0x42]));
        assert!(PcxDecoder.decode(&[0x0A, 0, 0]).is_err()); // too short
    }
}
