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
//! Implemented so far: the classic hard-edge family (Scale2x/EPX, Scale3x, Eagle 2×/3×)
//! and **xBR 2×/3×/4×** (the edge-directed *blending* family — 2×/3× ported faithfully
//! from libxbr-standalone's FILT2/FILT3, 4× = 2× chained twice). `Scaler` is the enum
//! the UI + persistence key off; adding a new algorithm is one variant + one function.

/// Which pixel-art upscaler to apply (or `None` = leave the source untouched).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scaler {
    #[default]
    None,
    Scale2x,
    Scale3x,
    Eagle2x,
    Eagle3x,
    Xbr2x,
    Xbr3x,
    Xbr4x,
}

impl Scaler {
    /// Every scaler, in menu order. `ALL[i] as u8 == i`, so the index persists.
    pub const ALL: [Scaler; 8] = [
        Scaler::None,
        Scaler::Scale2x,
        Scaler::Scale3x,
        Scaler::Eagle2x,
        Scaler::Eagle3x,
        Scaler::Xbr2x,
        Scaler::Xbr3x,
        Scaler::Xbr4x,
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
            Scaler::Xbr2x => "xBR 2×",
            Scaler::Xbr3x => "xBR 3×",
            Scaler::Xbr4x => "xBR 4×",
        }
    }

    /// The integer enlargement factor (1 for `None`).
    pub fn factor(self) -> usize {
        match self {
            Scaler::None => 1,
            Scaler::Scale2x | Scaler::Eagle2x | Scaler::Xbr2x => 2,
            Scaler::Scale3x | Scaler::Eagle3x | Scaler::Xbr3x => 3,
            Scaler::Xbr4x => 4,
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
            Scaler::Xbr2x => xbr2x(&src),
            Scaler::Xbr3x => xbr3x(&src),
            Scaler::Xbr4x => xbr4x(&src),
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
            // Only the four corners get the Eagle treatment; the cross (edges + centre)
            // stays the source pixel — filling the edges with neighbours fringed colour.
            let cell = [tl, e, tr, e, e, e, bl, e, br];
            put_cell(&mut out, ow, x as usize, y as usize, 3, &cell);
        }
    }
    (out, ow, oh)
}

// ---------------------------------------------------------------------------------
// xBR (Hyllian) — an edge-directed *blending* scaler (the smooth family). Ported
// faithfully from libxbr-standalone's FILT2 macro (Treeki/Hyllian). Unlike the hard
// scalers above it produces intermediate colours, so it needs YUV-distance edge
// detection + weighted blends. The 4 output sub-pixels are handled by rotating the
// neighbourhood 90° four times and applying the SAME filter — the rotation is derived
// from the 5×5 grid geometry (below), so there's no hand-transcribed permutation.
// ---------------------------------------------------------------------------------

/// BT.601 Y/U/V (0..255-ish) for a pixel — the space xBR measures colour distance in.
#[inline]
fn yuv(p: [u8; 4]) -> (i32, i32, i32) {
    let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
    let y = (299 * r + 587 * g + 114 * b) / 1000;
    let u = (-169 * r - 331 * g + 500 * b) / 1000 + 128;
    let v = (500 * r - 419 * g - 81 * b) / 1000 + 128;
    (y, u, v)
}

/// xBR pixel difference: |Δα| + |ΔY| + |ΔU| + |ΔV| (the libxbr `pixel_diff`).
#[inline]
fn df(a: [u8; 4], b: [u8; 4]) -> u32 {
    let (ay, au, av) = yuv(a);
    let (by, bu, bv) = yuv(b);
    ((a[3] as i32 - b[3] as i32).abs() + (ay - by).abs() + (au - bu).abs() + (av - bv).abs()) as u32
}

/// "Equal enough" — the libxbr threshold (`df < 155`).
#[inline]
fn eq(a: [u8; 4], b: [u8; 4]) -> bool {
    df(a, b) < 155
}

/// Blend `a` toward `b` by `n`/256 per channel (the `ALPHA_BLEND_n_W` weights: 128 =
/// 50%, 192 = ¾, 224 = ⅞, 64 = ¼).
#[inline]
fn blend(a: [u8; 4], b: [u8; 4], n: i32) -> [u8; 4] {
    std::array::from_fn(|k| {
        let (av, bv) = (a[k] as i32, b[k] as i32);
        (av + (bv - av) * n / 256).clamp(0, 255) as u8
    })
}

/// The xBR neighbourhood — the inner 3×3 (pa..pi, pe = centre) plus the 12 outer
/// pixels, on the 5×5-minus-corners grid:
/// ```text
///        a1 b1 c1
///     a0 pa pb pc c4
///     d0 pd pe pf f4
///     g0 pg ph pi i4
///        g5 h5 i5
/// ```
#[derive(Clone, Copy)]
#[rustfmt::skip]
struct Nb {
    pa: [u8;4], pb: [u8;4], pc: [u8;4],
    pd: [u8;4], pe: [u8;4], pf: [u8;4],
    pg: [u8;4], ph: [u8;4], pi: [u8;4],
    a0: [u8;4], a1: [u8;4], b1: [u8;4], c1: [u8;4], c4: [u8;4],
    d0: [u8;4], f4: [u8;4], g0: [u8;4], g5: [u8;4], h5: [u8;4], i4: [u8;4], i5: [u8;4],
}

impl Nb {
    /// Rotate the whole neighbourhood 90° clockwise (new(r,c) = old(4−c, r)). Applying
    /// the filter to each of the 4 rotations covers the 4 output quadrants.
    #[rustfmt::skip]
    fn rot_cw(&self) -> Nb {
        Nb {
            pa: self.pg, pb: self.pd, pc: self.pa,
            pd: self.ph, pe: self.pe, pf: self.pb,
            pg: self.pi, ph: self.pf, pi: self.pc,
            a1: self.g0, b1: self.d0, c1: self.a0, a0: self.g5, c4: self.a1,
            d0: self.h5, f4: self.b1, g0: self.i5, i4: self.c1, g5: self.i4, h5: self.f4, i5: self.c4,
        }
    }
}

/// One xBR-2× filter pass for the current orientation, blending into the 2×2 output
/// block `out` (indices 0=TL 1=TR 2=BL 3=BR) via `n` = the rotated (n0,n1,n2,n3). Only
/// the "main" corner `n3` (+ n1/n2 on a strong edge) are touched. Verbatim FILT2.
#[allow(clippy::nonminimal_bool)]
fn xbr_filt2(nb: &Nb, out: &mut [[u8; 4]; 4], n: [usize; 4]) {
    let (pe, pf, ph) = (nb.pe, nb.pf, nb.ph);
    if pe == ph || pe == pf {
        return;
    }
    let (pb, pc, pd, pg, pi) = (nb.pb, nb.pc, nb.pd, nb.pg, nb.pi);
    let (f4, h5, i4, i5) = (nb.f4, nb.h5, nb.i4, nb.i5);
    let e = df(pe, pc) + df(pe, pg) + df(pi, h5) + df(pi, f4) + (df(ph, pf) << 2);
    let i = df(ph, pd) + df(ph, i5) + df(pf, i4) + df(pf, pb) + (df(pe, pi) << 2);
    if e > i {
        return;
    }
    let px = if df(pe, pf) <= df(pe, ph) { pf } else { ph };
    let (n1, n2, n3) = (n[1], n[2], n[3]);
    let strong = e < i
        && ((!eq(pf, pb) && !eq(ph, pd))
            || (eq(pe, pi) && !eq(pf, i4) && !eq(ph, i5))
            || eq(pe, pg)
            || eq(pe, pc));
    if strong {
        let ke = df(pf, pg);
        let ki = df(ph, pc);
        let left = ke * 2 <= ki && pe != pg && pd != pg;
        let up = ke >= ki * 2 && pe != pc && pb != pc;
        if left && up {
            out[n3] = blend(out[n3], px, 224);
            out[n2] = blend(out[n2], px, 64);
            out[n1] = out[n2];
        } else if left {
            out[n3] = blend(out[n3], px, 192);
            out[n2] = blend(out[n2], px, 64);
        } else if up {
            out[n3] = blend(out[n3], px, 192);
            out[n1] = blend(out[n1], px, 64);
        } else {
            out[n3] = blend(out[n3], px, 128);
        }
    } else {
        out[n3] = blend(out[n3], px, 128);
    }
}

/// Read the 21-pixel xBR neighbourhood centred on `(x, y)` (edge-clamped).
#[rustfmt::skip]
fn read_nb(s: &Grid, x: isize, y: isize) -> Nb {
    Nb {
        pa: s.at(x-1, y-1), pb: s.at(x, y-1), pc: s.at(x+1, y-1),
        pd: s.at(x-1, y),   pe: s.at(x, y),   pf: s.at(x+1, y),
        pg: s.at(x-1, y+1), ph: s.at(x, y+1), pi: s.at(x+1, y+1),
        a1: s.at(x-1, y-2), b1: s.at(x, y-2), c1: s.at(x+1, y-2),
        a0: s.at(x-2, y-1), c4: s.at(x+2, y-1),
        d0: s.at(x-2, y),   f4: s.at(x+2, y),
        g0: s.at(x-2, y+1), i4: s.at(x+2, y+1),
        g5: s.at(x-1, y+2), h5: s.at(x, y+2), i5: s.at(x+1, y+2),
    }
}

fn xbr2x(s: &Grid) -> (Vec<u8>, usize, usize) {
    let (ow, oh) = (s.w * 2, s.h * 2);
    let mut out = vec![0u8; ow * oh * 4];
    // Output-index maps per 90° rotation (0=TL 1=TR 2=BL 3=BR); the "main" corner n3
    // rotates BR→TR→TL→BL, matching the libxbr call order.
    const MAPS: [[usize; 4]; 4] = [[0, 1, 2, 3], [2, 0, 3, 1], [3, 2, 1, 0], [1, 3, 0, 2]];
    for y in 0..s.h as isize {
        for x in 0..s.w as isize {
            let mut cell = [s.at(x, y); 4]; // TL, TR, BL, BR
            let mut cur = read_nb(s, x, y);
            for m in MAPS {
                xbr_filt2(&cur, &mut cell, m);
                cur = cur.rot_cw();
            }
            put_cell(&mut out, ow, x as usize, y as usize, 2, &cell);
        }
    }
    (out, ow, oh)
}

/// One xBR-3× filter pass, blending into the 3×3 output block `out` (row-major 0..8,
/// 8 = bottom-right = the "main" corner) via the rotated index map `n`. Verbatim FILT3.
#[allow(clippy::nonminimal_bool)]
fn xbr_filt3(nb: &Nb, out: &mut [[u8; 4]; 9], n: [usize; 9]) {
    let (pe, pf, ph) = (nb.pe, nb.pf, nb.ph);
    if pe == ph || pe == pf {
        return;
    }
    let (pb, pc, pd, pg, pi) = (nb.pb, nb.pc, nb.pd, nb.pg, nb.pi);
    let (f4, h5, i4, i5) = (nb.f4, nb.h5, nb.i4, nb.i5);
    let e = df(pe, pc) + df(pe, pg) + df(pi, h5) + df(pi, f4) + (df(ph, pf) << 2);
    let i = df(ph, pd) + df(ph, i5) + df(pf, i4) + df(pf, pb) + (df(pe, pi) << 2);
    if e > i {
        return;
    }
    let px = if df(pe, pf) <= df(pe, ph) { pf } else { ph };
    let strong = e < i
        && ((!eq(pf, pb) && !eq(pf, pc))
            || (!eq(ph, pd) && !eq(ph, pg))
            || (eq(pe, pi) && ((!eq(pf, f4) && !eq(pf, i4)) || (!eq(ph, h5) && !eq(ph, i5))))
            || eq(pe, pg)
            || eq(pe, pc));
    let (n2, n5, n6, n7, n8) = (n[2], n[5], n[6], n[7], n[8]);
    if strong {
        let ke = df(pf, pg);
        let ki = df(ph, pc);
        let left = ke * 2 <= ki && pe != pg && pd != pg;
        let up = ke >= ki * 2 && pe != pc && pb != pc;
        if left && up {
            out[n7] = blend(out[n7], px, 192);
            out[n6] = blend(out[n6], px, 64);
            out[n5] = out[n7];
            out[n2] = out[n6];
            out[n8] = px;
        } else if left {
            out[n7] = blend(out[n7], px, 192);
            out[n5] = blend(out[n5], px, 64);
            out[n6] = blend(out[n6], px, 64);
            out[n8] = px;
        } else if up {
            out[n5] = blend(out[n5], px, 192);
            out[n7] = blend(out[n7], px, 64);
            out[n2] = blend(out[n2], px, 64);
            out[n8] = px;
        } else {
            out[n8] = blend(out[n8], px, 224);
            out[n5] = blend(out[n5], px, 32);
            out[n7] = blend(out[n7], px, 32);
        }
    } else {
        out[n8] = blend(out[n8], px, 128);
    }
}

fn xbr3x(s: &Grid) -> (Vec<u8>, usize, usize) {
    let (ow, oh) = (s.w * 3, s.h * 3);
    let mut out = vec![0u8; ow * oh * 4];
    // 3×3 output-index maps per 90° rotation (row-major 0..8); main corner 8 → 2 → 0 → 6.
    const MAPS: [[usize; 9]; 4] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8],
        [6, 3, 0, 7, 4, 1, 8, 5, 2],
        [8, 7, 6, 5, 4, 3, 2, 1, 0],
        [2, 5, 8, 1, 4, 7, 0, 3, 6],
    ];
    for y in 0..s.h as isize {
        for x in 0..s.w as isize {
            let mut cell = [s.at(x, y); 9];
            let mut cur = read_nb(s, x, y);
            for m in MAPS {
                xbr_filt3(&cur, &mut cell, m);
                cur = cur.rot_cw();
            }
            put_cell(&mut out, ow, x as usize, y as usize, 3, &cell);
        }
    }
    (out, ow, oh)
}

/// xBR 4× — chain xBR 2× twice (a genuinely smooth 4×; libxbr's native FILT4 isn't
/// transcribed here). Each pass re-detects edges on the already-doubled image.
fn xbr4x(s: &Grid) -> (Vec<u8>, usize, usize) {
    let (two, w2, h2) = xbr2x(s);
    xbr2x(&Grid {
        rgba: &two,
        w: w2,
        h: h2,
    })
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
        // Every non-None scaler enlarges a 4×4 by its factor and keeps a flat field flat.
        let flat = buf(&[128; 16], 4);
        for sc in Scaler::ALL {
            if sc == Scaler::None {
                continue;
            }
            let f = sc.factor();
            let (out, ow, oh) = sc.apply(&flat, 4, 4);
            assert_eq!((ow, oh), (4 * f, 4 * f), "{:?} size", sc);
            assert_eq!(out.len(), ow * oh * 4, "{:?} buffer len", sc);
            assert!(
                out.chunks_exact(4).all(|p| p[0] == 128),
                "{:?} keeps a flat field flat",
                sc
            );
        }
        // None is a passthrough.
        let src = buf(&[0, 255, 255, 0], 2);
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
    fn eagle3x_cross_stays_center() {
        // Distinct centre + neighbours → the 3×3 block's cross (edges + centre) must all
        // be the centre; only corners may differ. Regression for the cross leaking
        // neighbour colours (the edge-fringe bug).
        let src = buf(&[1, 2, 3, 4, 5, 6, 7, 8, 9], 3); // all distinct grays
        let (out, ow, _) = Scaler::Eagle3x.apply(&src, 3, 3);
        let at = |x: usize, y: usize| out[(y * ow + x) * 4];
        // Source centre (1,1)=5 → output block rows 3..6, cols 3..6.
        assert_eq!(at(4, 3), 5, "top-middle = centre");
        assert_eq!(at(3, 4), 5, "middle-left = centre");
        assert_eq!(at(4, 4), 5, "centre = centre");
        assert_eq!(at(5, 4), 5, "middle-right = centre");
        assert_eq!(at(4, 5), 5, "bottom-middle = centre");
    }

    #[test]
    fn xbr2x_size_flat_and_blends() {
        // Correct 2× size, and a flat field must stay perfectly flat (the filter early-
        // outs when the centre equals its neighbours — no phantom edges).
        let (out, ow, oh) = Scaler::Xbr2x.apply(&buf(&[128; 9], 3), 3, 3);
        assert_eq!((ow, oh), (6, 6));
        assert!(out.chunks_exact(4).all(|p| p[0] == 128), "flat stays flat");
        // A hard diagonal edge should yield at least one *blended* (intermediate) pixel,
        // which is what makes xBR smooth rather than blocky.
        //   W W W . .
        //   W W . . .
        //   W . . . .   (5×5 upper-left white triangle, centre is on the edge)
        let mut v = vec![0u8; 25];
        for y in 0..5 {
            for x in 0..5 {
                if x + y < 3 {
                    v[y * 5 + x] = 255;
                }
            }
        }
        let (out, _, _) = Scaler::Xbr2x.apply(&buf(&v, 5), 5, 5);
        assert!(
            out.chunks_exact(4).any(|p| p[0] > 0 && p[0] < 255),
            "xBR produced a blended edge pixel"
        );
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
