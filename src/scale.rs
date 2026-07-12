//! Pixel-art **upscaling** algorithms for the Recolor pane's Resize → *Upscale* mode.
//!
//! Integer scalers that enlarge low-res sprites with edge-aware interpolation instead
//! of blocky nearest / blurry linear — the family charted at
//! <https://en.wikipedia.org/wiki/Pixel-art_scaling_algorithms>. They run as a
//! pre-pipeline step (upscale the source, then the rest of the recolor stack + Save
//! operate on the enlarged art). Everything works on straight (un-premultiplied) RGBA
//! byte buffers, comparing whole `[u8; 4]` pixels for equality, so it's self-contained
//! (no image-crate/version coupling) and unit-testable.
//!
//! Implemented so far: the classic hard-edge family (Scale2x/EPX, Scale3x, Eagle 2×/3×).
//! `Scaler` is the reorderable-free enum the UI + persistence key off; adding a new
//! algorithm is one variant + one function.

/// Which pixel-art upscaler to apply (or `None` = leave the source untouched).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scaler {
    #[default]
    None,
    Scale2x,
    Scale3x,
    Eagle2x,
    Eagle3x,
}

impl Scaler {
    /// Every scaler, in menu order. `ALL[i] as u8 == i`, so the index persists.
    pub const ALL: [Scaler; 5] = [
        Scaler::None,
        Scaler::Scale2x,
        Scaler::Scale3x,
        Scaler::Eagle2x,
        Scaler::Eagle3x,
    ];

    pub fn from_u8(b: u8) -> Scaler {
        Self::ALL.get(b as usize).copied().unwrap_or(Scaler::None)
    }

    pub fn label(self) -> &'static str {
        match self {
            Scaler::None => "None",
            Scaler::Scale2x => "Scale2x (EPX)",
            Scaler::Scale3x => "Scale3x",
            Scaler::Eagle2x => "Eagle 2×",
            Scaler::Eagle3x => "Eagle 3×",
        }
    }

    /// The integer enlargement factor (1 for `None`).
    pub fn factor(self) -> usize {
        match self {
            Scaler::None => 1,
            Scaler::Scale2x | Scaler::Eagle2x => 2,
            Scaler::Scale3x | Scaler::Eagle3x => 3,
        }
    }

    /// Upscale `rgba` (`w*h*4` bytes) → `(out, w*factor, h*factor)`. `None` (or a
    /// degenerate size) returns the input unchanged so callers can apply it blindly.
    pub fn apply(self, rgba: &[u8], w: usize, h: usize) -> (Vec<u8>, usize, usize) {
        if self == Scaler::None || w == 0 || h == 0 || rgba.len() < w * h * 4 {
            return (rgba.to_vec(), w, h);
        }
        let src = Grid { rgba, w, h };
        match self {
            Scaler::None => unreachable!(),
            Scaler::Scale2x => scale2x(&src),
            Scaler::Scale3x => scale3x(&src),
            Scaler::Eagle2x => eagle2x(&src),
            Scaler::Eagle3x => eagle3x(&src),
        }
    }
}

/// A borrowed RGBA source with edge-clamped pixel access.
struct Grid<'a> {
    rgba: &'a [u8],
    w: usize,
    h: usize,
}

impl Grid<'_> {
    /// Pixel at `(x, y)`, clamped to the edges (so a 3×3 neighbourhood never reads
    /// out of bounds and the border replicates rather than wraps).
    #[inline]
    fn at(&self, x: isize, y: isize) -> [u8; 4] {
        let cx = x.clamp(0, self.w as isize - 1) as usize;
        let cy = y.clamp(0, self.h as isize - 1) as usize;
        let i = (cy * self.w + cx) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

/// Write one `factor×factor` output cell for source pixel `(x, y)` into `out`.
#[inline]
fn put_cell(out: &mut [u8], ow: usize, x: usize, y: usize, factor: usize, cell: &[[u8; 4]]) {
    for dy in 0..factor {
        for dx in 0..factor {
            let px = cell[dy * factor + dx];
            let o = ((y * factor + dy) * ow + (x * factor + dx)) * 4;
            out[o..o + 4].copy_from_slice(&px);
        }
    }
}

/// **Scale2x / EPX** — the classic 2× expander (Andrea Mazzoleni / Eric Johnston). Each
/// pixel becomes 2×2: a corner takes an orthogonal neighbour's colour only when that
/// neighbour matches the adjacent one *and* the opposing pair differs (an edge), which
/// rounds jaggies without inventing colours. Uses the 3×3 neighbourhood A B C / D E F /
/// G H I (E = centre).
fn scale2x(s: &Grid) -> (Vec<u8>, usize, usize) {
    let (ow, oh) = (s.w * 2, s.h * 2);
    let mut out = vec![0u8; ow * oh * 4];
    for y in 0..s.h as isize {
        for x in 0..s.w as isize {
            let (b, d, e, f, hh) = (
                s.at(x, y - 1),
                s.at(x - 1, y),
                s.at(x, y),
                s.at(x + 1, y),
                s.at(x, y + 1),
            );
            let (mut e0, mut e1, mut e2, mut e3) = (e, e, e, e);
            if b != hh && d != f {
                if d == b {
                    e0 = d;
                }
                if b == f {
                    e1 = f;
                }
                if d == hh {
                    e2 = d;
                }
                if hh == f {
                    e3 = f;
                }
            }
            put_cell(&mut out, ow, x as usize, y as usize, 2, &[e0, e1, e2, e3]);
        }
    }
    (out, ow, oh)
}

/// **Scale3x** — the 3× sibling of Scale2x (same edge test, nine output pixels). The
/// centre stays the source; corners mirror Scale2x; edges fill on the two-neighbour
/// rules. 3×3 neighbourhood A B C / D E F / G H I (E = centre).
fn scale3x(s: &Grid) -> (Vec<u8>, usize, usize) {
    let (ow, oh) = (s.w * 3, s.h * 3);
    let mut out = vec![0u8; ow * oh * 4];
    for y in 0..s.h as isize {
        for x in 0..s.w as isize {
            let a = s.at(x - 1, y - 1);
            let b = s.at(x, y - 1);
            let c = s.at(x + 1, y - 1);
            let d = s.at(x - 1, y);
            let e = s.at(x, y);
            let f = s.at(x + 1, y);
            let g = s.at(x - 1, y + 1);
            let hh = s.at(x, y + 1);
            let i = s.at(x + 1, y + 1);
            let cell = if b != hh && d != f {
                [
                    if d == b { d } else { e },
                    if (d == b && e != c) || (b == f && e != a) {
                        b
                    } else {
                        e
                    },
                    if b == f { f } else { e },
                    if (d == b && e != g) || (d == hh && e != a) {
                        d
                    } else {
                        e
                    },
                    e,
                    if (b == f && e != i) || (hh == f && e != c) {
                        f
                    } else {
                        e
                    },
                    if d == hh { d } else { e },
                    if (d == hh && e != i) || (hh == f && e != g) {
                        hh
                    } else {
                        e
                    },
                    if hh == f { f } else { e },
                ]
            } else {
                [e; 9]
            };
            put_cell(&mut out, ow, x as usize, y as usize, 3, &cell);
        }
    }
    (out, ow, oh)
}

/// **Eagle 2×** — each output corner takes the corner colour when the three source
/// neighbours forming it (two orthogonal + the diagonal) all match, else the centre.
/// Fatter than Scale2x on solid diagonals. 3×3 A B C / D E F / G H I (E = centre).
fn eagle2x(s: &Grid) -> (Vec<u8>, usize, usize) {
    let (ow, oh) = (s.w * 2, s.h * 2);
    let mut out = vec![0u8; ow * oh * 4];
    for y in 0..s.h as isize {
        for x in 0..s.w as isize {
            let a = s.at(x - 1, y - 1);
            let b = s.at(x, y - 1);
            let c = s.at(x + 1, y - 1);
            let d = s.at(x - 1, y);
            let e = s.at(x, y);
            let f = s.at(x + 1, y);
            let g = s.at(x - 1, y + 1);
            let hh = s.at(x, y + 1);
            let i = s.at(x + 1, y + 1);
            let cell = [
                if a == b && b == d { a } else { e },
                if b == c && c == f { c } else { e },
                if d == g && g == hh { g } else { e },
                if f == hh && hh == i { i } else { e },
            ];
            put_cell(&mut out, ow, x as usize, y as usize, 2, &cell);
        }
    }
    (out, ow, oh)
}

/// **Eagle 3×** — Eagle's corner rule at 3× (the four corners get the Eagle treatment;
/// the centre cross stays the source pixel). 3×3 A B C / D E F / G H I (E = centre).
fn eagle3x(s: &Grid) -> (Vec<u8>, usize, usize) {
    let (ow, oh) = (s.w * 3, s.h * 3);
    let mut out = vec![0u8; ow * oh * 4];
    for y in 0..s.h as isize {
        for x in 0..s.w as isize {
            let a = s.at(x - 1, y - 1);
            let b = s.at(x, y - 1);
            let c = s.at(x + 1, y - 1);
            let d = s.at(x - 1, y);
            let e = s.at(x, y);
            let f = s.at(x + 1, y);
            let g = s.at(x - 1, y + 1);
            let hh = s.at(x, y + 1);
            let i = s.at(x + 1, y + 1);
            let tl = if a == b && b == d { a } else { e };
            let tr = if b == c && c == f { c } else { e };
            let bl = if d == g && g == hh { g } else { e };
            let br = if f == hh && hh == i { i } else { e };
            let cell = [tl, b, tr, d, e, f, bl, hh, br];
            put_cell(&mut out, ow, x as usize, y as usize, 3, &cell);
        }
    }
    (out, ow, oh)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A tiny helper: build a w×h RGBA buffer from a grayscale grid (0/255).
    fn buf(vals: &[u8], w: usize) -> Vec<u8> {
        vals.iter().flat_map(|&v| [v, v, v, 255]).collect()
    }
    fn px(out: &[u8], ow: usize, x: usize, y: usize) -> u8 {
        out[(y * ow + x) * 4]
    }

    #[test]
    fn factor_and_output_size() {
        let src = buf(&[0, 255, 255, 0], 2); // 2×2
        for (sc, f) in [
            (Scaler::Scale2x, 2),
            (Scaler::Scale3x, 3),
            (Scaler::Eagle2x, 2),
            (Scaler::Eagle3x, 3),
        ] {
            let (out, ow, oh) = sc.apply(&src, 2, 2);
            assert_eq!((ow, oh), (2 * f, 2 * f));
            assert_eq!(out.len(), ow * oh * 4);
        }
        // None is a passthrough.
        let (out, ow, oh) = Scaler::None.apply(&src, 2, 2);
        assert_eq!((ow, oh, out), (2, 2, src));
    }

    #[test]
    fn scale2x_rounds_a_diagonal_step() {
        // A 3×3 with a diagonal edge: centre is white, up+left are white, the opposing
        // down/right are black → Scale2x fills the top-left output corner white.
        //   B W .        (W=255, .=0)
        //   W W .
        //   . . .
        let src = buf(&[255, 255, 0, 255, 255, 0, 0, 0, 0], 3);
        let (out, ow, _) = Scaler::Scale2x.apply(&src, 3, 3);
        // Centre pixel (1,1) expands to out (2,2)..(3,3): its top-left corner rounds to
        // white because up(=255)==left(=255) and up!=down / left!=right.
        assert_eq!(px(&out, ow, 2, 2), 255, "top-left corner rounded to white");
    }

    #[test]
    fn eagle2x_fills_a_solid_corner() {
        // up-left, up and left all white around a black centre → Eagle paints the whole
        // top-left output corner white (fatter than Scale2x).
        //   W W .
        //   W . .
        //   . . .
        let src = buf(&[255, 255, 0, 255, 0, 0, 0, 0, 0], 3);
        let (out, ow, _) = Scaler::Eagle2x.apply(&src, 3, 3);
        assert_eq!(px(&out, ow, 2, 2), 255, "eagle filled the corner white");
    }
}
