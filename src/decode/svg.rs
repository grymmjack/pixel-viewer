use super::{DecodeError, Decoder};
use crate::image_types::PixImage;
use resvg::{tiny_skia, usvg};

/// SVG via resvg/usvg/tiny-skia — rasterizes at the SVG's intrinsic size (capped
/// so a huge viewBox can't allocate gigabytes). Text uses usvg's default fonts.
pub struct SvgDecoder;

const MAX_DIM: f32 = 2048.0;

impl Decoder for SvgDecoder {
    fn name(&self) -> &'static str {
        "svg"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["svg"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        let head = String::from_utf8_lossy(&header[..header.len().min(64)]);
        let head = head.trim_start();
        head.starts_with("<?xml") || head.starts_with("<svg") || head.contains("<svg")
    }

    fn decode(&self, bytes: &[u8]) -> Result<PixImage, DecodeError> {
        let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
            .map_err(|e| DecodeError::Malformed(e.to_string()))?;
        let size = tree.size();
        let scale = (MAX_DIM / size.width().max(size.height()).max(1.0)).clamp(0.01, 1.0);
        let w = (size.width() * scale).round().max(1.0) as u32;
        let h = (size.height() * scale).round().max(1.0) as u32;

        let mut pixmap = tiny_skia::Pixmap::new(w, h)
            .ok_or_else(|| DecodeError::Malformed("SVG too large to rasterize".into()))?;
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );

        // tiny-skia stores premultiplied RGBA; un-premultiply for display.
        let pixels = pixmap
            .pixels()
            .iter()
            .map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        Ok(PixImage::from_rgba(w, h, pixels))
    }
}
