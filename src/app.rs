use crate::colo_thumb::RemoteThumbs;
use crate::decode::Registry;
use crate::thumb::ThumbBuilder;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
// `OsString` is only referenced by the freedesktop trash paths, which are
// configured out on macOS.
#[cfg(not(target_os = "macos"))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

const THUMB_PX: u32 = 512; // max thumbnail dimension; smaller art is kept at source res
const DEFAULT_TILE: f32 = 140.0; // default tile box (square) in points — the "100%" thumbnail size
const MIN_TILE: f32 = 56.0;
const MAX_TILE: f32 = 640.0; // zoom thumbnails further than Gwenview, toward Dolphin
const GAP: f32 = 10.0; // default horizontal gap between grid tiles, in points
const GAP_Y: f32 = 10.0; // default vertical gap between grid rows, in points

// Grid tile caption fields (request: choose what's shown under each thumbnail).
// Stored as a u16 bitmask so persistence is a plain integer.
const CAP_NAME: u16 = 1 << 0;
const CAP_KIND: u16 = 1 << 1;
const CAP_SIZE: u16 = 1 << 2;
const CAP_RATING: u16 = 1 << 3;
const CAP_COLORS: u16 = 1 << 4;
const CAP_DIMENSIONS: u16 = 1 << 5;
const CAPTION_FIELDS: &[(u16, &str)] = &[
    (CAP_NAME, "Filename"),
    (CAP_KIND, "File type"),
    (CAP_SIZE, "Size"),
    (CAP_RATING, "Rating"),
    (CAP_COLORS, "Colors"),
    (CAP_DIMENSIONS, "Dimensions"),
];

// Table view: which *optional* columns show in the file (local-disk) layout. The
// thumbnail + Name columns are always shown; the rest are user-toggled (Preferences →
// "Table columns"). Stored as a u16 bitmask so persistence is a plain integer. (The
// 16colo scene layout — artist/year/group/pack — is a fixed set, not configurable.)
const TC_TYPE: u16 = 1 << 0;
const TC_SIZE: u16 = 1 << 1;
const TC_DIMENSIONS: u16 = 1 << 2;
const TC_COLORS: u16 = 1 << 3;
const TC_RATING: u16 = 1 << 4;
const TC_MODIFIED: u16 = 1 << 5;
const TC_CREATED: u16 = 1 << 6;
const TABLE_COLUMNS: &[(u16, &str)] = &[
    (TC_TYPE, "Type"),
    (TC_SIZE, "Size"),
    (TC_DIMENSIONS, "Dimensions"),
    (TC_COLORS, "Colors"),
    (TC_RATING, "Rating"),
    (TC_MODIFIED, "Modified"),
    (TC_CREATED, "Created"),
];
// Default mirrors the original fixed column set (Created off).
const TABLE_COLUMNS_DEFAULT: u16 =
    TC_TYPE | TC_SIZE | TC_DIMENSIONS | TC_COLORS | TC_RATING | TC_MODIFIED;

// Which optional columns show in the 16colo scene layout (Filename + Download are
// always shown). Toggled from the header right-click menu, persisted as a bitmask.
const CS_ARTIST: u16 = 1 << 0;
const CS_TYPE: u16 = 1 << 1;
const CS_YEAR: u16 = 1 << 2;
const CS_GROUP: u16 = 1 << 3;
const CS_PACK: u16 = 1 << 4;
const CS_RATING: u16 = 1 << 5;
const COLO_COLUMNS: &[(u16, &str)] = &[
    (CS_ARTIST, "Artist"),
    (CS_TYPE, "Type"),
    (CS_YEAR, "Year"),
    (CS_GROUP, "Group"),
    (CS_PACK, "Pack"),
    (CS_RATING, "Rating"),
];
const COLO_COLUMNS_DEFAULT: u16 = CS_ARTIST | CS_TYPE | CS_YEAR | CS_GROUP | CS_PACK | CS_RATING;

/// A table column's identity — drives both its content and its sort key. Kept separate
/// from the on/off bitmask so the render + sort logic key off meaning, not position.
#[derive(Clone, Copy, PartialEq)]
enum ColKind {
    Thumb,
    Name, // filename in both layouts
    Type,
    Size,
    Dims,
    Colors,
    Rating,
    Modified,
    Created,
    Artist, // scene-only
    Year,
    Group,
    Pack,
    Download,
}
const CAP_LINE_H: f32 = 15.0; // height of one caption line, in points

// GPL palette library (palette-swap preview). The default dir is the user's
// collection; it's persisted and can point elsewhere.
// Optional *extra* palette source, resolved under `$HOME` so it isn't a dead
// absolute path off the author's machine. The bundled palettes (`palettes_builtin`)
// always load regardless; anything here is added on top.
const DEFAULT_PALETTE_SUBDIR: &str = "git/DRAW/ASSETS/PALETTES";
const DEFAULT_PALETTE_FAVS: &[&str] = &[
    "EGA (16).GPL",
    "VGA (256).GPL",
    "CGA0-HIGH (4).GPL",
    "C=64 (16).GPL",
    "ANSI32 (32).GPL",
];

#[derive(PartialEq)]
enum Mode {
    Grid,
    Single,
}

/// One cell in the grid: either a subdirectory or an image file.
#[derive(Clone)]
struct Entry {
    path: PathBuf,
    is_dir: bool,
    is_archive: bool, // a browsable archive (.zip/.lha/…) — navigated into like a folder
    size: u64,
    mtime: Option<SystemTime>,
    ctime: Option<SystemTime>,
    rating: u8, // 0..=5 stars, read from the `user.baloo.rating` xattr
}

/// An archive currently mounted as a virtual folder: the real archive file (its
/// display identity) and the temp directory its contents were extracted into.
/// `to_display`/`real_path` translate between the two so breadcrumbs read
/// `…/pack.zip/sub` while I/O still targets the real temp files.
struct ArchiveMount {
    archive: PathBuf,
    temp_root: PathBuf,
}

/// A result delivered from a background 16colo.rs fetch/download thread.
enum RemoteMsg {
    /// Packs for a year arrived: (the virtual year path, pack entries, name→url map).
    Packs(PathBuf, Vec<Entry>, HashMap<PathBuf, String>),
    /// A pack zip finished downloading: (the virtual pack path, the local zip path).
    PackZip(PathBuf, PathBuf),
    Err(String),
}

/// A screensaver random-pack pick: `(year, pack name, download URL)` or an error.
type RandomPick = Result<(u32, String, String), String>;

/// A streamed result from a 16colo.rs flat-piece listing (artist/group/search). Mirrors
/// `SearchMsg`: one `Hit` per piece as it's discovered, then `Done(total)`. Each hit
/// carries the piece's `Entry` (virtual path) plus its [`ColoPiece`] metadata.
enum ColoMsg {
    Hit(Entry, Box<ColoPiece>), // boxed: ColoPiece dwarfs the other variants
    Done(usize),
    Err(String),
}

/// What a 16colo.rs flat-piece listing is built from (see
/// [`PixelView::start_colo_pieces`]): an artist, a group, or a server-side search.
#[derive(Clone)]
enum ColoSource {
    Artist(String),
    Group(String),
    Search(String),        // substring across artists *and* groups
    SearchArtists(String), // substring across artist names only
    SearchGroups(String),  // substring across group names only
}

/// A virtual (non-on-disk) directory `Entry`, used for the 16colo.rs year/pack tree.
fn virtual_dir(path: PathBuf) -> Entry {
    Entry {
        path,
        is_dir: true,
        is_archive: false,
        size: 0,
        mtime: None,
        ctime: None,
        rating: 0,
    }
}

/// Decoded-image metadata, filled in lazily by the thumbnail worker.
#[derive(Clone, Copy)]
struct ImgMeta {
    w: u32,
    h: u32,
    colors: Option<usize>,
}

/// Per-piece 16colo.rs metadata, keyed by the piece's virtual display path
/// (`<16colo.rs>/<year>/<pack>/<FILE>`). Populated when an artist/group/search view
/// is flattened into individual pieces (see [`PixelView::start_colo_pieces`]); it
/// drives the table's scene columns, the scene sort keys, and the per-row download
/// actions. The thumbnail comes from `tn_url` (16colo's pre-rendered PNG), and
/// opening a piece downloads `raw_url` (the single file) rather than its whole pack.
#[derive(Clone)]
struct ColoPiece {
    artist: String,
    group: String,
    year: u32,
    pack: String,
    raw_url: String,                    // single-file download (the .ans/.png itself)
    tn_url: String,                     // pre-rendered thumbnail PNG
    sauce: Option<crate::sauce::Sauce>, // from the 16colo API (pack endpoint), if any
}

/// An animated GIF being viewed: uploaded frame textures + playback timing.
struct AnimState {
    path: PathBuf,
    frames: Vec<egui::TextureHandle>,
    delays_ms: Vec<u16>,
    size: [usize; 2],
    current: usize,
    playing: bool,
    acc_ms: f32, // elapsed time accumulated toward the current frame's delay
}

/// Simulated modem baud rate for "type-out" / "watch-it-draw" playback of text-mode
/// (ANSI/ANSImation) and RIP art. `None` renders instantly (no animation).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Baud {
    None,
    B300,
    B1200,
    B2400,
    B4800,
    B9600,
    B14400,
    B28800,
    B33600,
    B56000,
    B115200,
}

impl Baud {
    const ALL: [Baud; 11] = [
        Baud::None,
        Baud::B300,
        Baud::B1200,
        Baud::B2400,
        Baud::B4800,
        Baud::B9600,
        Baud::B14400,
        Baud::B28800,
        Baud::B33600,
        Baud::B56000,
        Baud::B115200,
    ];

    fn label(self) -> &'static str {
        match self {
            Baud::None => "None",
            Baud::B300 => "300",
            Baud::B1200 => "1200",
            Baud::B2400 => "2400",
            Baud::B4800 => "4800",
            Baud::B9600 => "9600",
            Baud::B14400 => "14.4k",
            Baud::B28800 => "28.8k",
            Baud::B33600 => "33.6k",
            Baud::B56000 => "56k",
            Baud::B115200 => "115.2k",
        }
    }

    /// Characters per second. 8N1 framing = 10 bits/char (cps = baud/10), but real BBS
    /// modems (V.42bis, the 14.4k era) compressed ANSI text roughly 4:1, so the screen
    /// filled far faster than the raw rate — which is why 14.4k "felt fast" on a real
    /// board. Model it: ≤2400 crawl uncompressed, ramping to ~4× by 14.4k+. `None` =
    /// instant.
    fn cps(self) -> Option<f32> {
        let baud: f32 = match self {
            Baud::None => return None,
            Baud::B300 => 300.0,
            Baud::B1200 => 1200.0,
            Baud::B2400 => 2400.0,
            Baud::B4800 => 4800.0,
            Baud::B9600 => 9600.0,
            Baud::B14400 => 14400.0,
            Baud::B28800 => 28800.0,
            Baud::B33600 => 33600.0,
            Baud::B56000 => 56000.0,
            Baud::B115200 => 115200.0,
        };
        let compression = (baud / 3600.0).clamp(1.0, 4.0);
        Some(baud / 10.0 * compression)
    }

    fn to_u8(self) -> u8 {
        Baud::ALL.iter().position(|&b| b == self).unwrap_or(0) as u8
    }
    fn from_u8(v: u8) -> Baud {
        Baud::ALL.get(v as usize).copied().unwrap_or(Baud::None)
    }
}

/// A progressive renderable for baud playback. `Text`/`Rip` render the first `n`
/// *bytes* of a character/command stream; `Cells` reveals the first `n` *cells* of an
/// already-decoded image — the binary scene formats (XBin/BIN/Tundra/IDF/ADF/PETSCII)
/// aren't byte streams (RLE/headers/embedded font+palette), so they "type out" by
/// revealing the decoded grid cell-by-cell instead.
enum Stream {
    Text(crate::decode::TextStream),
    Rip(crate::decode::RipStream),
    Cells(CellReveal),
}

/// Progressive cell reveal of an already-decoded text-mode image: paint the first `n`
/// cells (reading order, top→bottom, left→right) over a black background. This is the
/// baud "typeout" for the binary scene formats, which can't be rendered from a byte
/// prefix. Cell size is the format's glyph box (8×16, or 8×8 for the C64/PETSCII font).
struct CellReveal {
    pixels: Vec<[u8; 4]>, // the full decoded image, row-major
    w: usize,
    h: usize,
    cell_w: usize,
    cell_h: usize,
    cols: usize,
    cells: usize, // total cell count (cols × rows)
}

impl CellReveal {
    fn new(pixels: Vec<[u8; 4]>, w: usize, h: usize, cell_w: usize, cell_h: usize) -> CellReveal {
        let cols = (w / cell_w.max(1)).max(1);
        let rows = (h / cell_h.max(1)).max(1);
        CellReveal {
            pixels,
            w,
            h,
            cell_w,
            cell_h,
            cols,
            cells: cols * rows,
        }
    }

    /// The frame with the first `limit` cells revealed (rest black), plus the revealed
    /// bottom in source pixels (for BBS-style auto-scroll).
    fn render(&self, limit: usize) -> (crate::image_types::PixImage, u32) {
        let lim = limit.min(self.cells);
        let full_rows = lim / self.cols;
        let partial = lim % self.cols;
        let mut out = vec![[0u8, 0, 0, 255]; self.w * self.h];
        let rows_px = (full_rows * self.cell_h).min(self.h);
        out[..rows_px * self.w].copy_from_slice(&self.pixels[..rows_px * self.w]);
        // The partially-typed current row: reveal its first `partial` cells.
        if partial > 0 && rows_px < self.h {
            let pw = (partial * self.cell_w).min(self.w);
            let y1 = (rows_px + self.cell_h).min(self.h);
            for y in rows_px..y1 {
                let base = y * self.w;
                out[base..base + pw].copy_from_slice(&self.pixels[base..base + pw]);
            }
        }
        let cursor_px = ((full_rows + 1) * self.cell_h).min(self.h) as u32;
        (
            crate::image_types::PixImage::from_rgba(self.w as u32, self.h as u32, out),
            cursor_px,
        )
    }
}

/// The glyph-box (cell) size for a text-mode file: 8×8 for the C64/PETSCII font,
/// otherwise the standard 8×16 VGA cell. Used to drive [`CellReveal`].
fn textmode_cell(path: &Path) -> (usize, usize) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("seq" | "pet" | "petscii" | "petmate") => (8, 8),
        _ => (8, 16),
    }
}

impl Stream {
    /// Build a stream for byte-streamable art — RIPscript, or an ANSI/CP437 stream
    /// (the kind a modem actually typed out). Binary text-mode formats (XBin/BIN/IDF/
    /// ADF/Tundra/PETSCII) aren't character streams, so they get no player. None means
    /// "not stream-playable" → the static path handles it.
    fn for_file(bytes: &[u8], path: &Path) -> Option<Stream> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        let ext = ext.as_deref();
        if ext == Some("rip") || bytes.starts_with(b"!|") {
            Some(Stream::Rip(crate::decode::RipStream::new(bytes)))
        } else if matches!(
            ext,
            Some("ans" | "asc" | "nfo" | "diz" | "ice" | "cia" | "txt") | None
        ) {
            crate::decode::TextStream::new(bytes).map(Stream::Text)
        } else {
            None
        }
    }
    fn len(&self) -> usize {
        match self {
            Stream::Text(s) => s.len(),
            Stream::Rip(s) => s.len(),
            Stream::Cells(c) => c.cells,
        }
    }
    fn is_rip(&self) -> bool {
        matches!(self, Stream::Rip(_))
    }
    /// The frame at `limit` bytes, plus the content's current bottom in source pixels
    /// (the typing cursor) for BBS-style auto-scroll. RIP draws all over a fixed screen,
    /// so it reports 0 = "no auto-scroll".
    fn render(&self, limit: usize) -> (crate::image_types::PixImage, u32) {
        match self {
            Stream::Text(s) => s.render_frame(limit),
            Stream::Rip(s) => (s.render(limit), 0),
            Stream::Cells(c) => c.render(limit),
        }
    }
}

/// Baud-rate playback state for the open text/RIP art: a byte cursor advanced at the
/// simulated rate, with the current frame cached so it only re-renders when it moves.
struct Player {
    path: PathBuf,
    stream: Stream,
    len: usize,
    pos: usize, // current byte position (0..=len)
    playing: bool,
    acc: f32,                           // fractional bytes accumulated toward `pos`
    cursor_px: u32,                     // content bottom in source px at `pos` (0 = no auto-scroll)
    tex: Option<(usize, TiledTexture)>, // cached (pos, frame)
}

impl Player {
    fn new(path: PathBuf, stream: Stream, autoplay: bool) -> Player {
        let len = stream.len();
        Player {
            path,
            stream,
            len,
            pos: if autoplay { 0 } else { len },
            playing: autoplay,
            acc: 0.0,
            cursor_px: 0,
            tex: None,
        }
    }

    /// Advance the cursor by `dt` seconds at `baud`. Returns true while still playing.
    fn advance(&mut self, baud: Baud, dt: f32) {
        if !self.playing {
            return;
        }
        match baud.cps() {
            Some(cps) => {
                self.acc += cps * dt;
                let step = self.acc.floor().max(0.0) as usize;
                self.acc -= step as f32;
                self.pos = (self.pos + step).min(self.len);
            }
            None => self.pos = self.len, // instant
        }
        if self.pos >= self.len {
            self.playing = false;
        }
    }

    /// The frame texture at the current position, rendered + uploaded on demand and
    /// cached by position (so a paused/static player costs nothing).
    fn frame(&mut self, ctx: &egui::Context) -> TiledTexture {
        if self.tex.as_ref().map(|(p, _)| *p) != Some(self.pos) {
            let (img, cursor_px) = self.stream.render(self.pos);
            self.cursor_px = cursor_px;
            let size = [img.width as usize, img.height as usize];
            let tex = TiledTexture::from_rgba(
                ctx,
                &format!("{}#baud", self.path.to_string_lossy()),
                size,
                &img.rgba_bytes(),
                egui::TextureOptions::NEAREST_REPEAT,
            );
            self.tex = Some((self.pos, tex));
        }
        self.tex.as_ref().unwrap().1.clone()
    }
}

/// What the grid is sorted by (Phase 5).
#[derive(Clone, Copy, PartialEq, Debug)]
enum SortKey {
    Name,
    Type,
    Modified,
    Created,
    Size,
    Rating,
    Colors,
    Dimensions,
    // 16colo.rs flat-listing columns — sort by the `colo_pieces` metadata map. They
    // never reach the sortbar combo (`COMMON`); only the table's scene headers set them.
    Artist,
    Group,
    Year,
    Pack,
}

impl SortKey {
    // New keys are appended so persisted indices (`to_u8`) stay valid across upgrades.
    const ALL: [SortKey; 12] = [
        SortKey::Name,
        SortKey::Type,
        SortKey::Modified,
        SortKey::Created,
        SortKey::Size,
        SortKey::Rating,
        SortKey::Colors,
        SortKey::Dimensions,
        SortKey::Artist,
        SortKey::Group,
        SortKey::Year,
        SortKey::Pack,
    ];
    /// The keys offered in the sort-bar combo (the scene-only keys are excluded —
    /// they're only meaningful in a 16colo.rs flat listing and set via the table).
    const COMMON: [SortKey; 8] = [
        SortKey::Name,
        SortKey::Type,
        SortKey::Modified,
        SortKey::Created,
        SortKey::Size,
        SortKey::Rating,
        SortKey::Colors,
        SortKey::Dimensions,
    ];
    fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Type => "Type",
            SortKey::Modified => "Modified",
            SortKey::Created => "Created",
            SortKey::Size => "Size",
            SortKey::Rating => "Rating",
            SortKey::Colors => "Colors",
            SortKey::Dimensions => "Dimensions",
            SortKey::Artist => "Artist",
            SortKey::Group => "Group",
            SortKey::Year => "Year",
            SortKey::Pack => "Pack",
        }
    }
    fn to_u8(self) -> u8 {
        SortKey::ALL.iter().position(|&k| k == self).unwrap_or(0) as u8
    }
    fn from_u8(v: u8) -> SortKey {
        SortKey::ALL
            .get(v as usize)
            .copied()
            .unwrap_or(SortKey::Name)
    }
}

/// A deferred menu-bar action, applied after the menu closure returns (so the
/// nested menu closures never need to borrow `self` mutably).
enum MenuAction {
    Open,
    Quit,
    ToggleExplorer,
    ToggleDetails,
    ToggleRecolor,
    ToggleTable,
    Up,
    Home,
    Nav(PathBuf),
    Sort(SortKey),
    ToggleDesc,
    ToggleDirsFirst,
    ResetThumb,
    Hotkeys,
    Prefs,
    File(FileAction),
    Undo,
    Search,
}

/// The advanced-search form: every field is optional (empty = no constraint). The
/// numeric bounds are kept as text so a blank box means "unbounded".
#[derive(Clone, Default)]
struct SearchSpec {
    name: String, // filename contains (case-insensitive)
    ext: String,  // comma/space-separated extensions; empty = any type
    wmin: String, // pixel-dimension bounds
    wmax: String,
    hmin: String,
    hmax: String,
    sauce: String, // matches any of SAUCE title/author/group/font
    smin: String,  // file-size bounds, in KB
    smax: String,
    dfrom: String, // modified-date bounds, YYYY-MM-DD (inclusive)
    dto: String,
    rmin: String, // minimum star rating (0..=5)
}

impl SearchSpec {
    fn all_fields(&self) -> [&String; 12] {
        [
            &self.name,
            &self.ext,
            &self.wmin,
            &self.wmax,
            &self.hmin,
            &self.hmax,
            &self.sauce,
            &self.smin,
            &self.smax,
            &self.dfrom,
            &self.dto,
            &self.rmin,
        ]
    }

    fn is_blank(&self) -> bool {
        self.all_fields().iter().all(|s| s.trim().is_empty())
    }

    /// Flatten to the persisted field order (extend with new fields at the END so old
    /// saved filters still load — `from_record` defaults any missing trailing field).
    fn record(&self) -> Vec<String> {
        self.all_fields().iter().map(|s| (*s).clone()).collect()
    }

    fn from_record(r: &[String]) -> Self {
        let g = |i: usize| r.get(i).cloned().unwrap_or_default();
        SearchSpec {
            name: g(0),
            ext: g(1),
            wmin: g(2),
            wmax: g(3),
            hmin: g(4),
            hmax: g(5),
            sauce: g(6),
            smin: g(7),
            smax: g(8),
            dfrom: g(9),
            dto: g(10),
            rmin: g(11),
        }
    }

    /// A short human label of the active criteria, e.g. `dragon · *.ans · sauce:acid`.
    fn summary(&self) -> String {
        let rng = |lo: &str, hi: &str, label: &str, unit: &str| -> Option<String> {
            match (lo.trim(), hi.trim()) {
                ("", "") => None,
                (lo, "") => Some(format!("{label}≥{lo}{unit}")),
                ("", hi) => Some(format!("{label}≤{hi}{unit}")),
                (lo, hi) => Some(format!("{label}{lo}-{hi}{unit}")),
            }
        };
        let mut parts: Vec<String> = Vec::new();
        if !self.name.trim().is_empty() {
            parts.push(self.name.trim().to_string());
        }
        if !self.ext.trim().is_empty() {
            parts.push(format!("*.{}", self.ext.trim()));
        }
        parts.extend(rng(&self.wmin, &self.wmax, "W", ""));
        parts.extend(rng(&self.hmin, &self.hmax, "H", ""));
        parts.extend(rng(&self.smin, &self.smax, "", "KB"));
        match (self.dfrom.trim(), self.dto.trim()) {
            ("", "") => {}
            (a, b) if a == b => parts.push(a.to_string()),
            (a, "") => parts.push(format!("≥{a}")),
            ("", b) => parts.push(format!("≤{b}")),
            (a, b) => parts.push(format!("{a}..{b}")),
        }
        if let Some(r) = parse_dim(&self.rmin).filter(|&r| r > 0) {
            parts.push(format!("{}★+", r.min(5)));
        }
        if !self.sauce.trim().is_empty() {
            parts.push(format!("sauce:{}", self.sauce.trim()));
        }
        if parts.is_empty() {
            "filter".into()
        } else {
            parts.join(" · ")
        }
    }
}

/// A result delivered from the background search thread.
enum SearchMsg {
    Hit(Entry),  // a matching image file
    Done(usize), // search finished; total matches
}

/// "Smart filter on…" — seed a recursive search from one attribute of a clicked file.
#[derive(Clone, Copy)]
enum SmartCriterion {
    Type,
    Name,
    Size,
    Date,
    Rating,
    Group,
    Artist,
}

/// A rebindable navigation action (round-2 #12 groundwork). Ratings (1-5/0),
/// clicks, and zoom stay fixed for now; this is the structure to grow.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Action {
    PrevImage,
    NextImage,
    BackToGrid,
    ParentDir,
    ToggleView,
}

impl Action {
    // New actions are appended so persisted keymap indices (`to_u8`) stay valid.
    const ALL: [Action; 5] = [
        Action::PrevImage,
        Action::NextImage,
        Action::BackToGrid,
        Action::ParentDir,
        Action::ToggleView,
    ];
    fn label(self) -> &'static str {
        match self {
            Action::PrevImage => "Previous image",
            Action::NextImage => "Next image",
            Action::BackToGrid => "Back to grid",
            Action::ParentDir => "Parent folder",
            Action::ToggleView => "Toggle grid / table view",
        }
    }
    fn default_key(self) -> egui::Key {
        match self {
            Action::PrevImage => egui::Key::ArrowLeft,
            Action::NextImage => egui::Key::ArrowRight,
            Action::BackToGrid => egui::Key::Escape,
            Action::ParentDir => egui::Key::Backspace,
            Action::ToggleView => egui::Key::T,
        }
    }
    fn to_u8(self) -> u8 {
        Action::ALL.iter().position(|&a| a == self).unwrap_or(0) as u8
    }
}

/// A reversible file operation, for in-app Undo (Ctrl+Z).
enum UndoOp {
    // Only constructed on platforms with `trash::os_limited` (not macOS); see the
    // cfg guards in `delete_selection`.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Trash(Vec<trash::TrashItem>), // restore from the system trash
    Move(Vec<(PathBuf, PathBuf)>), // (current location, original) — move each back
    NewFolder(PathBuf),            // remove the (empty) created folder
    PasteCopy(Vec<PathBuf>),       // trash the copies that paste created
}

/// A file operation requested from a grid context menu (applied after the scroll
/// area, to avoid borrowing `self` inside the row closure).
#[derive(Clone, Copy)]
enum FileAction {
    Copy,
    Cut,
    Paste,
    Rename,
    Delete,
    NewFolder,
}

pub struct PixelView {
    registry: Arc<Registry>,
    thumbs: ThumbBuilder,

    folder: Option<PathBuf>,
    all_entries: Vec<Entry>, // raw scan; `entries` is the filtered + sorted view of it
    entries: Vec<Entry>,
    thumb_tex: HashMap<PathBuf, egui::TextureHandle>,
    img_meta: HashMap<PathBuf, ImgMeta>,
    sauce_cache: HashMap<PathBuf, Option<crate::sauce::Sauce>>, // parsed SAUCE per path
    palettes: HashMap<PathBuf, Option<Vec<[u8; 4]>>>, // details swatches; None = too many colors
    thumb_rgba: HashMap<PathBuf, (usize, usize, Vec<u8>)>, // thumb CPU pixels (for grid recolor)
    grid_recolor: HashMap<PathBuf, (String, egui::TextureHandle)>, // recolored grid tiles, per recolor key
    folder_info: HashMap<PathBuf, FolderInfo>, // cached montage previews + counts per folder

    mode: Mode,
    selected: usize,
    selection: HashSet<PathBuf>, // keyed by path so it survives re-sorting/filtering
    anchor: Option<usize>,
    hovered: Option<usize>,

    // sort & filter (Phase 5)
    sort_key: SortKey,
    sort_desc: bool,
    dirs_first: bool,
    min_rating: u8,
    // Cross-platform ratings sidecar — the source of truth for virtual art (inside
    // archives / 16colo.rs) that can't carry a `user.baloo.rating` xattr.
    ratings: crate::ratings::RatingStore,
    // View history (visited state + counts + last-viewed), SQLite-backed. None if the
    // db couldn't be opened — the app then runs with view-tracking disabled.
    viewdb: Option<crate::viewdb::ViewDb>,

    // chrome (Phase 6 + round 2)
    show_explorer: bool,
    show_details: bool,
    show_recolor: bool,                      // the Recolor dock pane
    recolor_grid: bool,                      // apply the active recolor to grid thumbnails too
    last_inspected: Option<PathBuf>,         // sticky target for details/recolor in grid mode
    custom_palette: Option<Vec<[u8; 4]>>,    // a generated/edited palette (e.g. Random)
    flash: Option<[u8; 4]>,                  // swatch held down: highlight this color
    editing_color: Option<(usize, [u8; 4])>, // swatch right-click: editing palette[idx]
    adjust: Adjust,                          // tone adjustments applied before the palette map
    explorer_filter: String,                 // folder-tree search box (runtime only)
    colo_search: String,                     // 16colo.rs nav-bar search box (runtime only)
    explorer_tab: u8,                        // 0 = Places, 1 = Folders
    places_tab: u8,                          // Places sub-tab: 0 = Local, 1 = 16colo.rs
    show_hotkeys: bool,
    show_prefs: bool,

    // preferences (round 2 #10)
    theme: u8,                                              // 0 = dark, 1 = light
    grid_gap: f32,           // horizontal spacing between grid tiles, in points
    grid_gap_y: f32,         // vertical spacing between grid rows, in points
    caption_fields: u16,     // bitmask: what to show under each grid thumbnail
    quantize_on: bool,       // details palette: reduce to N colors (median cut)
    quantize_n: usize,       // target color count when reducing
    dither_method: u8,       // index into thumb::DITHER_NAMES (0 = none)
    dither_amount: f32,      // 0..1 dither strength
    dither_custom: Vec<u32>, // custom ordered-dither threshold matrix (row-major)
    dither_custom_n: usize,  // custom matrix dimension (n×n; 2/4/8)
    balance_color: [u8; 3],  // color-balance op: ±offset color (128 = neutral)
    balance_strength: f32,   // color-balance op: 0..1 amount
    balance_hex: String,     // live buffer for the hex paste field
    quantize_cache: Option<(PathBuf, usize, Vec<[u8; 4]>)>, // memoized reduction
    // GPL palette library (palette-swap preview).
    palette_dir: PathBuf,                            // where the .gpl files live
    palette_files: Vec<PathBuf>,                     // scanned *.gpl in palette_dir
    palette_favorites: Vec<PathBuf>,                 // pinned palettes (quick buttons)
    selected_palette: Option<PathBuf>,               // active swap palette (None = Reduce/Original)
    loaded_palettes: HashMap<PathBuf, Vec<[u8; 4]>>, // parsed .gpl cache
    // Recolored preview for the details thumbnail: decoded source pixels (per path)
    // + the remapped texture (keyed by a recolor string), so it updates live.
    preview_src: Option<(PathBuf, usize, usize, Vec<u8>)>,
    preview_tex: Option<(PathBuf, String, egui::TextureHandle)>,
    // hotkeys (round 2 #12)
    keymap: HashMap<Action, egui::Key>,
    rebinding: Option<Action>, // the action awaiting a new key, if any

    // single-view state
    full_tex: Option<(PathBuf, TiledTexture)>,
    full_src: Option<(PathBuf, [usize; 2], Vec<u8>)>, // decoded full pixels (for reduction)
    full_reduced: Option<(PathBuf, String, TiledTexture)>, // remapped, per (path,recolor)
    anim: Option<AnimState>,                          // Some when viewing an animated GIF
    hover_anim: Option<AnimState>,                    // the hovered grid GIF, playing in its tile
    player: Option<Player>, // baud-rate playback for the open text/RIP art (ANSImation)
    zoom: f32,
    zoom_lock: bool, // viewer: snap zoom to 100% steps (¼ steps below 100%)
    offset: egui::Vec2,
    view_to_top: bool, // viewer: a freshly opened image starts at its top-left (set on
    // load, applied once its pixel size is known in draw_image_view)
    fit_width_on_open: bool, // viewer: cap a freshly opened text-mode image's zoom so its
    // full width fits the viewport (wide ANSIs don't clip)
    nav_drag: bool, // viewer: the active drag began on the navigator minimap, so it
    // stays in navigator mode even if the cursor leaves the strip
    // viewer: a crisp minimap built from the full-res pixels at the strip's device
    // resolution (path, [w,h] device px, texture), rebuilt on image/size change.
    minimap: Option<(PathBuf, [usize; 2], egui::TextureHandle)>,
    // viewer: during baud playback, the typing cursor's source-Y (px) to keep at the
    // bottom of the viewport (BBS-style auto-scroll); None when not playing/scrolling.
    play_autoscroll: Option<f32>,
    tile_mode: bool, // viewer: tile the image across the whole window (texture testing)
    fit_requested: bool, // viewer: apply fit-to-window on the next draw
    fit_mode: bool,  // viewer: sticky — re-fit every newly loaded image (persisted)
    // Text-mode / scene art (ANSI/XBin/BIN/…) renders at a tiny native 8×16 px per
    // cell, so a true 1:1 (100%) view is unreadably small. We give it its own
    // remembered zoom (default 3×, the "real text screen" feel) kept independent of
    // the raster-image zoom, plus an optional non-square CRT aspect stretch.
    viewing_textmode: bool, // is the currently-loaded viewer image text-mode art?
    crt_aspect: bool,       // stretch text-mode art ~1.2× vertically (4:3 CRT look)
    font_9px: bool,         // render the VGA font in a 9-dot cell (true DOS width)
    baud_ansi: Baud,        // simulated modem speed for ANSImation playback (persisted)
    baud_rip: Baud,         // simulated modem speed for RIP playback — independent (persisted)
    crt_scanline_dark: f32, // retro-monitor scanline darkness, 0 = off (persisted)
    crt_scanline_scale: bool, // scale scanline spacing with the zoom (persisted)
    glow: bool,             // phosphor-glow bloom around bright pixels (persisted)
    glow_amt: f32,          // phosphor-glow intensity (persisted)
    black_bg: bool,         // fill the viewer background black instead of dark grey (persisted)
    // Metadata OSD: a fading info overlay shown on each newly opened image.
    osd_enabled: bool,  // show the OSD at all (persisted)
    osd_position: u8,   // anchor 0..=7: TL,T,TR,L,R,BL,B,BR (3×3 grid, no center) (persisted)
    osd_secs: f32,      // how long it holds at full opacity before fading (persisted)
    osd_t: f32,         // animation clock since the current image opened (runtime)
    osd_rect: Option<egui::Rect>,        // last-painted OSD bounds (for hover-to-pin)
    osd_links: Vec<(egui::Rect, PathBuf)>, // clickable field rects → open_folder target
    osd_close: Option<egui::Rect>,       // the [×] dismiss button's last-painted rect
    osd_dismissed: bool, // [×] clicked → hide the OSD for *this* image (reset on next load)
    scanline_tex: Option<egui::TextureHandle>, // lazily-built 1×3 scanline pattern
    auto_next: bool,        // slideshow: auto-advance to the next file after a delay (persisted)
    auto_next_secs: u8,     // slideshow delay in seconds: 1/3/5/10 (persisted)
    auto_next_dwell: f32,   // slideshow: seconds the current (settled) file has been shown
    auto_paused: bool,      // slideshow paused by a user interaction (scroll/key/drag); not persisted
    immersive: bool,        // F11 fullscreen: hide all bars/UI, show only the art
    idle_t: f32,            // seconds of mouse stillness (immersive cursor auto-hide)
    shuffle: bool,          // screensaver: at a pack's end, load another random pack (persisted)
    // screensaver: a worker picking a random 16colo.rs pack → (year, name, download URL)
    random_rx: Option<std::sync::mpsc::Receiver<RandomPick>>,
    pending_autoplay: bool, // open the first art file once a (random) pack finishes mounting
    textmode_zoom: f32,     // remembered viewer zoom for text-mode art (persisted)
    raster_zoom: f32,       // remembered viewer zoom for ordinary raster art (persisted)
    adjust_drag: Option<usize>, // colorizer: order index being drag-reordered

    // navigation (Phase 2 + round 4)
    favorites: Vec<PathBuf>,
    // Optional color tag per favorite/pin (the favorite path → RGB), for organizing
    // Places. Assigned from the favorite's right-click ANSI32 palette. Persisted.
    fav_colors: HashMap<PathBuf, [u8; 3]>,
    path_edit: Option<String>, // Some(..) while the breadcrumb is in type-a-path mode
    focus_path: bool,          // request focus on the path field next frame
    dir_history: Vec<PathBuf>, // visited folders, for mouse back/forward
    dir_pos: usize,
    suppress_history: bool, // set while navigating *via* history (don't re-record)
    archive_mount: Option<ArchiveMount>, // the archive currently browsed as a folder
    remote_rx: Option<std::sync::mpsc::Receiver<RemoteMsg>>, // pending 16colo.rs fetch
    remote_urls: HashMap<PathBuf, String>, // virtual pack path → download URL
    #[allow(clippy::type_complexity)]
    remote_cache: HashMap<PathBuf, (Vec<Entry>, HashMap<PathBuf, String>)>, // year → packs (session)
    scroll_target: Option<usize>, // grid: scroll so this entry index becomes visible
    // Per-folder grid scroll offset (y), so navigating back to a folder restores where
    // you were. egui persists a ScrollArea by id, but the grid shares one id across all
    // folders — so we key the offset by folder path ourselves. `grid_scroll_pending` is
    // the offset to apply *once* on the next grid frame (set on every folder switch).
    grid_scroll: HashMap<PathBuf, f32>,
    grid_scroll_pending: Option<f32>,
    search: Option<String>,       // grid: live filename filter (vim-style '/')
    focus_search: bool,
    // advanced recursive search → a "Search results" grid
    show_search: bool,      // the advanced-search panel is open
    focus_adv_search: bool, // request focus on the panel's first field next frame
    search_spec: SearchSpec,
    search_results: Option<Vec<Entry>>, // Some → the grid renders results, not the folder
    search_root: Option<PathBuf>, // folder the active search started from (for relative labels)
    search_rx: Option<std::sync::mpsc::Receiver<SearchMsg>>,
    search_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    search_running: bool,
    saved_filters: Vec<(String, SearchSpec)>, // "smart filters": named saved searches (persisted)
    // file operations (round 4)
    clipboard: Option<(Vec<PathBuf>, bool)>, // (paths, is_cut)
    undo_stack: Vec<UndoOp>,
    renaming: Option<(PathBuf, String)>, // (path being renamed, new-name buffer)
    focus_rename: bool,

    status: String,
    want_repaint: bool,

    // Persisted UI scale (egui zoom factor) — what Ctrl +/- adjusts (whole GUI).
    ui_zoom: f32,
    // Persisted on-screen thumbnail tile size in points — Ctrl+wheel in the grid.
    // Independent of `ui_zoom`: this scales tiles only, not the chrome.
    thumb_size: f32,

    // Table view (an alternate renderer for the browse/grid mode — `Mode` stays
    // `Grid`, so selection/ratings/nav/keys all work unchanged). Persisted.
    table_view: bool,
    // Which optional file columns show in the table (bitmask of TC_*). Persisted.
    table_columns: u16,
    // Which optional 16colo *scene* columns show (bitmask of CS_*). Persisted.
    colo_columns: u16,
    // Per-column width overrides (ColKind as u8 → points), for drag-to-resize. Persisted.
    col_widths: HashMap<u8, f32>,
    // User column order (ColKind as u8) for the file / scene table layouts, from
    // drag-to-reorder; empty = the natural build order. Unknown/new kinds append.
    table_order: Vec<u8>,
    colo_order: Vec<u8>,
    // 16colo.rs flat-piece listing state. `colo_flat` marks the current view as a
    // flattened artist/group/search listing (→ the table shows scene columns). The
    // map carries per-piece metadata keyed by virtual display path (see [`ColoPiece`]).
    colo_flat: bool,
    colo_pieces: HashMap<PathBuf, ColoPiece>,
    // Streaming channel for a piece listing (mirrors `search_rx`): Hit(piece) per match.
    colo_rx: Option<std::sync::mpsc::Receiver<ColoMsg>>,
    colo_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    // Remote thumbnail fetcher for 16colo pieces (downloads `tn` PNGs off the UI thread).
    colo_thumbs: RemoteThumbs,
    // Downloaded single-piece files: virtual display path → local cache file, so the
    // viewer can decode a piece opened from the flat listing (see `resolve_local`).
    colo_files: HashMap<PathBuf, PathBuf>,
    // A piece whose `raw` file is downloading so we can open it in the viewer once ready.
    #[allow(clippy::type_complexity)]
    colo_open_rx:
        Option<std::sync::mpsc::Receiver<Result<(PathBuf, PathBuf, Option<crate::sauce::Sauce>), String>>>,
    // Status messages from "Download file/pack" save threads (drained into `status`).
    colo_save_rx: Option<std::sync::mpsc::Receiver<String>>,
    // Background pack-SAUCE fetcher for *inspected* (hovered) 16colo pieces. 16colo
    // strips SAUCE from the raw file and the artist/search endpoints omit it, so when
    // the Details panel inspects a piece with no SAUCE we fetch its whole pack's SAUCE
    // (`?sauce=true`) once and seed every file. Shared channel (tx cloned per fetch)
    // since several packs can be in flight; `colo_sauce_done` dedupes per "year/pack".
    // Each seed is (virtual path, optional SAUCE, filesize) — one per file in the pack,
    // so the same fetch backfills both the SAUCE panel and the "0 B" Size (the listing
    // endpoint omits both). filesize is 0 when unknown.
    #[allow(clippy::type_complexity)]
    colo_sauce_tx: std::sync::mpsc::Sender<Vec<(PathBuf, Option<crate::sauce::Sauce>, u64)>>,
    #[allow(clippy::type_complexity)]
    colo_sauce_rx: std::sync::mpsc::Receiver<Vec<(PathBuf, Option<crate::sauce::Sauce>, u64)>>,
    colo_sauce_done: HashSet<String>,
    colo_sauce_pending: usize, // in-flight pack-SAUCE fetches (for the busy spinner)
}

impl PixelView {
    /// Storage key for the persisted UI zoom factor.
    const ZOOM_KEY: &'static str = "ui_zoom_factor";
    /// Storage key for the persisted thumbnail tile size.
    const THUMB_KEY: &'static str = "thumb_size";
    /// Storage key for the persisted favorite folders.
    const FAV_KEY: &'static str = "favorites";
    const FAV_COLORS_KEY: &'static str = "fav_colors";
    /// Storage key for saved searches ("smart filters"): `Vec<Vec<String>>`, each
    /// row = `[display_name, name, ext, wmin, wmax, hmin, hmax, sauce]`.
    const SAVED_FILTERS_KEY: &'static str = "saved_filters";
    /// Storage key for the last-opened folder (reopened on launch).
    const FOLDER_KEY: &'static str = "last_folder";
    /// Storage keys for sort & filter state.
    const SORT_KEY: &'static str = "sort_key";
    const SORT_DESC: &'static str = "sort_desc";
    const DIRS_FIRST: &'static str = "dirs_first";
    const MIN_RATING: &'static str = "min_rating";
    const EXPLORER_KEY: &'static str = "show_explorer";
    const DETAILS_KEY: &'static str = "show_details";
    const RECOLOR_KEY: &'static str = "show_recolor";
    const RECOLOR_GRID_KEY: &'static str = "recolor_grid";
    const ZOOM_LOCK_KEY: &'static str = "zoom_lock";
    const FIT_MODE_KEY: &'static str = "fit_mode";
    const TEXTMODE_ZOOM_KEY: &'static str = "textmode_zoom";
    const CRT_ASPECT_KEY: &'static str = "crt_aspect";
    const FONT_9PX_KEY: &'static str = "font_9px";
    const BAUD_ANSI_KEY: &'static str = "baud_ansi";
    const BAUD_RIP_KEY: &'static str = "baud_rip";
    const CRT_SCANLINE_DARK_KEY: &'static str = "crt_scanline_dark";
    const CRT_SCANLINE_SCALE_KEY: &'static str = "crt_scanline_scale";
    const BLACK_BG_KEY: &'static str = "black_bg";
    const OSD_ENABLED_KEY: &'static str = "osd_enabled";
    const OSD_POSITION_KEY: &'static str = "osd_position";
    const OSD_SECS_KEY: &'static str = "osd_secs";
    const AUTO_NEXT_KEY: &'static str = "auto_next";
    const AUTO_NEXT_SECS_KEY: &'static str = "auto_next_secs";
    const GLOW_KEY: &'static str = "glow";
    const GLOW_AMT_KEY: &'static str = "glow_amt";
    const SHUFFLE_KEY: &'static str = "shuffle";
    const ADJUST_KEY: &'static str = "adjust";
    const ADJUST_ORDER_KEY: &'static str = "adjust_order";
    const IMG_ZOOM_KEY: &'static str = "image_zoom";
    const THEME_KEY: &'static str = "theme";
    const GAP_KEY: &'static str = "grid_gap";
    const GAP_Y_KEY: &'static str = "grid_gap_y";
    const CAPTION_KEY: &'static str = "caption_fields";
    const QUANT_ON_KEY: &'static str = "quantize_on";
    const QUANT_N_KEY: &'static str = "quantize_n";
    const DITHER_METHOD_KEY: &'static str = "dither_method";
    const DITHER_AMOUNT_KEY: &'static str = "dither_amount";
    const DITHER_CUSTOM_KEY: &'static str = "dither_custom";
    const DITHER_CUSTOM_N_KEY: &'static str = "dither_custom_n";
    const BALANCE_COLOR_KEY: &'static str = "balance_color";
    const BALANCE_STRENGTH_KEY: &'static str = "balance_strength";
    const PALETTE_DIR_KEY: &'static str = "palette_dir";
    const PALETTE_FAV_KEY: &'static str = "palette_favorites";
    const SELECTED_PAL_KEY: &'static str = "selected_palette";
    const KEYMAP_KEY: &'static str = "keymap";
    /// Whether the browse view renders as a table (vs the thumbnail grid).
    const TABLE_VIEW_KEY: &'static str = "table_view";
    /// Bitmask of optional table columns shown (TC_*).
    const TABLE_COLUMNS_KEY: &'static str = "table_columns";
    const COLO_COLUMNS_KEY: &'static str = "colo_columns";
    const COL_WIDTHS_KEY: &'static str = "col_widths";
    const TABLE_ORDER_KEY: &'static str = "table_order";
    const COLO_ORDER_KEY: &'static str = "colo_order";

    pub fn new(cc: &eframe::CreationContext<'_>, cli: CliArgs) -> Self {
        let registry = Arc::new(Registry::with_builtins());
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        let thumbs = ThumbBuilder::new(Arc::clone(&registry), workers);
        // A few HTTP workers for 16colo.rs thumbnail PNGs (don't hammer the server).
        let colo_thumbs = RemoteThumbs::new(workers.min(6));

        // Restore the UI scale the user last set with Ctrl +/- (defaults to 1.0).
        let ui_zoom = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::ZOOM_KEY))
            .unwrap_or(1.0);
        cc.egui_ctx.set_zoom_factor(ui_zoom);

        // Thumbnail tile size: CLI flag wins, else the persisted value, else default.
        let thumb_size = cli
            .thumb_size
            .or_else(|| {
                cc.storage
                    .and_then(|s| eframe::get_value::<f32>(s, Self::THUMB_KEY))
            })
            .unwrap_or(DEFAULT_TILE)
            .clamp(MIN_TILE, MAX_TILE);

        let favorites = cc
            .storage
            .and_then(|s| eframe::get_value::<Vec<PathBuf>>(s, Self::FAV_KEY))
            .unwrap_or_default();
        let fav_colors: HashMap<PathBuf, [u8; 3]> = cc
            .storage
            .and_then(|s| eframe::get_value::<Vec<(PathBuf, [u8; 3])>>(s, Self::FAV_COLORS_KEY))
            .map(|v| v.into_iter().collect())
            .unwrap_or_default();

        // Saved searches: each persisted row is [display_name, ...7 spec fields].
        let saved_filters: Vec<(String, SearchSpec)> = cc
            .storage
            .and_then(|s| eframe::get_value::<Vec<Vec<String>>>(s, Self::SAVED_FILTERS_KEY))
            .unwrap_or_default()
            .into_iter()
            .filter(|row| !row.is_empty())
            .map(|row| (row[0].clone(), SearchSpec::from_record(&row[1..])))
            .collect();

        let last_folder = cc
            .storage
            .and_then(|s| eframe::get_value::<Option<PathBuf>>(s, Self::FOLDER_KEY))
            .flatten();
        // CLI --folder wins over the last-opened folder; canonicalize so a relative
        // path from the shell becomes a clean absolute one for the breadcrumb.
        let open_target = cli
            .folder
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
            .or(last_folder);

        let get_u8 = |k| cc.storage.and_then(|s| eframe::get_value::<u8>(s, k));
        let get_bool = |k| cc.storage.and_then(|s| eframe::get_value::<bool>(s, k));
        let sort_key = SortKey::from_u8(get_u8(Self::SORT_KEY).unwrap_or(0));
        let sort_desc = get_bool(Self::SORT_DESC).unwrap_or(false);
        let table_view = get_bool(Self::TABLE_VIEW_KEY).unwrap_or(false);
        let table_columns = cc
            .storage
            .and_then(|s| eframe::get_value::<u16>(s, Self::TABLE_COLUMNS_KEY))
            .unwrap_or(TABLE_COLUMNS_DEFAULT);
        let colo_columns = cc
            .storage
            .and_then(|s| eframe::get_value::<u16>(s, Self::COLO_COLUMNS_KEY))
            .unwrap_or(COLO_COLUMNS_DEFAULT);
        let col_widths: HashMap<u8, f32> = cc
            .storage
            .and_then(|s| eframe::get_value::<Vec<(u8, f32)>>(s, Self::COL_WIDTHS_KEY))
            .map(|v| v.into_iter().collect())
            .unwrap_or_default();
        let table_order: Vec<u8> = cc
            .storage
            .and_then(|s| eframe::get_value(s, Self::TABLE_ORDER_KEY))
            .unwrap_or_default();
        let colo_order: Vec<u8> = cc
            .storage
            .and_then(|s| eframe::get_value(s, Self::COLO_ORDER_KEY))
            .unwrap_or_default();
        let dirs_first = get_bool(Self::DIRS_FIRST).unwrap_or(true);
        let min_rating = get_u8(Self::MIN_RATING).unwrap_or(0);
        let show_explorer = get_bool(Self::EXPLORER_KEY).unwrap_or(false);
        let show_details = get_bool(Self::DETAILS_KEY).unwrap_or(false);
        let show_recolor = get_bool(Self::RECOLOR_KEY).unwrap_or(false);
        let recolor_grid = get_bool(Self::RECOLOR_GRID_KEY).unwrap_or(false);
        let zoom_lock = get_bool(Self::ZOOM_LOCK_KEY).unwrap_or(true);
        let fit_mode = get_bool(Self::FIT_MODE_KEY).unwrap_or(false);
        // Adjust values: current = [f32; 12] (with invert); fall back to the legacy
        // [f32; 11] so old saves keep their tone/color settings.
        let adjust = cc
            .storage
            .and_then(|s| {
                eframe::get_value::<[f32; 12]>(s, Self::ADJUST_KEY)
                    .map(Adjust::from_array)
                    .or_else(|| {
                        eframe::get_value::<[f32; 11]>(s, Self::ADJUST_KEY)
                            .map(Adjust::from_array11)
                    })
            })
            .unwrap_or_default();
        // Apply order: current = [u8; 15]; fall back to the legacy [u8; 12] order
        // (with_order appends any ops that didn't exist when it was saved).
        let adjust = cc
            .storage
            .and_then(|s| {
                eframe::get_value::<[u8; 15]>(s, Self::ADJUST_ORDER_KEY)
                    .map(|o| o.to_vec())
                    .or_else(|| {
                        eframe::get_value::<[u8; 12]>(s, Self::ADJUST_ORDER_KEY).map(|o| o.to_vec())
                    })
            })
            .map(|o| adjust.with_order(&o))
            .unwrap_or(adjust);
        let img_zoom = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::IMG_ZOOM_KEY))
            .unwrap_or(1.0)
            .clamp(0.05, 64.0);
        let textmode_zoom = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::TEXTMODE_ZOOM_KEY))
            .unwrap_or(3.0)
            .clamp(0.05, 64.0);
        // Defaults tuned for the late-night-BBS look (the user's preferred setup): CRT
        // aspect + 9px cell + scanlines + scale + glow + black bg + Snap, ANSI at 4800
        // baud (RIP at 9600), auto-advance on. Any persisted value still overrides.
        let crt_aspect = get_bool(Self::CRT_ASPECT_KEY).unwrap_or(true);
        // NB: left default-off. Its setter flips a process-global flag the decoder reads,
        // which would leak into (parallel) decode tests via PixelView::new; the user's
        // persisted choice still applies. The other look defaults are per-instance + safe.
        let font_9px = get_bool(Self::FONT_9PX_KEY).unwrap_or(false);
        let baud_ansi = get_u8(Self::BAUD_ANSI_KEY)
            .map(Baud::from_u8)
            .unwrap_or(Baud::B4800);
        let baud_rip = get_u8(Self::BAUD_RIP_KEY)
            .map(Baud::from_u8)
            .unwrap_or(Baud::B9600);
        let crt_scanline_dark = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::CRT_SCANLINE_DARK_KEY))
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let crt_scanline_scale = get_bool(Self::CRT_SCANLINE_SCALE_KEY).unwrap_or(true);
        let black_bg = get_bool(Self::BLACK_BG_KEY).unwrap_or(true);
        let osd_enabled = get_bool(Self::OSD_ENABLED_KEY).unwrap_or(true);
        let osd_position = get_u8(Self::OSD_POSITION_KEY).unwrap_or(1).min(7);
        let osd_secs = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::OSD_SECS_KEY))
            .unwrap_or(3.0)
            .clamp(0.5, 30.0);
        let auto_next = get_bool(Self::AUTO_NEXT_KEY).unwrap_or(true);
        let auto_next_secs = get_u8(Self::AUTO_NEXT_SECS_KEY).unwrap_or(5).clamp(1, 60);
        let glow = get_bool(Self::GLOW_KEY).unwrap_or(true);
        let shuffle = get_bool(Self::SHUFFLE_KEY).unwrap_or(false);
        let glow_amt = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::GLOW_AMT_KEY))
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        // Prime the decoder before the first decode so restored art renders at the
        // persisted cell width.
        crate::decode::set_font_9px(font_9px);
        // Ratings sidecar lives alongside eframe's own storage (e.g.
        // ~/.local/share/pixelview/ratings.json); temp dir is a harmless fallback.
        let data_dir = eframe::storage_dir("pixelview").unwrap_or_else(std::env::temp_dir);
        let ratings = crate::ratings::RatingStore::load(&data_dir);
        // View history lives alongside the ratings sidecar + eframe storage.
        let viewdb = crate::viewdb::ViewDb::open(&data_dir);
        // Persistent on-disk HTTP cache (16colo JSON / thumbnails / files / zips).
        crate::cache::init(&data_dir);
        let theme = get_u8(Self::THEME_KEY).unwrap_or(0);
        if theme == 1 {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
        }
        let grid_gap = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::GAP_KEY))
            .unwrap_or(GAP)
            .clamp(0.0, 40.0);
        let grid_gap_y = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::GAP_Y_KEY))
            .unwrap_or(GAP_Y)
            .clamp(0.0, 80.0);
        let caption_fields = cc
            .storage
            .and_then(|s| eframe::get_value::<u16>(s, Self::CAPTION_KEY))
            .unwrap_or(CAP_NAME);
        let quantize_on = get_bool(Self::QUANT_ON_KEY).unwrap_or(false);
        let quantize_n = cc
            .storage
            .and_then(|s| eframe::get_value::<usize>(s, Self::QUANT_N_KEY))
            .unwrap_or(16)
            .clamp(2, 256);
        let dither_method = cc
            .storage
            .and_then(|s| eframe::get_value::<u8>(s, Self::DITHER_METHOD_KEY))
            .unwrap_or(0);
        let dither_amount = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::DITHER_AMOUNT_KEY))
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let dither_custom_n = cc
            .storage
            .and_then(|s| eframe::get_value::<usize>(s, Self::DITHER_CUSTOM_N_KEY))
            .filter(|&n| n == 2 || n == 4 || n == 8)
            .unwrap_or(4);
        let dither_custom = cc
            .storage
            .and_then(|s| eframe::get_value::<Vec<u32>>(s, Self::DITHER_CUSTOM_KEY))
            .filter(|v| v.len() == dither_custom_n * dither_custom_n)
            .unwrap_or_else(|| crate::thumb::bayer_values(dither_custom_n));
        let balance_color = cc
            .storage
            .and_then(|s| eframe::get_value::<[u8; 3]>(s, Self::BALANCE_COLOR_KEY))
            .unwrap_or([128, 128, 128]);
        let balance_strength = cc
            .storage
            .and_then(|s| eframe::get_value::<f32>(s, Self::BALANCE_STRENGTH_KEY))
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let balance_hex = format!(
            "{:02X}{:02X}{:02X}",
            balance_color[0], balance_color[1], balance_color[2]
        );
        let palette_dir = cc
            .storage
            .and_then(|s| eframe::get_value::<PathBuf>(s, Self::PALETTE_DIR_KEY))
            .unwrap_or_else(|| {
                home_dir()
                    .map(|h| h.join(DEFAULT_PALETTE_SUBDIR))
                    .unwrap_or_default()
            });
        let palette_files = all_palettes(&palette_dir);
        let palette_favorites = cc
            .storage
            .and_then(|s| eframe::get_value::<Vec<PathBuf>>(s, Self::PALETTE_FAV_KEY))
            .unwrap_or_else(|| {
                // Default favorites point at the built-in palettes so they work
                // with no external palette directory.
                let root = Path::new(crate::palettes_builtin::BUILTIN_ROOT);
                DEFAULT_PALETTE_FAVS.iter().map(|n| root.join(n)).collect()
            });
        // Normalize persisted favorites onto the canonical `palette_files` paths
        // (matched by file name) so the gold ★ lights up. Favorites saved before the
        // palettes were embedded used real file paths, which no longer equal the
        // virtual "<built-in palettes>/…" entries — making `contains()` always false.
        // Remap them (and de-dupe); favorites whose palette is gone are kept as-is.
        let palette_favorites: Vec<PathBuf> = {
            let mut seen = std::collections::HashSet::new();
            palette_favorites
                .into_iter()
                .map(|fav| {
                    palette_files
                        .iter()
                        .find(|p| p.file_name() == fav.file_name())
                        .cloned()
                        .unwrap_or(fav)
                })
                .filter(|p| seen.insert(p.clone()))
                .collect()
        };
        let selected_palette = cc
            .storage
            .and_then(|s| eframe::get_value::<Option<PathBuf>>(s, Self::SELECTED_PAL_KEY))
            .flatten();

        let mut keymap: HashMap<Action, egui::Key> =
            Action::ALL.iter().map(|&a| (a, a.default_key())).collect();
        if let Some(saved) = cc
            .storage
            .and_then(|s| eframe::get_value::<Vec<(u8, String)>>(s, Self::KEYMAP_KEY))
        {
            for (au, kn) in saved {
                if let (Some(&a), Some(k)) =
                    (Action::ALL.get(au as usize), egui::Key::from_name(&kn))
                {
                    keymap.insert(a, k);
                }
            }
        }

        // Shared channel for background pack-SAUCE fetches (see `ensure_colo_sauce`).
        let (colo_sauce_tx, colo_sauce_rx) = std::sync::mpsc::channel();

        let mut app = Self {
            registry,
            thumbs,
            folder: None,
            all_entries: Vec::new(),
            entries: Vec::new(),
            thumb_tex: HashMap::new(),
            img_meta: HashMap::new(),
            sauce_cache: HashMap::new(),
            palettes: HashMap::new(),
            thumb_rgba: HashMap::new(),
            grid_recolor: HashMap::new(),
            folder_info: HashMap::new(),
            mode: Mode::Grid,
            selected: 0,
            selection: HashSet::new(),
            anchor: None,
            hovered: None,
            sort_key,
            sort_desc,
            dirs_first,
            min_rating,
            ratings,
            viewdb,
            show_explorer,
            show_details,
            show_recolor,
            recolor_grid,
            last_inspected: None,
            custom_palette: None,
            flash: None,
            editing_color: None,
            adjust,
            explorer_filter: String::new(),
            colo_search: String::new(),
            explorer_tab: 0,
            places_tab: 0,
            show_hotkeys: false,
            show_prefs: false,
            theme,
            grid_gap,
            grid_gap_y,
            caption_fields,
            quantize_on,
            quantize_n,
            dither_method,
            dither_amount,
            dither_custom,
            dither_custom_n,
            balance_color,
            balance_strength,
            balance_hex,
            quantize_cache: None,
            palette_dir,
            palette_files,
            palette_favorites,
            selected_palette,
            loaded_palettes: HashMap::new(),
            preview_src: None,
            preview_tex: None,
            keymap,
            rebinding: None,
            full_tex: None,
            full_src: None,
            full_reduced: None,
            anim: None,
            hover_anim: None,
            player: None,
            zoom: img_zoom,
            zoom_lock,
            offset: egui::Vec2::ZERO,
            view_to_top: false,
            fit_width_on_open: false,
            nav_drag: false,
            minimap: None,
            play_autoscroll: None,
            tile_mode: false,
            fit_requested: fit_mode, // fit the first-loaded image if the mode is on
            fit_mode,
            viewing_textmode: false,
            crt_aspect,
            font_9px,
            baud_ansi,
            baud_rip,
            crt_scanline_dark,
            crt_scanline_scale,
            black_bg,
            osd_enabled,
            osd_position,
            osd_secs,
            osd_t: f32::INFINITY, // start hidden until the first image opens
            osd_rect: None,
            osd_links: Vec::new(),
            osd_close: None,
            osd_dismissed: false,
            glow,
            glow_amt,
            scanline_tex: None,
            auto_next,
            auto_next_secs,
            auto_next_dwell: 0.0,
            auto_paused: false,
            immersive: false,
            idle_t: 0.0,
            shuffle,
            random_rx: None,
            pending_autoplay: false,
            textmode_zoom,
            raster_zoom: img_zoom,
            adjust_drag: None,
            favorites,
            fav_colors,
            path_edit: None,
            focus_path: false,
            dir_history: Vec::new(),
            archive_mount: None,
            remote_rx: None,
            remote_urls: HashMap::new(),
            remote_cache: HashMap::new(),
            dir_pos: 0,
            suppress_history: false,
            scroll_target: None,
            grid_scroll: HashMap::new(),
            grid_scroll_pending: None,
            search: None,
            focus_search: false,
            show_search: false,
            focus_adv_search: false,
            search_spec: SearchSpec::default(),
            search_results: None,
            search_root: None,
            search_rx: None,
            search_cancel: None,
            search_running: false,
            saved_filters,
            clipboard: None,
            undo_stack: Vec::new(),
            renaming: None,
            focus_rename: false,
            status: "Open a folder to begin.".into(),
            want_repaint: false,
            ui_zoom,
            thumb_size,
            table_view,
            table_columns,
            colo_columns,
            col_widths,
            table_order,
            colo_order,
            colo_flat: false,
            colo_pieces: HashMap::new(),
            colo_rx: None,
            colo_cancel: None,
            colo_thumbs,
            colo_files: HashMap::new(),
            colo_open_rx: None,
            colo_save_rx: None,
            colo_sauce_tx,
            colo_sauce_rx,
            colo_sauce_done: HashSet::new(),
            colo_sauce_pending: 0,
        };

        // Reopen wherever we left off so the grid, breadcrumb, and favorites are all
        // visible on launch instead of an empty window. `open_folder` itself routes the
        // virtual cases, so allow a real dir, a 16colo.rs path (re-fetched), or an
        // archive file (re-extracted) — not only an on-disk directory.
        if let Some(dir) = open_target {
            if dir.is_dir()
                || crate::sixteen::is_remote(&dir)
                || (dir.is_file() && crate::archive::is_archive(&dir))
            {
                app.open_folder(dir);
            }
        }
        app
    }

    /// Scan `dir` into folder + image entries (directories first, then images,
    /// each name-sorted — the default order; Phase 5 makes this configurable).
    fn open_folder(&mut self, dir: PathBuf) {
        // Visiting a folder (incl. a 16colo.rs pack) marks it viewed — recorded here,
        // before the remote/archive redirect, so the key is the tile's own path.
        self.mark_viewed(&dir);
        // The virtual 16colo.rs tree (years → packs → downloaded pack contents).
        if crate::sixteen::is_remote(&dir) {
            self.open_remote(dir);
            return;
        }
        // An archive path is a *virtual* folder: extract it once, then browse the
        // extracted temp dir. Routes here so every navigation path (click, crumb,
        // history, favorite) handles archives uniformly.
        if dir.is_file() && crate::archive::is_archive(&dir) {
            self.enter_archive(dir);
            return;
        }
        let mut all: Vec<Entry> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                // Skip dotfiles/hidden entries to match a Dolphin-like default view.
                if p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
                {
                    continue;
                }
                if p.is_dir() {
                    all.push(make_entry(p, true));
                } else if p
                    .extension()
                    .and_then(|x| x.to_str())
                    // No extension → scene/BBS art is often extensionless; show it
                    // (decoded as CP437 text). A known extension must actually match.
                    .is_none_or(|x| self.registry.known_extension(x))
                    || crate::archive::is_archive(&p)
                {
                    all.push(make_entry(p, false));
                }
            }
        }
        // Resolve ratings: on-disk files read their xattr (with a sidecar fallback);
        // files inside a mount (archive / 16colo.rs pack) — whose disk copy is a
        // disposable temp extraction — resolve from the sidecar by display path.
        for e in all.iter_mut().filter(|e| !e.is_dir) {
            e.rating = self.read_rating(&e.path);
        }
        self.show_folder(dir, all);
    }

    /// Commit a navigation: set the entry list + current folder, record history,
    /// reset selection, and switch to the grid. Shared by `open_folder` and the
    /// virtual 16colo.rs views.
    fn show_folder(&mut self, dir: PathBuf, entries: Vec<Entry>) {
        // Folder counts are rendered live in the status bar; clear any transient
        // file-op message carried over from the folder we're leaving (file ops set
        // their message *after* calling refresh(), so theirs survives this).
        self.status.clear();
        // Navigating leaves any "Search results" view (and stops a running search).
        if self.search_results.is_some() || self.search_running {
            self.cancel_search();
            self.search_results = None;
        }
        // …and leaves any 16colo.rs flat-piece listing (stops its stream + drops its
        // metadata). The listing handler re-sets `colo_flat` *after* calling us.
        self.cancel_colo();
        self.colo_flat = false;
        self.colo_pieces.clear();
        self.all_entries = entries;
        // Record in the back/forward history unless we're navigating *via* it.
        if !self.suppress_history {
            self.dir_history.truncate(self.dir_pos + 1);
            if self.dir_history.last() != Some(&dir) {
                self.dir_history.push(dir.clone());
            }
            self.dir_pos = self.dir_history.len().saturating_sub(1);
        }
        self.suppress_history = false;
        // Restore this folder's last grid scroll position (0 for a folder we've not seen
        // — needed to override egui's shared "grid" offset left by the folder we left).
        self.grid_scroll_pending = Some(self.grid_scroll.get(&dir).copied().unwrap_or(0.0));
        self.folder = Some(dir);
        // NB: keep `thumb_tex`/`img_meta` — persistent path-keyed caches (Phase 2 fix:
        // clearing them while the worker's `requested` set persists rendered black
        // tiles, and keeping them makes back-navigation instant).
        self.selected = 0;
        self.selection.clear();
        self.anchor = None;
        self.path_edit = None;
        self.mode = Mode::Grid;
        self.rebuild_view();
    }

    /// Navigate the virtual 16colo.rs tree. Level 0 (root) lists Years synchronously;
    /// level 1 (a year) fetches its Packs on a background thread; level 2 (a pack)
    /// downloads the zip, then hands off to the archive-browsing path.
    fn open_remote(&mut self, dir: PathBuf) {
        use crate::sixteen;
        let parts = sixteen::rel_parts(&dir);

        // A pack path (a known download URL, or a `year/pack` we can derive one for) →
        // download the zip and hand off to the archive-browsing path.
        let download_url = self.remote_urls.get(&dir).cloned().or_else(|| {
            (parts.len() == 2 && parts[0].parse::<u32>().is_ok())
                .then(|| sixteen::pack_url(parts[0].parse().unwrap_or(0), &parts[1]))
        });
        if let Some(url) = download_url {
            let pack = parts.last().cloned().unwrap_or_default();
            self.status = format!("Downloading {pack}…");
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(match sixteen::download(&url) {
                    Ok(zip) => RemoteMsg::PackZip(dir, zip),
                    Err(e) => RemoteMsg::Err(e),
                });
            });
            self.remote_rx = Some(rx);
            return;
        }

        // A cached listing (year packs, group/artist lists, …) → show instantly.
        if let Some((entries, urls)) = self.remote_cache.get(&dir) {
            let (entries, urls) = (entries.clone(), urls.clone());
            self.remote_urls.extend(urls);
            self.show_folder(dir, entries);
            self.status = format!("{} items (cached)", self.all_entries.len());
            return;
        }

        // Otherwise list this level (root is synchronous; the rest fetch in the bg).
        match parts.as_slice() {
            [] => {
                let root = Path::new(sixteen::ROOT);
                let mut entries = vec![
                    virtual_dir(root.join(sixteen::LATEST)),
                    virtual_dir(root.join(sixteen::GROUPS)),
                    virtual_dir(root.join(sixteen::ARTISTS)),
                ];
                entries.extend(
                    sixteen::years()
                        .into_iter()
                        .map(|y| virtual_dir(root.join(y.to_string()))),
                );
                self.show_folder(dir, entries);
                self.status = "16colo.rs — latest / groups / artists, or pick a year".into();
            }
            [s] if s.as_str() == sixteen::LATEST => {
                self.status = "Loading latest releases…".into();
                self.spawn_listing(dir, || {
                    sixteen::fetch_latest()
                        .map(|ps| ps.into_iter().map(|p| (p.name, Some(p.url))).collect())
                });
            }
            [s] if s.as_str() == sixteen::GROUPS => {
                self.status = "Loading groups…".into();
                self.spawn_listing(dir, || {
                    sixteen::fetch_groups().map(|ns| ns.into_iter().map(|n| (n, None)).collect())
                });
            }
            [s] if s.as_str() == sixteen::ARTISTS => {
                self.status = "Loading artists…".into();
                self.spawn_listing(dir, || {
                    sixteen::fetch_artists().map(|ns| ns.into_iter().map(|n| (n, None)).collect())
                });
            }
            // An artist / group / search now flattens to individual *pieces* (a sortable
            // table), not a grid of pack folders — the whole point of this view.
            [s, group] if s.as_str() == sixteen::GROUPS => {
                self.start_colo_pieces(dir, ColoSource::Group(group.clone()));
            }
            [s, artist] if s.as_str() == sixteen::ARTISTS => {
                self.start_colo_pieces(dir, ColoSource::Artist(artist.clone()));
            }
            [s, query] if s.as_str() == sixteen::SEARCH => {
                self.start_colo_pieces(dir, ColoSource::Search(query.clone()));
            }
            // Facet-scoped search: SEARCH/<scope>/<query> (built by the nav bar).
            [s, scope, query] if s.as_str() == sixteen::SEARCH && scope == "artist" => {
                self.start_colo_pieces(dir, ColoSource::SearchArtists(query.clone()));
            }
            [s, scope, query] if s.as_str() == sixteen::SEARCH && scope == "group" => {
                self.start_colo_pieces(dir, ColoSource::SearchGroups(query.clone()));
            }
            [year] if year.parse::<u32>().is_ok() => {
                let year: u32 = year.parse().unwrap_or(0);
                self.status = format!("Loading {year} packs…");
                self.spawn_listing(dir, move || {
                    sixteen::fetch_packs(year)
                        .map(|ps| ps.into_iter().map(|p| (p.name, Some(p.url))).collect())
                });
            }
            _ => {
                self.status = format!("16colo.rs: can't open {}", short_name(&dir));
            }
        }
    }

    /// Spawn a background 16colo.rs listing fetch. `f` yields `(name, Some(url))` per
    /// downloadable pack, or `(name, None)` for a sub-folder (group/artist); each
    /// becomes a virtual-dir entry under `dir`. Results land via [`poll_remote`].
    fn spawn_listing<F>(&mut self, dir: PathBuf, f: F)
    where
        F: FnOnce() -> Result<Vec<(String, Option<String>)>, String> + Send + 'static,
    {
        self.show_folder(dir.clone(), Vec::new());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let msg = match f() {
                Ok(items) => {
                    let mut entries = Vec::new();
                    let mut urls = HashMap::new();
                    for (name, url) in items {
                        let path = dir.join(&name);
                        if let Some(u) = url {
                            urls.insert(path.clone(), u);
                        }
                        entries.push(virtual_dir(path));
                    }
                    RemoteMsg::Packs(dir, entries, urls)
                }
                Err(e) => RemoteMsg::Err(e),
            };
            let _ = tx.send(msg);
        });
        self.remote_rx = Some(rx);
    }

    /// Start a flat 16colo.rs *piece* listing for `dir` (an artist / group / search).
    /// Pieces stream in on a background thread (mirrors the recursive-search pattern:
    /// `colo_rx` + an `AtomicBool` cancel), each a `ColoMsg::Hit(entry, piece)`, drained
    /// by [`poll_colo_pieces`]. Switches the view to the table so the scene columns show.
    fn start_colo_pieces(&mut self, dir: PathBuf, source: ColoSource) {
        // `show_folder` resets the view + clears the previous flat-listing state.
        let label = match &source {
            ColoSource::Artist(a) => format!("Loading {a}…"),
            ColoSource::Group(g) => format!("Loading {g}…"),
            ColoSource::Search(q) => format!("Searching “{q}”…"),
            ColoSource::SearchArtists(q) => format!("Searching artists “{q}”…"),
            ColoSource::SearchGroups(q) => format!("Searching groups “{q}”…"),
        };
        self.show_folder(dir.clone(), Vec::new());
        self.colo_flat = true;
        self.table_view = true; // a flat piece listing is the table's whole reason to exist
        self.status = label;

        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.colo_cancel = Some(Arc::clone(&cancel));
        let (tx, rx) = std::sync::mpsc::channel();
        self.colo_rx = Some(rx);
        std::thread::spawn(move || colo_walk(source, cancel, tx));
    }

    /// Stop a running flat-piece listing and forget its stream.
    fn cancel_colo(&mut self) {
        if let Some(c) = self.colo_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.colo_rx = None;
    }

    /// Drain streamed pieces into `all_entries` + `colo_pieces` (resolving each rating
    /// on the UI thread, like `open_folder`), re-sorting as they arrive.
    fn poll_colo_pieces(&mut self) {
        let Some(rx) = self.colo_rx.as_ref() else {
            return;
        };
        let mut got = 0usize;
        let mut done: Option<Result<usize, String>> = None;
        // Bounded drain per frame so a fast stream can't stall the UI.
        for _ in 0..512 {
            match rx.try_recv() {
                Ok(ColoMsg::Hit(mut entry, piece)) => {
                    // Skip non-viewable files (music/exec/video: .s3m/.it/.mod/.xm/.exe/
                    // .com/.mp4/…). 16colo has no thumbnail render for them, so their tile
                    // would spin forever. `known_extension` is the canonical art allowlist.
                    let viewable = entry
                        .path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_none_or(|e| self.registry.known_extension(e));
                    if !viewable {
                        continue;
                    }
                    entry.rating = self.read_rating(&entry.path);
                    // Pre-seed the SAUCE cache from the API so the Details panel shows
                    // title/author/group/dims for this piece without downloading it.
                    if piece.sauce.is_some() {
                        self.sauce_cache
                            .insert(entry.path.clone(), piece.sauce.clone());
                    }
                    self.colo_pieces.insert(entry.path.clone(), *piece);
                    self.all_entries.push(entry);
                    got += 1;
                }
                Ok(ColoMsg::Done(n)) => {
                    done = Some(Ok(n));
                    break;
                }
                Ok(ColoMsg::Err(e)) => {
                    done = Some(Err(e));
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    done = Some(Ok(self.all_entries.len()));
                    break;
                }
            }
        }
        if got > 0 {
            self.rebuild_view();
            self.want_repaint = true;
        }
        match done {
            Some(Ok(_)) => {
                self.colo_rx = None;
                self.colo_cancel = None;
                // Report the *shown* count (after the non-viewable filter), not the raw
                // fetched total, so it matches the rows on screen.
                self.status = format!("{} pieces", self.all_entries.len());
            }
            Some(Err(e)) => {
                self.colo_rx = None;
                self.colo_cancel = None;
                self.status = format!("16colo.rs: {e}");
            }
            None => self.want_repaint = true,
        }
    }

    /// Download a piece's single `raw` file (not its whole pack) so the viewer can open
    /// it; the local path lands via `colo_open_rx` → [`poll_colo_open`].
    fn start_piece_open(&mut self, vpath: PathBuf) {
        let Some(piece) = self.colo_pieces.get(&vpath) else {
            return;
        };
        let url = piece.raw_url.clone();
        let fname = vpath
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("art")
            .to_string();
        // 16colo strips SAUCE from the single `raw` file and the artist endpoint omits
        // it, so when a piece has no SAUCE yet (artist/search view) fetch its *pack's*
        // SAUCE (`?sauce=true`) to fill the Details panel. Keyed by (group, year, pack).
        let want_sauce = piece
            .sauce
            .is_none()
            .then(|| (piece.group.clone(), piece.year, piece.pack.clone(), fname.clone()));
        self.status = format!("Opening {fname}…");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let dl = crate::sixteen::download_file(&url, &fname);
            let sauce = want_sauce.and_then(|(group, year, pack, file)| {
                crate::sixteen::fetch_pack_pieces(&group, year, &pack)
                    .ok()?
                    .into_iter()
                    .find(|p| p.filename.eq_ignore_ascii_case(&file))?
                    .sauce
            });
            let _ = tx.send(dl.map(|local| (vpath, local, sauce)));
        });
        self.colo_open_rx = Some(rx);
    }

    /// A piece's `raw` file finished downloading → cache it + open it in the viewer.
    fn poll_colo_open(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.colo_open_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((vpath, local, sauce))) => {
                self.colo_open_rx = None;
                self.colo_files.insert(vpath.clone(), local);
                // Seed the SAUCE we fetched from the pack API (artist/search pieces); else
                // drop any stale `None` cached while the piece had no local file so
                // `cached_sauce` re-reads. (Group/pack pieces were already pre-seeded.)
                if sauce.is_some() {
                    self.sauce_cache.insert(vpath.clone(), sauce);
                } else {
                    self.sauce_cache.remove(&vpath);
                }
                self.load_full(ctx, vpath);
                self.mode = Mode::Single;
                self.want_repaint = true;
            }
            Ok(Err(e)) => {
                self.colo_open_rx = None;
                self.status = format!("Open failed: {e}");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => self.want_repaint = true,
            Err(_) => self.colo_open_rx = None,
        }
    }

    /// Lazily fetch SAUCE for an *inspected* (hovered/selected) 16colo piece. The artist
    /// and search listings come from endpoints that omit SAUCE, and the raw file has it
    /// stripped — so the Details panel would read "no record" until you actually open the
    /// piece. This fetches the piece's whole pack's SAUCE (`?sauce=true`) in the
    /// background and seeds *every* file in it, so hovering one piece fills the SAUCE for
    /// the rest of the pack too. Deduped per "year/pack" (`colo_sauce_done`) so sweeping a
    /// table costs one request per pack, not one per file. Cheap no-op when the piece
    /// already has SAUCE (pre-seeded, opened, or a prior pack fetch).
    fn ensure_colo_sauce(&mut self, path: &Path) {
        // Already resolved → nothing to do.
        if matches!(self.sauce_cache.get(path), Some(Some(_))) {
            return;
        }
        let Some(piece) = self.colo_pieces.get(path) else {
            return; // not a 16colo piece
        };
        if piece.sauce.is_some() {
            return; // pack/group listing already carried it
        }
        let (group, year, pack) = (piece.group.clone(), piece.year, piece.pack.clone());
        if !self.colo_sauce_done.insert(format!("{year}/{pack}")) {
            return; // this pack is already fetched or in flight
        }
        self.colo_sauce_pending += 1; // drives the SAUCE-panel + status spinner
        let tx = self.colo_sauce_tx.clone();
        std::thread::spawn(move || {
            let seeds: Vec<(PathBuf, Option<crate::sauce::Sauce>, u64)> =
                crate::sixteen::fetch_pack_pieces(&group, year, &pack)
                    .map(|pieces| {
                        pieces
                            .into_iter()
                            .map(|p| {
                                // Rebuild the same virtual display path `emit_piece` uses
                                // (ROOT/year/pack/filename) so the key matches the cache.
                                let vp = Path::new(crate::sixteen::ROOT)
                                    .join(p.year.to_string())
                                    .join(&p.pack)
                                    .join(&p.filename);
                                (vp, p.sauce, p.filesize)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            let _ = tx.send(seeds);
        });
        self.want_repaint = true;
    }

    /// Drain finished pack-SAUCE fetches into `sauce_cache` and backfill file sizes (the
    /// listing endpoint reports `0 B`; the pack endpoint carries the real size). See
    /// `ensure_colo_sauce`.
    fn poll_colo_sauce(&mut self) {
        while let Ok(seeds) = self.colo_sauce_rx.try_recv() {
            self.colo_sauce_pending = self.colo_sauce_pending.saturating_sub(1);
            let mut sizes: HashMap<PathBuf, u64> = HashMap::new();
            for (vpath, sauce, filesize) in seeds {
                if sauce.is_some() {
                    self.sauce_cache.insert(vpath.clone(), sauce);
                }
                if filesize > 0 {
                    sizes.insert(vpath, filesize);
                }
            }
            // Backfill the size into both the raw list and the rendered view so it shows
            // in Details and survives a later re-sort/filter (`rebuild_view`).
            if !sizes.is_empty() {
                for e in &mut self.all_entries {
                    if let Some(&sz) = sizes.get(&e.path) {
                        e.size = sz;
                    }
                }
                for e in &mut self.entries {
                    if let Some(&sz) = sizes.get(&e.path) {
                        e.size = sz;
                    }
                }
            }
            self.want_repaint = true;
        }
    }

    /// Drain a "Download file/pack" save thread's final status message.
    fn poll_colo_save(&mut self) {
        let Some(rx) = self.colo_save_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(msg) => {
                self.status = msg;
                self.colo_save_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => self.want_repaint = true,
            Err(_) => self.colo_save_rx = None,
        }
    }

    /// Map a display path to a locally-readable file for decoding: a downloaded 16colo
    /// piece resolves to its cached `raw` file; anything else is already real on disk.
    fn resolve_local(&self, path: &Path) -> PathBuf {
        self.colo_files
            .get(path)
            .cloned()
            .unwrap_or_else(|| path.to_path_buf())
    }

    /// A downloaded 16colo.rs pack zip finished: extract + mount it (with the
    /// virtual pack path as its display identity, so the breadcrumb keeps reading
    /// `16colo.rs › year › pack › …`) and browse its contents.
    fn enter_remote_pack(&mut self, vpath: PathBuf, zip: PathBuf) {
        match crate::archive::extract_to_cache(&zip) {
            Ok(temp_root) => {
                self.archive_mount = Some(ArchiveMount {
                    archive: vpath.clone(),
                    temp_root: temp_root.clone(),
                });
                self.open_folder(temp_root);
                self.status = format!("Opened {}", short_name(&vpath));
            }
            Err(e) => self.status = format!("Couldn't open pack: {e}"),
        }
    }

    /// Screensaver: kick off a worker that picks a random 16colo.rs pack (random year,
    /// random pack in it) and returns `(year, name, download URL)`.
    fn start_random_pack(&mut self) {
        if self.random_rx.is_some() {
            return; // one in flight already
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let pick: RandomPick = (|| {
                let years = crate::sixteen::years();
                let year = *pick_random(&years).ok_or("no years")?;
                let packs = crate::sixteen::fetch_packs(year)?;
                let pk = pick_random(&packs).ok_or("no packs that year")?;
                Ok((year, pk.name.clone(), pk.url.clone()))
            })();
            let _ = tx.send(pick);
        });
        self.random_rx = Some(rx);
        self.status = "Finding a random pack…".into();
    }

    /// Apply a finished random-pack pick: open the pack (it downloads + mounts), then
    /// `pending_autoplay` opens its first art file once it's ready.
    fn poll_random(&mut self) {
        let Some(rx) = self.random_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok((year, name, url))) => {
                self.random_rx = None;
                let path = Path::new(crate::sixteen::ROOT)
                    .join(year.to_string())
                    .join(&name);
                self.remote_urls.insert(path.clone(), url); // exact download URL
                self.pending_autoplay = true;
                self.open_folder(path);
                self.want_repaint = true;
            }
            Ok(Err(e)) => {
                self.random_rx = None;
                self.status = format!("random pack: {e}");
                if self.shuffle {
                    self.start_random_pack(); // keep the screensaver going
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => self.want_repaint = true,
            Err(_) => self.random_rx = None,
        }
    }

    /// Poll the background 16colo.rs thread; apply its result when ready.
    fn poll_remote(&mut self) {
        let Some(rx) = self.remote_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(RemoteMsg::Packs(dir, entries, urls)) => {
                self.remote_rx = None;
                // Cache for the session even if the user navigated away meanwhile.
                self.remote_cache
                    .insert(dir.clone(), (entries.clone(), urls.clone()));
                if self.folder.as_deref() == Some(dir.as_path()) {
                    self.remote_urls.extend(urls);
                    // "items" not "packs" — the listing may be groups/artists/search hits.
                    self.status = format!("{} items", entries.len());
                    self.all_entries = entries;
                    self.rebuild_view();
                }
            }
            Ok(RemoteMsg::PackZip(vpath, zip)) => {
                self.remote_rx = None;
                self.enter_remote_pack(vpath, zip);
            }
            Ok(RemoteMsg::Err(e)) => {
                self.remote_rx = None;
                self.status = format!("16colo.rs: {e}");
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => self.want_repaint = true,
            Err(_) => self.remote_rx = None,
        }
    }

    /// Enter an archive: extract it to a temp dir (cached), mount it, and browse
    /// that dir. The breadcrumb still shows the archive's name via `to_display`.
    fn enter_archive(&mut self, archive: PathBuf) {
        match crate::archive::extract_to_cache(&archive) {
            Ok(temp_root) => {
                self.archive_mount = Some(ArchiveMount {
                    archive: archive.clone(),
                    temp_root: temp_root.clone(),
                });
                self.open_folder(temp_root); // not an archive path → normal scan
                self.status = format!("Opened {}", short_name(&archive));
            }
            Err(e) => self.status = format!("Couldn't open {}: {e}", short_name(&archive)),
        }
    }

    /// Map a real path (possibly inside the mounted archive's temp dir) to the path
    /// shown to the user — the archive *file* stands in for its temp dir, so the
    /// breadcrumb reads `…/pack.zip/sub` instead of a temp hash.
    fn to_display(&self, real: &Path) -> PathBuf {
        if let Some(m) = &self.archive_mount {
            if let Ok(rel) = real.strip_prefix(&m.temp_root) {
                return m.archive.join(rel);
            }
        }
        real.to_path_buf()
    }

    /// Inverse of [`to_display`]: map a user-facing path back to the real on-disk path.
    fn real_path(&self, disp: &Path) -> PathBuf {
        if let Some(m) = &self.archive_mount {
            if let Ok(rel) = disp.strip_prefix(&m.archive) {
                return m.temp_root.join(rel);
            }
        }
        disp.to_path_buf()
    }

    /// The stable view-history key for a path (its display identity as a string), so a
    /// piece inside a zip / on 16colo.rs is tracked across re-extraction / re-download.
    fn view_key(&self, path: &Path) -> String {
        self.to_display(path).to_string_lossy().into_owned()
    }

    /// Record one view of `path` (open in viewer, or open a folder/pack) — bumps its
    /// count + last-viewed time. No-op if the view db couldn't be opened.
    fn mark_viewed(&mut self, path: &Path) {
        let key = self.view_key(path);
        let now = unix_now();
        if let Some(db) = self.viewdb.as_mut() {
            db.record(&key, now);
        }
    }

    /// True if `path` has been viewed at least once.
    fn is_viewed(&self, path: &Path) -> bool {
        self.viewdb
            .as_ref()
            .is_some_and(|db| db.is_viewed(&self.view_key(path)))
    }

    /// Any network request in flight (16colo listing / pack download / piece open /
    /// file save / SAUCE fetch / random pack). Drives the status-bar busy spinner.
    fn net_busy(&self) -> bool {
        self.remote_rx.is_some()
            || self.colo_rx.is_some()
            || self.colo_open_rx.is_some()
            || self.colo_save_rx.is_some()
            || self.random_rx.is_some()
            || self.colo_sauce_pending > 0
    }

    /// Full view record (count + first/last) for `path`, if tracked.
    fn view_record(&self, path: &Path) -> Option<crate::viewdb::ViewRecord> {
        self.viewdb.as_ref().and_then(|db| db.get(&self.view_key(path)))
    }

    /// Pin the current folder to Places (used inside a 16colo flat listing, whose rows
    /// are pieces, so the dir "Pin" doesn't apply). A pinned artist/group/search path
    /// re-runs that listing when clicked.
    fn pin_current_folder(&mut self) {
        if let Some(f) = self.folder.clone() {
            if !self.favorites.contains(&f) {
                let name = short_name(&f);
                self.favorites.push(f);
                self.status = format!("Pinned “{name}” to Places");
            }
        }
    }

    /// Manually toggle visited state for `path` (grid/table context menu).
    fn set_viewed(&mut self, path: &Path, viewed: bool) {
        let key = self.view_key(path);
        let now = unix_now();
        if let Some(db) = self.viewdb.as_mut() {
            db.set_viewed(&key, viewed, now);
        }
    }

    /// Resolve the star rating for a *real* entry path. Virtual art (inside an
    /// archive / 16colo.rs — i.e. its display path differs from disk) reads the
    /// sidecar; a plain on-disk file reads its `user.baloo.rating` xattr (so external
    /// Gwenview edits win), falling back to the sidecar where xattrs aren't available.
    fn read_rating(&self, real: &Path) -> u8 {
        let disp = self.to_display(real);
        if disp != *real {
            self.ratings.get(&disp)
        } else {
            match crate::rating::read(real) {
                0 => self.ratings.get(real),
                stars => stars,
            }
        }
    }

    /// Persist a star rating for a *real* entry path. Virtual art goes to the sidecar
    /// only; an on-disk file writes the xattr (Gwenview interop) **and** mirrors into
    /// the sidecar (a portable record + the fallback on non-xattr platforms). Always
    /// succeeds — the sidecar write can't fail the way an xattr write can.
    fn set_rating(&mut self, real: &Path, stars: u8) {
        let disp = self.to_display(real);
        if disp != *real {
            self.ratings.set(&disp, stars);
        } else {
            let _ = crate::rating::write(real, stars);
            self.ratings.set(real, stars);
        }
        self.ratings.save();
    }

    /// Rebuild `entries` from `all_entries`: apply the rating filter, then sort
    /// (directories optionally pinned first, ascending/descending otherwise).
    fn rebuild_view(&mut self) {
        // In advanced-search mode the grid renders the recursive results, not the
        // current folder; the same sort/filter pipeline applies either way.
        let src: &[Entry] = match &self.search_results {
            Some(r) => r,
            None => &self.all_entries,
        };
        self.entries = sorted_filtered_view(
            src,
            self.sort_key,
            self.sort_desc,
            self.dirs_first,
            self.min_rating,
            self.search.as_deref(),
            &self.img_meta,
            &self.colo_pieces,
        );
    }

    /// Kick off a recursive image search from the current folder on a background
    /// thread. Results stream into `search_results` (see [`poll_search`]).
    /// Filter the currently-loaded entries (a 16colo.rs flat listing / remote view) by
    /// `search_spec`, in memory — there's no on-disk tree to walk. Evaluates the fields
    /// we can from memory: filename, extension, size (after the pack backfill), rating,
    /// and SAUCE (the piece's artist/group + any cached SAUCE title/author/group/font).
    /// Dimensions / colours / modified-date aren't known for virtual pieces, so those
    /// filters are simply ignored here rather than rejecting everything.
    fn colo_filter_in_memory(&self) -> Vec<Entry> {
        let spec = &self.search_spec;
        let name_q = spec.name.trim().to_ascii_lowercase();
        let exts: Vec<String> = spec
            .ext
            .split([',', ' ', ';'])
            .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let sauce_q = spec.sauce.trim().to_ascii_lowercase();
        let (smin, smax) = (parse_dim(&spec.smin), parse_dim(&spec.smax)); // KB
        let rmin = parse_dim(&spec.rmin).unwrap_or(0).min(5) as u8;
        self.all_entries
            .iter()
            .filter(|e| {
                let fname = e
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !name_q.is_empty() {
                    // "Find in this listing" matches the visible row text — filename plus
                    // the scene columns (artist / group / pack) — so e.g. Ctrl+F "tainted"
                    // finds artist tainted's pieces, whose filenames are e.g. "67C21.ANS".
                    let mut hay = fname.clone();
                    if let Some(p) = self.colo_pieces.get(&e.path) {
                        for f in [&p.artist, &p.group, &p.pack] {
                            hay.push(' ');
                            hay.push_str(&f.to_ascii_lowercase());
                        }
                    }
                    if !hay.contains(&name_q) {
                        return false;
                    }
                }
                if !exts.is_empty() {
                    let ext = e
                        .path
                        .extension()
                        .and_then(|x| x.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if !exts.contains(&ext) {
                        return false;
                    }
                }
                if smin.is_some() || smax.is_some() {
                    let kb = e.size / 1024;
                    if smin.is_some_and(|v| kb < v as u64) || smax.is_some_and(|v| kb > v as u64) {
                        return false;
                    }
                }
                if rmin > 0 && e.rating < rmin {
                    return false;
                }
                if !sauce_q.is_empty() {
                    // The artist/group columns + any cached SAUCE are the searchable text
                    // for a virtual piece (its raw file isn't on disk to parse).
                    let mut hay = String::new();
                    if let Some(p) = self.colo_pieces.get(&e.path) {
                        hay.push_str(&p.artist.to_ascii_lowercase());
                        hay.push(' ');
                        hay.push_str(&p.group.to_ascii_lowercase());
                    }
                    if let Some(Some(s)) = self.sauce_cache.get(&e.path) {
                        for f in [&s.title, &s.author, &s.group, &s.font] {
                            hay.push(' ');
                            hay.push_str(&f.to_ascii_lowercase());
                        }
                    }
                    if !hay.contains(&sauce_q) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }

    fn start_search(&mut self) {
        let Some(root) = self.folder.clone() else {
            return;
        };
        if self.search_spec.is_blank() {
            self.status = "Enter at least one search field".into();
            return;
        }
        // A 16colo.rs listing (or any remote view) has no on-disk tree to walk, so filter
        // the pieces already loaded into `all_entries` in memory instead of crawling.
        if crate::sixteen::is_remote(&root) || self.colo_flat {
            self.cancel_search();
            let results = self.colo_filter_in_memory();
            let n = results.len();
            self.search_root = Some(root);
            self.search_results = Some(results);
            self.search_running = false;
            self.status = format!("{n} match(es) in this listing");
            self.mode = Mode::Grid;
            self.selection.clear();
            self.rebuild_view();
            return;
        }
        self.cancel_search(); // stop any prior run
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let (spec, registry, c) = (
            self.search_spec.clone(),
            Arc::clone(&self.registry),
            Arc::clone(&cancel),
        );
        self.search_root = Some(root.clone());
        let sidecar = self.ratings.snapshot();
        std::thread::spawn(move || search_walk(root, spec, registry, c, tx, sidecar));
        self.search_rx = Some(rx);
        self.search_cancel = Some(cancel);
        self.search_results = Some(Vec::new());
        self.search_running = true;
        self.status = "Searching…".into();
        self.mode = Mode::Grid;
        self.selection.clear();
        self.rebuild_view();
    }

    /// Save the current criteria as a named "smart filter" (deduped by name).
    fn save_filter(&mut self) {
        if self.search_spec.is_blank() {
            self.status = "Nothing to save — enter a search first".into();
            return;
        }
        let name = self.search_spec.summary();
        let spec = self.search_spec.clone();
        if let Some(slot) = self.saved_filters.iter_mut().find(|(n, _)| n == &name) {
            slot.1 = spec; // overwrite a same-named filter rather than duplicate
        } else {
            self.saved_filters.push((name.clone(), spec));
        }
        self.status = format!("Saved filter: {name}");
    }

    /// Recall a saved filter: load its criteria and run it.
    fn recall_filter(&mut self, i: usize) {
        if let Some((_, spec)) = self.saved_filters.get(i) {
            self.search_spec = spec.clone();
            self.show_search = true;
            self.start_search();
        }
    }

    /// Seed a fresh recursive search from one attribute of `entry` ("Smart filter on…"
    /// in the grid context menu), then run it from the current folder.
    fn smart_filter_from(&mut self, entry: &Entry, crit: SmartCriterion) {
        let mut spec = SearchSpec::default();
        let sauce_field = |pick: fn(crate::sauce::Sauce) -> String| -> String {
            read_file_tail(&entry.path, 128)
                .as_deref()
                .and_then(crate::sauce::parse)
                .map(pick)
                .unwrap_or_default()
        };
        match crit {
            SmartCriterion::Type => {
                spec.ext = entry
                    .path
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or_default()
                    .to_string();
            }
            SmartCriterion::Name => {
                // First "word" of the stem (≥3 chars) — finds siblings/versions.
                let stem = entry
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                spec.name = stem
                    .split(|c: char| !c.is_alphanumeric())
                    .find(|w| w.len() >= 3)
                    .unwrap_or(stem)
                    .to_string();
            }
            SmartCriterion::Size => {
                // ±20% of this file's size, in KB → "find similarly-sized files".
                let kb = (entry.size as f64 / 1024.0).max(1.0);
                spec.smin = ((kb * 0.8).floor() as u64).to_string();
                spec.smax = ((kb * 1.2).ceil() as u64).to_string();
            }
            SmartCriterion::Date => {
                if let Some(d) = entry.mtime.map(date_ymd) {
                    spec.dfrom = d.clone();
                    spec.dto = d;
                }
            }
            SmartCriterion::Rating => spec.rmin = entry.rating.max(1).to_string(),
            SmartCriterion::Group => spec.sauce = sauce_field(|s| s.group),
            SmartCriterion::Artist => spec.sauce = sauce_field(|s| s.author),
        }
        self.search_spec = spec;
        self.show_search = true;
        self.start_search();
    }

    /// The folder a search result lives in, relative to the search root — shown as an
    /// extra result-tile caption line. `📁 ·` means it's directly in the search root.
    fn result_folder_label(&self, path: &Path) -> String {
        let parent = path.parent().unwrap_or(path);
        let rel = self
            .search_root
            .as_deref()
            .and_then(|root| parent.strip_prefix(root).ok())
            .unwrap_or(parent);
        let s = rel.to_string_lossy();
        format!("📁 {}", if s.is_empty() { "·" } else { &s })
    }

    /// Signal the running search thread to stop and forget its channel.
    fn cancel_search(&mut self) {
        if let Some(c) = self.search_cancel.take() {
            c.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.search_rx = None;
        self.search_running = false;
    }

    /// Close the search panel and return the grid to the current folder.
    fn close_search(&mut self) {
        self.cancel_search();
        self.show_search = false;
        self.search_results = None;
        self.rebuild_view();
    }

    /// Drain freshly-found search hits each frame and refresh the results grid.
    fn poll_search(&mut self) {
        use std::sync::mpsc::TryRecvError;
        if self.search_rx.is_none() {
            return;
        }
        let mut hits: Vec<Entry> = Vec::new();
        let mut done: Option<usize> = None;
        if let Some(rx) = self.search_rx.as_ref() {
            for _ in 0..512 {
                match rx.try_recv() {
                    Ok(SearchMsg::Hit(e)) => hits.push(e),
                    Ok(SearchMsg::Done(n)) => {
                        done = Some(n);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        done = Some(self.search_results.as_ref().map_or(0, Vec::len));
                        break;
                    }
                }
            }
        }
        let got = !hits.is_empty();
        for mut e in hits {
            e.rating = self.read_rating(&e.path); // resolve stars (xattr / sidecar)
            self.search_results.get_or_insert_with(Vec::new).push(e);
        }
        if let Some(n) = done {
            self.search_running = false;
            self.search_rx = None;
            self.search_cancel = None;
            self.status = format!("{n} result{}", if n == 1 { "" } else { "s" });
            self.rebuild_view();
        } else if got {
            self.rebuild_view();
            self.want_repaint = true;
        } else {
            self.want_repaint = true; // keep polling while the thread runs
        }
    }

    /// The key bound to an action (falls back to its default).
    fn key_for(&self, a: Action) -> egui::Key {
        self.keymap
            .get(&a)
            .copied()
            .unwrap_or_else(|| a.default_key())
    }

    /// Navigate the folder history (mouse back/forward in the grid).
    fn go_history(&mut self, back: bool) {
        let target = if back {
            self.dir_pos.checked_sub(1)
        } else if self.dir_pos + 1 < self.dir_history.len() {
            Some(self.dir_pos + 1)
        } else {
            None
        };
        if let Some(pos) = target {
            let dir = self.dir_history[pos].clone();
            self.dir_pos = pos;
            self.suppress_history = true; // don't re-record this hop
            self.open_folder(dir);
        }
    }

    /// Move the grid selection to `idx` and scroll it into view (Home / End).
    fn select_index(&mut self, idx: usize) {
        if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
            self.selection.clear();
            self.selection.insert(p);
            self.anchor = Some(idx);
            self.hovered = Some(idx);
            self.scroll_target = Some(idx);
        }
    }

    // ----- File operations (round 4) -------------------------------------------
    // All triggers (keyboard / Edit menu / right-click) funnel through these.
    // Deletes go to the system trash; every mutating op pushes a reversible
    // `UndoOp` so Ctrl+Z can walk it back. After each op we `refresh()` the view.

    /// The paths an operation acts on: the multi-selection, or — if nothing is
    /// selected — the hovered tile (so a right-click on an unselected tile works).
    fn op_targets(&self) -> Vec<PathBuf> {
        if !self.selection.is_empty() {
            self.selection.iter().cloned().collect()
        } else if let Some(e) = self.hovered.and_then(|i| self.entries.get(i)) {
            vec![e.path.clone()]
        } else {
            Vec::new()
        }
    }

    /// Re-scan the current folder after a file op, without recording history.
    fn refresh(&mut self) {
        if let Some(f) = self.folder.clone() {
            self.suppress_history = true;
            self.open_folder(f);
        }
    }

    fn copy_selection(&mut self) {
        let t = self.op_targets();
        if !t.is_empty() {
            self.status = format!("Copied {} item(s) — Ctrl+V to paste", t.len());
            self.clipboard = Some((t, false));
        }
    }

    fn cut_selection(&mut self) {
        let t = self.op_targets();
        if !t.is_empty() {
            self.status = format!("Cut {} item(s) — Ctrl+V to paste", t.len());
            self.clipboard = Some((t, true));
        }
    }

    fn paste(&mut self) {
        let Some(dest) = self.folder.clone() else {
            return;
        };
        let Some((paths, is_cut)) = self.clipboard.clone() else {
            return;
        };
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut created: Vec<PathBuf> = Vec::new();
        let mut errs = 0usize;
        for src in &paths {
            let Some(name) = src.file_name() else {
                continue;
            };
            let target = dedup_path(&dest, name);
            let res = if is_cut {
                move_path(src, &target)
            } else {
                copy_recursive(src, &target)
            };
            match res {
                Ok(()) if is_cut => moved.push((target, src.clone())),
                Ok(()) => created.push(target),
                Err(_) => errs += 1,
            }
        }
        if is_cut {
            if !moved.is_empty() {
                self.undo_stack.push(UndoOp::Move(moved.clone()));
            }
            self.clipboard = None; // a cut is consumed by the paste
        } else if !created.is_empty() {
            self.undo_stack.push(UndoOp::PasteCopy(created.clone()));
        }
        let n = moved.len() + created.len();
        let msg = if errs > 0 {
            format!("Pasted {n} item(s); {errs} failed")
        } else {
            format!("Pasted {n} item(s)")
        };
        self.refresh();
        self.status = msg;
    }

    fn new_folder(&mut self) {
        let Some(dir) = self.folder.clone() else {
            return;
        };
        let target = dedup_path(&dir, std::ffi::OsStr::new("New Folder"));
        match std::fs::create_dir(&target) {
            Ok(()) => {
                self.undo_stack.push(UndoOp::NewFolder(target.clone()));
                self.refresh();
                self.status = format!("Created {}", short_name(&target));
                // Drop straight into renaming it (Dolphin-like).
                if let Some(name) = target.file_name().and_then(|n| n.to_str()) {
                    self.renaming = Some((target.clone(), name.to_string()));
                    self.focus_rename = true;
                }
            }
            Err(e) => self.status = format!("New folder failed: {e}"),
        }
    }

    fn start_rename(&mut self) {
        if let Some(path) = self.op_targets().into_iter().next() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                self.renaming = Some((path.clone(), name.to_string()));
                self.focus_rename = true;
            }
        }
    }

    fn apply_rename(&mut self, new_name: &str) {
        let Some((old, _)) = self.renaming.take() else {
            return;
        };
        let new_name = new_name.trim();
        let cur = old.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if new_name.is_empty() || new_name == cur {
            return;
        }
        if new_name.contains(['/', '\\']) {
            self.status = "Name can't contain a path separator".into();
            return;
        }
        let Some(parent) = old.parent() else { return };
        let target = parent.join(new_name);
        if target.exists() {
            self.status = format!("'{new_name}' already exists");
            return;
        }
        match std::fs::rename(&old, &target) {
            Ok(()) => {
                self.undo_stack.push(UndoOp::Move(vec![(target, old)]));
                self.refresh();
                self.status = format!("Renamed to {new_name}");
            }
            Err(e) => self.status = format!("Rename failed: {e}"),
        }
    }

    fn delete_selection(&mut self) {
        let t = self.op_targets();
        if t.is_empty() {
            return;
        }
        let n = t.len();
        // Snapshot the trash so we can identify exactly what we just added and
        // hand those items to Undo for restoration.
        #[cfg(not(target_os = "macos"))]
        let before: Vec<OsString> = trash::os_limited::list()
            .unwrap_or_default()
            .into_iter()
            .map(|i| i.id)
            .collect();
        match trash::delete_all(&t) {
            Ok(()) => {
                #[cfg(not(target_os = "macos"))]
                {
                    let before: std::collections::HashSet<OsString> = before.into_iter().collect();
                    let want: std::collections::HashSet<PathBuf> = t.iter().cloned().collect();
                    if let Ok(after) = trash::os_limited::list() {
                        let restorable: Vec<trash::TrashItem> = after
                            .into_iter()
                            .filter(|i| {
                                !before.contains(&i.id) && want.contains(&i.original_path())
                            })
                            .collect();
                        if !restorable.is_empty() {
                            self.undo_stack.push(UndoOp::Trash(restorable));
                        }
                    }
                }
                self.selection.clear();
                self.refresh();
                self.status = format!("Moved {n} item(s) to trash — Ctrl+Z to undo");
            }
            Err(e) => self.status = format!("Trash failed: {e}"),
        }
    }

    fn undo(&mut self) {
        let Some(op) = self.undo_stack.pop() else {
            self.status = "Nothing to undo".into();
            return;
        };
        let msg = match op {
            UndoOp::Trash(items) => {
                #[cfg(not(target_os = "macos"))]
                {
                    let n = items.len();
                    match trash::os_limited::restore_all(items) {
                        Ok(()) => format!("Restored {n} item(s) from trash"),
                        Err(e) => format!("Restore failed: {e}"),
                    }
                }
                #[cfg(target_os = "macos")]
                {
                    let _ = items;
                    "Undo of trash isn't supported on this platform".to_string()
                }
            }
            UndoOp::Move(pairs) => {
                // Move each item back to where it came from.
                let mut ok = 0;
                for (cur, orig) in &pairs {
                    if move_path(cur, orig).is_ok() {
                        ok += 1;
                    }
                }
                format!("Reverted {ok} move(s)")
            }
            UndoOp::NewFolder(p) => {
                let _ = std::fs::remove_dir_all(&p);
                format!("Removed {}", short_name(&p))
            }
            UndoOp::PasteCopy(paths) => {
                let _ = trash::delete_all(&paths);
                format!("Removed {} pasted copy(ies)", paths.len())
            }
        };
        self.refresh();
        self.status = msg;
    }

    /// Dispatch a file operation requested from a context menu / Edit menu.
    fn do_file_action(&mut self, a: FileAction) {
        match a {
            FileAction::Copy => self.copy_selection(),
            FileAction::Cut => self.cut_selection(),
            FileAction::Paste => self.paste(),
            FileAction::Rename => self.start_rename(),
            FileAction::Delete => self.delete_selection(),
            FileAction::NewFolder => self.new_folder(),
        }
    }

    /// The baud rate that applies to the currently-open stream art (RIP and ANSI keep
    /// independent speeds).
    fn current_baud(&self) -> Baud {
        let is_rip = self.player.as_ref().is_some_and(|p| p.stream.is_rip());
        if is_rip {
            self.baud_rip
        } else {
            self.baud_ansi
        }
    }

    fn load_full(&mut self, ctx: &egui::Context, path: PathBuf) {
        let already = self
            .full_tex
            .as_ref()
            .map(|(p, _)| p == &path)
            .unwrap_or(false)
            || self.anim.as_ref().map(|a| a.path == path).unwrap_or(false);
        if already {
            return;
        }
        // Opening an image in the viewer counts as a view (bumps its count + last-viewed).
        self.mark_viewed(&path);
        self.osd_t = 0.0; // restart the metadata OSD fade-in for the new image
        self.osd_dismissed = false; // a fresh image un-hides the OSD ([×] is per-view only)
        let _ = self.cached_sauce(&path); // populate SAUCE (columns/lines/font/comment) for the OSD
        // `path` is the display identity (kept for stepping/ratings/full_tex keys); a
        // downloaded 16colo piece reads its bytes from the local cache file instead.
        let src = self.resolve_local(&path);
        // Pick the remembered zoom for this image's kind: text-mode art (tiny 8×16
        // cells) opens at its own larger default, raster art at its own. Sticky fit,
        // when on, overrides both and re-fits the window.
        self.viewing_textmode = is_textmode_ext(&path);
        if self.fit_mode {
            self.fit_requested = true;
        } else if self.auto_next && !self.viewing_textmode {
            // Slideshow: fit RIP / raster art to the screen so the whole thing is visible
            // for its brief moment (text-mode keeps its readable zoom + fit-to-width).
            self.fit_requested = true;
        } else if self.viewing_textmode {
            // textmode_zoom is in device pixels per source pixel; convert to logical.
            self.zoom = self.textmode_zoom / ctx.pixels_per_point();
            // …but cap it to fit the viewport width once the size is known, so a wide
            // (landscape) ANSI opens fully visible instead of clipped off the sides.
            self.fit_width_on_open = true;
        } else {
            self.zoom = self.raster_zoom;
        }
        self.anim = None;
        self.player = None;
        self.full_tex = None;
        self.full_src = None;
        self.full_reduced = None;
        self.minimap = None; // rebuilt for the new image at its first draw
                             // Zoom was set per-kind above (persistent within raster / text-mode);
                             // re-center the pan for the freshly loaded image.
        self.offset = egui::Vec2::ZERO;
        self.view_to_top = true; // start at the top-left, not centered, for tall art
        self.auto_next_dwell = 0.0; // restart the slideshow dwell for the new file

        // Animated GIF → upload every frame as a texture for playback (no size cap:
        // the user explicitly opened this one).
        if is_gif(&path) {
            if let Some(mut a) = build_anim(ctx, &src, usize::MAX) {
                a.path = path.clone(); // keep the virtual identity for `already` + stepping
                self.anim = Some(a);
                return;
            }
        }

        // Baud-rate playback for stream art (ANSImation / "watch RIP draw"). The static
        // decode below still builds full_tex/full_src for the minimap, recolor and
        // palette panes; the player only drives the main view while it's animating.
        self.player = std::fs::read(&src)
            .ok()
            .and_then(|bytes| Stream::for_file(&bytes, &path))
            .map(|stream| {
                // RIP and ANSImation each remember their own baud (a RIP scene draws very
                // differently from ANSI typing out).
                let baud = if stream.is_rip() {
                    self.baud_rip
                } else {
                    self.baud_ansi
                };
                Player::new(path.clone(), stream, baud != Baud::None)
            });

        // Static image.
        match self.registry.decode_path(&src) {
            Ok(img) => {
                let size = [img.width as usize, img.height as usize];
                let rgba = img.rgba_bytes();
                // NEAREST_REPEAT so the Tile (wallpaper) view can wrap a single
                // texture. Tiled so an over-8192 image still uploads at full res.
                let tex = TiledTexture::from_rgba(
                    ctx,
                    &path.to_string_lossy(),
                    size,
                    &rgba,
                    egui::TextureOptions::NEAREST_REPEAT,
                );
                // Binary scene formats (XBin/BIN/Tundra/IDF/ADF/PETSCII) aren't byte
                // streams, so `for_file` gave no player — drive a cell-reveal "typeout"
                // off the decoded image instead, so they animate at the baud rate too.
                if self.player.is_none() && is_textmode_ext(&path) {
                    let (cw, ch) = textmode_cell(&path);
                    let reveal = CellReveal::new(img.pixels.clone(), size[0], size[1], cw, ch);
                    if reveal.cells > 1 {
                        // Show binary art *instantly* on open (autoplay=false → starts at
                        // the end, fully revealed). The baud picker + ▶/Replay controls
                        // then let you watch it "type out" on demand — unlike ANSI, which
                        // auto-plays, these always rendered instantly, so keep that.
                        self.player =
                            Some(Player::new(path.clone(), Stream::Cells(reveal), false));
                    }
                }
                // Keep the CPU pixels so the optional palette reduction can remap
                // the full image without re-decoding.
                self.full_src = Some((path.clone(), size, rgba));
                self.full_tex = Some((path, tex));
            }
            Err(e) => self.status = format!("decode failed: {e}"),
        }
    }

    /// Re-decode the image currently in the viewer in place, keeping zoom/pan.
    /// Used when a render preference (e.g. the 9px VGA cell) changes and the
    /// cached texture + CPU pixels need rebuilding without re-opening the file.
    fn redecode_full(&mut self, ctx: &egui::Context) {
        let Some(path) = self
            .full_tex
            .as_ref()
            .map(|(p, _)| p.clone())
            .or_else(|| self.full_src.as_ref().map(|(p, _, _)| p.clone()))
        else {
            return;
        };
        // Decode the *real* file (a 16colo piece's bytes live at a cache path keyed by the
        // virtual display path), keeping `path` as the stored identity — same split as
        // `load_full`. Without this, re-decoding a 16colo/virtual piece tried to read the
        // virtual path off disk, failed, and left the view unchanged (9px toggle no-op).
        let src = self.resolve_local(&path);
        match self.registry.decode_path(&src) {
            Ok(img) => {
                let size = [img.width as usize, img.height as usize];
                let rgba = img.rgba_bytes();
                let tex = TiledTexture::from_rgba(
                    ctx,
                    &path.to_string_lossy(),
                    size,
                    &rgba,
                    egui::TextureOptions::NEAREST_REPEAT,
                );
                self.full_src = Some((path.clone(), size, rgba));
                self.full_tex = Some((path, tex));
                self.full_reduced = None; // width changed → drop any palette-reduced copy
                self.minimap = None; // pixels changed → rebuild the minimap from them
            }
            Err(e) => self.status = format!("decode failed: {e}"),
        }
    }

    /// Click/open an entry: descend into folders, open images in the single view.
    fn activate(&mut self, ctx: &egui::Context, idx: usize) {
        let Some(entry) = self.entries.get(idx).cloned() else {
            return;
        };
        if entry.is_dir || entry.is_archive {
            self.open_folder(entry.path); // archives route to enter_archive
        } else if self.colo_pieces.contains_key(&entry.path)
            && !self.colo_files.contains_key(&entry.path)
        {
            // A 16colo flat-listing piece not yet downloaded → fetch its single `raw`
            // file, then open it once ready (mode flips in `poll_colo_open`).
            self.selected = idx;
            self.start_piece_open(entry.path);
        } else {
            self.selected = idx;
            self.load_full(ctx, entry.path);
            self.mode = Mode::Single;
        }
    }

    /// In the single view, step to the previous/next *image* entry, skipping folders.
    fn step_image(&mut self, ctx: &egui::Context, forward: bool) {
        let n = self.entries.len();
        let mut i = self.selected as isize;
        loop {
            i += if forward { 1 } else { -1 };
            if i < 0 || i as usize >= n {
                return;
            }
            let e = &self.entries[i as usize];
            if !e.is_dir && !e.is_archive {
                self.activate(ctx, i as usize);
                return;
            }
        }
    }

    /// Dolphin-style breadcrumb bar: clickable path segments, with a "✎" toggle
    /// that swaps in an editable text field (Enter navigates).
    fn ui_breadcrumbs(&mut self, ui: &mut egui::Ui) {
        let Some(folder) = self.folder.clone() else {
            return;
        };
        // Inside a mounted archive, show the archive's path (…/pack.zip/sub), not the
        // temp dir; clicks/edits map back to real paths via `real_path`.
        let disp = self.to_display(&folder);
        ui.horizontal(|ui| {
            if let Some(mut text) = self.path_edit.take() {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .desired_width(f32::INFINITY)
                        .hint_text("Type a folder path — Enter to go, Esc to cancel"),
                );
                if self.focus_path {
                    resp.request_focus();
                    self.focus_path = false;
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter {
                    let p = self.real_path(&PathBuf::from(text.trim()));
                    if p.is_dir() {
                        self.open_folder(p);
                    } else {
                        self.status = format!("Not a folder: {text}");
                    }
                    // leave edit mode (path_edit already taken -> stays None)
                } else if resp.lost_focus() {
                    // clicked away / Esc — cancel, stay in breadcrumb mode
                } else {
                    self.path_edit = Some(text); // still editing
                }
            } else {
                if ui.button("✎").on_hover_text("Edit path").clicked() {
                    self.path_edit = Some(disp.to_string_lossy().into_owned());
                    self.focus_path = true;
                }
                ui.separator();
                let crumbs: Vec<PathBuf> = disp
                    .ancestors()
                    .filter(|p| !p.as_os_str().is_empty()) // virtual paths end in ""
                    .map(|p| p.to_path_buf())
                    .collect();
                for (i, a) in crumbs.iter().rev().enumerate() {
                    if i > 0 {
                        ui.label("›");
                    }
                    let label = a
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| a.to_string_lossy().into_owned());
                    // Show the virtual 16colo.rs root by its friendly name.
                    let label = if label == crate::sixteen::ROOT {
                        "16colo.rs".to_string()
                    } else {
                        label
                    };
                    if ui.button(label).clicked() {
                        let real = self.real_path(a);
                        self.open_folder(real);
                    }
                }
            }
        });
    }

    /// 16colo.rs quick-jump bar: the Years list + the Latest / Groups / Artists
    /// sub-roots. The active facet is highlighted (works inside a mounted pack too,
    /// keyed off the mount's virtual display path).
    fn ui_colo_nav(&mut self, ui: &mut egui::Ui) {
        use crate::sixteen;
        let root = Path::new(sixteen::ROOT);
        // The virtual path that places us in the tree: the folder if it's remote, else
        // the mounted pack's display path (when browsing inside a downloaded pack).
        let ctx_path = self
            .folder
            .clone()
            .filter(|p| sixteen::is_remote(p))
            .or_else(|| self.archive_mount.as_ref().map(|m| m.archive.clone()));
        let parts = ctx_path
            .as_deref()
            .map(sixteen::rel_parts)
            .unwrap_or_default();
        let first = parts.first().map(String::as_str);
        let in_years = first.is_none_or(|s| s.parse::<u32>().is_ok());
        let mut nav: Option<PathBuf> = None;
        // Plain-text labels: the bundled egui font renders many emoji as tofu (see the
        // font gotcha in CLAUDE.md), so avoid 📅/👥/🎨 here.
        ui.horizontal(|ui| {
            ui.label("16colo.rs:");
            if ui.selectable_label(in_years, "Years").clicked() {
                nav = Some(root.to_path_buf());
            }
            if ui
                .selectable_label(first == Some(sixteen::LATEST), "Latest")
                .clicked()
            {
                nav = Some(root.join(sixteen::LATEST));
            }
            if ui
                .selectable_label(first == Some(sixteen::GROUPS), "Groups")
                .clicked()
            {
                nav = Some(root.join(sixteen::GROUPS));
            }
            if ui
                .selectable_label(first == Some(sixteen::ARTISTS), "Artists")
                .clicked()
            {
                nav = Some(root.join(sixteen::ARTISTS));
            }
            ui.separator();
            // The search is scoped to the active facet: on the Artists tab it matches
            // artist names only, on Groups it matches group names only, otherwise both.
            let scope = if first == Some(sixteen::ARTISTS) {
                Some("artist")
            } else if first == Some(sixteen::GROUPS) {
                Some("group")
            } else {
                None
            };
            let hint = match scope {
                Some("artist") => "search artists",
                Some("group") => "search groups",
                _ => "search artist / group",
            };
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.colo_search)
                    .desired_width(150.0)
                    .hint_text(hint),
            );
            let go = ui.small_button("🔍").clicked()
                || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go {
                let q = self.colo_search.trim();
                if !q.is_empty() {
                    // Scoped search adds a token segment (SEARCH/<scope>/<query>) so the
                    // dispatch can tell it from a plain both-fields SEARCH/<query>.
                    let base = root.join(sixteen::SEARCH);
                    nav = Some(match scope {
                        Some(s) => base.join(s).join(q),
                        None => base.join(q),
                    });
                }
            }
        });
        if let Some(p) = nav {
            self.open_folder(p);
        }
    }

    /// Pinned-folder buttons under the menu bar. Click to jump; drag to reorder;
    /// right-click to remove.
    fn ui_favorites(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Favorites:");
            let can_pin = self
                .folder
                .as_ref()
                .is_some_and(|f| !self.favorites.contains(f));
            if ui
                .add_enabled(can_pin, egui::Button::new("★ Pin"))
                .clicked()
            {
                if let Some(f) = self.folder.clone() {
                    if !self.favorites.contains(&f) {
                        self.favorites.push(f);
                    }
                }
            }
            if let Some(p) = self.favorites_buttons(ui, "📁", |_| true) {
                self.open_folder(p);
            }
        });
    }

    /// Render the favorites as draggable, reorderable, right-click-removable
    /// buttons in whatever layout the caller set up (horizontal in the toolbar,
    /// vertical in the Places dock). Returns a folder to navigate to, if clicked.
    /// Render the favorite buttons matching `filter` (so the Places dock can split them
    /// into Local vs 16colo.rs sub-tabs). Reorder/remove use the *global* favorite index,
    /// so filtering with `continue` keeps them correct.
    fn favorites_buttons(
        &mut self,
        ui: &mut egui::Ui,
        icon: &str,
        filter: impl Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        // Buttons must not wrap their text vertically near the right edge of
        // horizontal_wrapped (Extend = wrap whole buttons to the next row instead).
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let mut nav: Option<PathBuf> = None;
        let mut remove: Option<usize> = None;
        let mut reorder: Option<(usize, usize)> = None;
        let mut set_color: Option<(usize, Option<[u8; 3]>)> = None;
        for (i, fav) in self.favorites.iter().enumerate() {
            if !filter(fav) {
                continue;
            }
            let label = format!("{icon} {}", short_name(fav));
            let color = self.fav_colors.get(fav).copied();
            // ONE widget that senses click *and* drag, so egui can tell a click
            // (jump there) from a drag (reorder). A separate drag sensor over the
            // button — the previous approach — swallowed the click. Note: NOT
            // dnd_drag_source, whose scope breaks horizontal_wrapped's wrapping.
            let text: egui::WidgetText = match color {
                Some(c) => egui::RichText::new(label.as_str()).color(contrast_text(c)).into(),
                None => label.as_str().into(),
            };
            let mut btn = egui::Button::new(text).sense(egui::Sense::click_and_drag());
            if let Some(c) = color {
                btn = btn.fill(egui::Color32::from_rgb(c[0], c[1], c[2]));
            }
            let resp = ui.add(btn).on_hover_text(fav.to_string_lossy());
            if resp.dragged() {
                egui::DragAndDrop::set_payload(ui.ctx(), i);
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if resp.clicked() {
                nav = Some(fav.clone());
            }
            resp.context_menu(|ui| {
                if ui.button("✕ Remove from favorites").clicked() {
                    remove = Some(i);
                    ui.close();
                }
                ui.separator();
                if ui.button("✕ Clear color").clicked() {
                    set_color = Some((i, None));
                    ui.close();
                }
                // The ANSI32 swatches: sharp 18px squares, packed tight in an 8×4 grid.
                egui::Grid::new(("fav_color_grid", i))
                    .spacing([1.0, 1.0])
                    .min_col_width(0.0)
                    .show(ui, |ui| {
                        for (n, &c) in ansi32_palette().iter().enumerate() {
                            if n > 0 && n % 8 == 0 {
                                ui.end_row();
                            }
                            let (rect, resp) = ui
                                .allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
                            let col = egui::Color32::from_rgb(c[0], c[1], c[2]);
                            ui.painter().rect_filled(rect, 0.0, col); // 0 radius = square
                            let outline = if color == Some(c) {
                                egui::Stroke::new(2.0, egui::Color32::WHITE)
                            } else if resp.hovered() {
                                egui::Stroke::new(1.0, egui::Color32::WHITE)
                            } else {
                                egui::Stroke::new(1.0, egui::Color32::from_black_alpha(70))
                            };
                            ui.painter()
                                .rect_stroke(rect, 0.0, outline, egui::StrokeKind::Inside);
                            if resp.clicked() {
                                set_color = Some((i, Some(c)));
                                ui.close();
                            }
                        }
                    });
            });
            if let Some(src) = resp.dnd_release_payload::<usize>() {
                reorder = Some((*src, i));
            }
        }
        if let Some((i, c)) = set_color {
            if let Some(p) = self.favorites.get(i).cloned() {
                match c {
                    Some(col) => {
                        self.fav_colors.insert(p, col);
                    }
                    None => {
                        self.fav_colors.remove(&p);
                    }
                }
            }
        } else if let Some(i) = remove {
            if i < self.favorites.len() {
                let p = self.favorites.remove(i);
                self.fav_colors.remove(&p); // drop its color tag too
            }
        } else if let Some((from, to)) = reorder {
            reorder_favorites(&mut self.favorites, from, to);
        }
        nav
    }

    /// Sort & filter toolbar (Phase 5). Edits locals, then applies + rebuilds once
    /// (so the ComboBox closures never need to borrow `self`).
    fn ui_sortbar(&mut self, ui: &mut egui::Ui) {
        let mut key = self.sort_key;
        let mut desc = self.sort_desc;
        let mut dirs_first = self.dirs_first;
        let mut min_rating = self.min_rating;

        ui.horizontal(|ui| {
            // View toggle: thumbnail grid vs sortable table (an alternate layout of the
            // same browse mode). Two selectable labels read clearer than one glyph.
            ui.label("View:");
            if ui.selectable_label(!self.table_view, "Grid").clicked() {
                self.table_view = false;
            }
            if ui.selectable_label(self.table_view, "Table").clicked() {
                self.table_view = true;
            }
            ui.separator();
            ui.label("Sort:");
            let cr = egui::ComboBox::from_id_salt("sort_key")
                .selected_text(key.label())
                .show_ui(ui, |ui| {
                    for k in SortKey::COMMON {
                        ui.selectable_value(&mut key, k, k.label());
                    }
                });
            // Wheel-cycle within the common keys; a scene key (set via the table) maps
            // to its position if present, else stays put.
            let mut ki = SortKey::COMMON.iter().position(|&k| k == key).unwrap_or(0);
            if wheel_cycle(ui, &cr.response, &mut ki, SortKey::COMMON.len()) {
                key = SortKey::COMMON[ki];
            }
            if ui
                .button(if desc { "↓ Desc" } else { "↑ Asc" })
                .on_hover_text("Toggle ascending/descending")
                .clicked()
            {
                desc = !desc;
            }
            ui.checkbox(&mut dirs_first, "Dirs first");
            ui.separator();
            ui.label("Rating");
            let cr = egui::ComboBox::from_id_salt("min_rating")
                .selected_text(if min_rating == 0 {
                    "All".to_string()
                } else {
                    format!("≥ {min_rating}★")
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut min_rating, 0, "All");
                    for r in 1..=5u8 {
                        ui.selectable_value(&mut min_rating, r, format!("≥ {r}★"));
                    }
                });
            let mut ri = min_rating as usize;
            if wheel_cycle(ui, &cr.response, &mut ri, 6) {
                min_rating = ri as u8;
            }
        });

        if key != self.sort_key
            || desc != self.sort_desc
            || dirs_first != self.dirs_first
            || min_rating != self.min_rating
        {
            self.sort_key = key;
            self.sort_desc = desc;
            self.dirs_first = dirs_first;
            self.min_rating = min_rating;
            self.rebuild_view();
        }
    }

    /// Vim-style filename filter bar (opened with '/'); live-filters the grid.
    fn ui_searchbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("🔍 Filter:");
            let mut q = self.search.take().unwrap_or_default();
            let resp = ui.add(
                egui::TextEdit::singleline(&mut q)
                    .desired_width(260.0)
                    .hint_text("filename contains… (Esc to close)"),
            );
            if self.focus_search {
                resp.request_focus();
                self.focus_search = false;
            }
            // Esc-to-close is handled globally (see `ui`) so it works even when focus
            // isn't in this box; here we only need the explicit ✕ clear button.
            let clear = ui.button("✕").on_hover_text("Clear filter").clicked();
            let n = self.entries.len();
            ui.weak(format!("{n} match{}", if n == 1 { "" } else { "es" }));
            if clear {
                self.search = None;
                self.rebuild_view();
            } else {
                let changed = resp.changed();
                self.search = Some(q);
                if changed {
                    self.rebuild_view();
                }
            }
        });
    }

    /// Advanced recursive-search bar: criteria across the current folder + subfolders,
    /// streaming into a "Search results" grid. Enter (or the button) runs it.
    fn ui_search(&mut self, ui: &mut egui::Ui) {
        let mut go = false;
        let mut enter = false;
        let field = |ui: &mut egui::Ui, v: &mut String, w: f32, hint: &str| {
            let r = ui.add(
                egui::TextEdit::singleline(v)
                    .desired_width(w)
                    .hint_text(hint),
            );
            r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
        };
        ui.horizontal_wrapped(|ui| {
            ui.label("🔍 Find:");
            let first = ui.add(
                egui::TextEdit::singleline(&mut self.search_spec.name)
                    .desired_width(150.0)
                    .hint_text("filename"),
            );
            if self.focus_adv_search {
                first.request_focus();
                self.focus_adv_search = false;
            }
            enter |= first.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.label("type");
            enter |= field(ui, &mut self.search_spec.ext, 84.0, "png,ans");
            ui.label("W");
            enter |= field(ui, &mut self.search_spec.wmin, 46.0, "min");
            ui.label("–");
            enter |= field(ui, &mut self.search_spec.wmax, 46.0, "max");
            ui.label("H");
            enter |= field(ui, &mut self.search_spec.hmin, 46.0, "min");
            ui.label("–");
            enter |= field(ui, &mut self.search_spec.hmax, 46.0, "max");
            ui.label("Size");
            enter |= field(ui, &mut self.search_spec.smin, 50.0, "min");
            ui.label("–");
            enter |= field(ui, &mut self.search_spec.smax, 50.0, "max KB");
            ui.label("Date");
            enter |= field(ui, &mut self.search_spec.dfrom, 92.0, "from yyyy-mm-dd");
            ui.label("–");
            enter |= field(ui, &mut self.search_spec.dto, 92.0, "to");
            ui.label("★≥");
            enter |= field(ui, &mut self.search_spec.rmin, 28.0, "0");
            ui.label("SAUCE");
            enter |= field(ui, &mut self.search_spec.sauce, 130.0, "title/author…");
            if ui.button("Search").clicked() {
                go = true;
            }
            if ui
                .button("Save")
                .on_hover_text("Save these criteria as a smart filter (🔍 in Places)")
                .clicked()
            {
                self.save_filter();
            }
            if self.search_running {
                ui.spinner();
            }
            if let Some(r) = &self.search_results {
                let n = r.len();
                ui.weak(format!(
                    "{n} result{}{}",
                    if n == 1 { "" } else { "s" },
                    if self.search_running { "…" } else { "" }
                ));
            }
            if ui.button("✕").on_hover_text("Close search (Esc)").clicked() {
                self.close_search();
            }
        });
        if go || enter {
            self.start_search();
        }
    }

    /// Right dock: live details for the hovered (grid) or current (single) entry.
    /// Median-cut `full` to `quantize_n` colors, memoized by (path, N).
    fn reduce_palette(&mut self, path: &Path, full: &[[u8; 4]]) -> Vec<[u8; 4]> {
        let n = self.quantize_n.clamp(2, 256);
        let stale = self
            .quantize_cache
            .as_ref()
            .map(|(p, cn, _)| p != path || *cn != n)
            .unwrap_or(true);
        if stale {
            self.quantize_cache = Some((path.to_path_buf(), n, crate::thumb::median_cut(full, n)));
        }
        self.quantize_cache.as_ref().unwrap().2.clone()
    }

    /// Parse + cache a `.gpl` palette file. None if unreadable or empty.
    fn load_gpl(&mut self, path: &Path) -> Option<Vec<[u8; 4]>> {
        if let Some(p) = self.loaded_palettes.get(path) {
            return Some(p.clone());
        }
        // Real files on disk win; built-in (virtual) paths fall back to the
        // contents embedded in the binary.
        let text = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => builtin_palette_contents(path)?.to_string(),
        };
        let pal = crate::thumb::parse_gpl(&text);
        if pal.is_empty() {
            return None;
        }
        self.loaded_palettes.insert(path.to_path_buf(), pal.clone());
        Some(pal)
    }

    /// The per-channel additive offset for the Color-balance op: the picked color
    /// read as a ±offset around neutral grey (128), scaled by strength. Zero when
    /// neutral or strength 0, so the op no-ops.
    fn balance_offset(&self) -> [i16; 3] {
        let s = self.balance_strength.clamp(0.0, 1.0);
        if s <= 0.0 {
            return [0, 0, 0];
        }
        std::array::from_fn(|k| ((self.balance_color[k] as f32 - 128.0) * 2.0 * s).round() as i16)
    }

    /// Bundle the marker-op inputs (dither + balance + the snap palette) for a
    /// pipeline run. `palette` is the active recolor palette, or None.
    fn pipe_aux<'a>(&'a self, palette: Option<&'a [[u8; 4]]>) -> PipeAux<'a> {
        PipeAux {
            dither_method: self.dither_method,
            dither_amount: self.dither_amount,
            dither_custom: &self.dither_custom,
            dither_n: self.dither_custom_n,
            balance: self.balance_offset(),
            palette,
        }
    }

    /// Is *any* pipeline stage active (an adjustment, color balance, or dither)?
    /// Drives the "render a processed preview" gate and the "Adjustments *" marker.
    fn pipeline_active(&self) -> bool {
        !self.adjust.is_identity()
            || self.balance_offset() != [0, 0, 0]
            || (self.dither_method != 0 && self.dither_amount > 0.0)
    }

    /// A cache key for the whole adjustment pipeline (everything *except* the snap
    /// palette, which callers append as `rkey`). Folds in the dither method/amount
    /// (+ the custom matrix when it's selected) and the color-balance offset, so any
    /// of them changing invalidates the preview/grid/full caches.
    fn pipeline_key(&self) -> String {
        let off = self.balance_offset();
        let dsig = if self.dither_method == crate::thumb::DITHER_CUSTOM {
            format!("{}:{:?}", self.dither_custom_n, self.dither_custom)
        } else {
            String::new()
        };
        format!(
            "{}|D{}:{:.2}:{dsig}|B{},{},{}",
            self.adjust.key(),
            self.dither_method,
            self.dither_amount,
            off[0],
            off[1],
            off[2],
        )
    }

    /// The palette to remap the inspected image to right now, plus a cache key
    /// identifying it — or None for "show the original". Precedence: an explicitly
    /// selected GPL palette, then the Reduce-to-N median cut, then nothing. The
    /// dither/balance/adjust state is keyed separately via [`pipeline_key`].
    fn active_recolor(&mut self, path: &Path) -> Option<(String, Vec<[u8; 4]>)> {
        // A generated/edited palette (Random, or color edits) wins over everything.
        if let Some(cp) = self.custom_palette.clone() {
            return Some((format!("custom:{}", palette_hash(&cp)), cp));
        }
        if let Some(pp) = self.selected_palette.clone() {
            let pal = self.load_gpl(&pp)?;
            return Some((format!("pal:{}", pp.display()), pal));
        }
        if self.quantize_on {
            let full = self.palettes.get(path)?.clone()?;
            let reduced = self.reduce_palette(path, &full);
            return Some((format!("reduce:{}", self.quantize_n), reduced));
        }
        None
    }

    /// Cache key for the active *grid* recolor, or None if "Apply to grid" is off
    /// or nothing is selected.
    fn grid_recolor_key(&self) -> Option<String> {
        if !self.recolor_grid {
            return None;
        }
        let rkey = if let Some(cp) = &self.custom_palette {
            format!("custom:{}", palette_hash(cp))
        } else if let Some(pp) = &self.selected_palette {
            format!("pal:{}", pp.display())
        } else if self.quantize_on {
            format!("reduce:{}", self.quantize_n)
        } else {
            "orig".to_string()
        };
        // Nothing to do if there's neither a pipeline stage nor a recolor.
        if rkey == "orig" && !self.pipeline_active() {
            return None;
        }
        Some(format!("{}|{rkey}", self.pipeline_key()))
    }

    /// The palette to recolor grid tile `path` to: a fixed palette (Random/custom
    /// or a GPL swap) applies to every tile; Reduce median-cuts each tile's own
    /// colors to N.
    fn tile_palette(&mut self, path: &Path) -> Option<Vec<[u8; 4]>> {
        if let Some(cp) = &self.custom_palette {
            return Some(cp.clone());
        }
        if let Some(pp) = self.selected_palette.clone() {
            return self.load_gpl(&pp);
        }
        if self.quantize_on {
            let full = self.palettes.get(path)?.clone()?;
            return Some(crate::thumb::median_cut(&full, self.quantize_n));
        }
        None
    }

    /// A recolored thumbnail texture for grid tile `path`, built from the cached
    /// thumb pixels (no re-decode), memoized per (path, recolor `key`). None until
    /// the thumb is decoded.
    fn grid_recolored_tex(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        key: &str,
    ) -> Option<egui::TextureHandle> {
        if let Some((k, tex)) = self.grid_recolor.get(path) {
            if k == key {
                return Some(tex.clone());
            }
        }
        let (w, h, mut rgba) = self.thumb_rgba.get(path)?.clone();
        let palette = self.tile_palette(path);
        let aux = self.pipe_aux(palette.as_deref());
        apply_pipeline(&mut rgba, w, h, &self.adjust, &aux);
        let color = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
        // Match the plain-thumb path: a downscaled (area-averaged) tile displays
        // LINEAR so it isn't re-aliased; a source-res sprite stays crisp NEAREST.
        let downscaled = self
            .img_meta
            .get(path)
            .is_some_and(|m| m.w as usize > w || m.h as usize > h);
        let opts = if downscaled {
            egui::TextureOptions::LINEAR
        } else {
            egui::TextureOptions::NEAREST
        };
        let tex = ctx.load_texture("grid_recolor", color, opts);
        self.grid_recolor
            .insert(path.to_path_buf(), (key.to_string(), tex.clone()));
        Some(tex)
    }

    /// A highlight texture: the thumbnail (optionally recolored to `palette`) with
    /// every pixel that ISN'T `flash` dimmed, so a clicked swatch shows where its
    /// color lives. Rebuilt each frame while flashing (cheap for a thumbnail).
    fn make_flash_tex(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        palette: Option<Vec<[u8; 4]>>,
        flash: [u8; 4],
    ) -> Option<egui::TextureHandle> {
        let (w, h, mut rgba) = self.thumb_rgba.get(path)?.clone();
        // Flash highlights where a palette color is used — run adjustments + balance
        // + the plain snap, but skip dither (its noise would scatter the highlight).
        let aux = PipeAux {
            dither_method: 0,
            dither_amount: 0.0,
            dither_custom: &[],
            dither_n: 0,
            balance: self.balance_offset(),
            palette: palette.as_deref(),
        };
        apply_pipeline(&mut rgba, w, h, &self.adjust, &aux);
        for px in rgba.chunks_exact_mut(4) {
            if px[3] == 0 {
                continue;
            }
            if px[0] == flash[0] && px[1] == flash[1] && px[2] == flash[2] {
                continue; // keep the matching color bright
            }
            px[0] /= 5;
            px[1] /= 5;
            px[2] /= 5; // dim everything else
        }
        let color = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
        Some(ctx.load_texture("pv_flash", color, egui::TextureOptions::NEAREST))
    }

    /// A thumbnail texture of the inspected image remapped to `palette` (the live
    /// recolor preview), keyed by `key`. Source pixels are decoded once per path;
    /// the remapped texture is rebuilt only when the path or recolor changes.
    fn make_preview(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        key: &str,
        palette: Option<&[[u8; 4]]>,
    ) -> Option<egui::TextureHandle> {
        if let Some((p, k, tex)) = &self.preview_tex {
            if p == path && k == key {
                return Some(tex.clone());
            }
        }
        if self
            .preview_src
            .as_ref()
            .map(|(p, ..)| p != path)
            .unwrap_or(true)
        {
            let reg = self.registry.clone();
            let (w, h, rgba) = crate::thumb::decode_thumb(&reg, path, THUMB_PX)?;
            self.preview_src = Some((path.to_path_buf(), w, h, rgba));
        }
        let (w, h, mut rgba) = {
            let s = self.preview_src.as_ref().unwrap();
            (s.1, s.2, s.3.clone())
        };
        let aux = self.pipe_aux(palette);
        apply_pipeline(&mut rgba, w, h, &self.adjust, &aux);
        let color = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
        let tex = ctx.load_texture("pv_preview", color, egui::TextureOptions::NEAREST);
        self.preview_tex = Some((path.to_path_buf(), key.to_string(), tex.clone()));
        Some(tex)
    }

    /// A full-resolution texture of the inspected image remapped to `palette` — the
    /// viewer's recolor view. Reuses the full pixels cached by `load_full` (else
    /// decodes), and rebuilds only when the path or recolor (`key`) changes.
    fn make_full_reduced(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        key: &str,
        palette: Option<&[[u8; 4]]>,
    ) -> Option<TiledTexture> {
        if let Some((p, k, tt)) = &self.full_reduced {
            if p == path && k == key {
                return Some(tt.clone());
            }
        }
        let (size, mut rgba) = match &self.full_src {
            Some((p, sz, px)) if p == path => (*sz, px.clone()),
            _ => {
                let img = self.registry.decode_path(path).ok()?;
                ([img.width as usize, img.height as usize], img.rgba_bytes())
            }
        };
        let (w, h) = (size[0], size[1]);
        let aux = self.pipe_aux(palette);
        apply_pipeline(&mut rgba, w, h, &self.adjust, &aux);
        let tt = TiledTexture::from_rgba(
            ctx,
            "pv_full_reduced",
            size,
            &rgba,
            egui::TextureOptions::NEAREST_REPEAT,
        );
        self.full_reduced = Some((path.to_path_buf(), key.to_string(), tt.clone()));
        Some(tt)
    }

    /// The image the Details / Recolor panes act on: in single view the open
    /// image; in the grid the hovered tile, falling back to the last-hovered one
    /// (so the panes stay usable after the pointer moves onto them).
    fn inspected_entry(&self) -> Option<Entry> {
        let path = match self.mode {
            Mode::Single => self
                .full_tex
                .as_ref()
                .map(|(p, _)| p.clone())
                .or_else(|| self.entries.get(self.selected).map(|e| e.path.clone())),
            Mode::Grid => self
                .hovered
                .and_then(|i| self.entries.get(i))
                .map(|e| e.path.clone())
                .or_else(|| self.last_inspected.clone()),
        };
        path.and_then(|p| self.entries.iter().find(|e| e.path == p).cloned())
    }

    /// The file's SAUCE record (title/author/group/…), parsed from its last 128
    /// bytes and cached per path. None when the file has no SAUCE.
    fn cached_sauce(&mut self, path: &Path) -> Option<crate::sauce::Sauce> {
        if let Some(c) = self.sauce_cache.get(path) {
            return c.clone();
        }
        // 16colo.rs pieces show under a *virtual* display path; the downloaded bytes
        // live elsewhere (`colo_files` maps virtual → real). Read SAUCE from the real
        // file so the Details panel works for an opened piece. (Un-opened pieces get
        // their SAUCE pre-seeded into this same cache from the 16colo API.)
        let real = self.colo_files.get(path).map_or(path, |p| p.as_path());
        // Read enough tail to include the COMNT block, which sits *before* the 128-byte
        // record (up to 255 lines × 64 + "COMNT"); else `sauce::parse` can't recover the
        // comment/description. ~16 KB covers any valid block + the record + stray EOL.
        const SAUCE_TAIL: u64 = 5 + 255 * 64 + 128 + 16;
        let parsed = read_file_tail(real, SAUCE_TAIL).and_then(|t| crate::sauce::parse(&t));
        self.sauce_cache.insert(path.to_path_buf(), parsed.clone());
        parsed
    }

    /// The `(w, h)` aspect to render `path`'s Details / Recolor preview at, so it agrees
    /// with the main view + navigator minimap. Those use the **full** texture; if `path`'s
    /// cached thumbnail was decoded at a different width (e.g. before a 9px-cell toggle,
    /// which re-decodes the full view but not thumbnails), the thumbnail's own dims would
    /// render the preview squished. So prefer the open image's full texture; else fall back
    /// to the thumbnail's own size (`tsz`) — correct for a hovered, not-open entry.
    fn preview_aspect(&self, path: &Path, tsz: egui::Vec2) -> (f32, f32) {
        self.full_tex
            .as_ref()
            .filter(|(p, _)| p == path)
            .map(|(_, tt)| (tt.size[0] as f32, tt.size[1] as f32))
            .unwrap_or((tsz.x, tsz.y))
    }

    fn ui_details(&mut self, ui: &mut egui::Ui) {
        ui.strong("Details");
        ui.separator();
        let Some(entry) = self.inspected_entry() else {
            ui.weak("Hover an item (or open one) to inspect it.");
            return;
        };
        // Deferred: the Save dialogs run after the closure (needs `&mut self`).
        let mut want_download = false;
        let mut want_pack: Option<PathBuf> = None;
        let disp = self.to_display(&entry.path);
        // Inspecting a downloadable pack folder (it has a fetched zip URL). Keyed off
        // `remote_urls`, not path depth, so `groups/<g>` / `artists/<a>` listing folders
        // (also depth 2) aren't mistaken for packs.
        let inspected_pack = entry.is_dir && self.remote_urls.contains_key(&disp);
        // Browsing *inside* a 16colo.rs pack: the mount's display path is the pack.
        let mount_pack = self
            .archive_mount
            .as_ref()
            .map(|m| m.archive.clone())
            .filter(|a| crate::sixteen::is_remote(a));
        egui::ScrollArea::vertical()
            .id_salt("details")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.label(egui::RichText::new(short_name(&entry.path)).strong());
                ui.add_space(6.0);
                if entry.is_dir {
                    if inspected_pack {
                        // A 16colo.rs pack folder isn't on disk until opened, so the
                        // usual image/subfolder scan is meaningless here — offer the
                        // whole-pack download instead.
                        ui.weak("16colo.rs pack");
                        ui.add_space(8.0);
                        if ui
                            .button("⬇ Download pack .zip")
                            .on_hover_text("Download the entire pack archive to your disk")
                            .clicked()
                        {
                            want_pack = Some(disp.clone());
                        }
                    } else {
                        let info = self
                            .folder_info
                            .entry(entry.path.clone())
                            .or_insert_with(|| scan_folder_info(&entry.path))
                            .clone();
                        egui::Grid::new("details_dir")
                            .num_columns(2)
                            .spacing([12.0, 5.0])
                            .show(ui, |ui| {
                                ui.weak("Type");
                                ui.label("Folder");
                                ui.end_row();
                                ui.weak("Images");
                                ui.label(format!("{}", info.images));
                                ui.end_row();
                                ui.weak("Subfolders");
                                ui.label(format!("{}", info.subdirs));
                                ui.end_row();
                            });
                    }
                } else {
                    let meta = self.img_meta.get(&entry.path).copied();
                    // View history (count + last-viewed) for the stats rows below.
                    let views = self.view_record(&entry.path);
                    // Plain thumbnail (the recolored preview lives in the Recolor pane).
                    if let Some(tex) = self.thumb_tex.get(&entry.path) {
                        let tsz = tex.size_vec2();
                        // Fill the pane width so the thumbnail grows as the dock is
                        // widened; cap height so a very tall image stays usable. Match the
                        // full viewer's CRT stretch (≈1.2× taller) for text-mode art, so
                        // the preview isn't squished next to the stretched full view. Take
                        // the aspect from the open image's full texture (not the thumbnail's
                        // own dims) so it agrees with the main view + minimap.
                        let ar_y = if self.crt_aspect && is_textmode_ext(&entry.path) {
                            1.2
                        } else {
                            1.0
                        };
                        let (bw, bh) = self.preview_aspect(&entry.path, tsz);
                        let mut w = ui.available_width();
                        let mut h = w * (bh * ar_y) / bw;
                        let max_h = 600.0;
                        if h > max_h {
                            h = max_h;
                            w = h * bw / (bh * ar_y);
                        }
                        ui.vertical_centered(|ui| {
                            ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(w, h)));
                        });
                        ui.add_space(6.0);
                    } else {
                        self.thumbs.request(&entry.path, THUMB_PX);
                        self.want_repaint = true;
                    }
                    let fmt = entry
                        .path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_uppercase();
                    egui::Grid::new("details_img")
                        .num_columns(2)
                        .spacing([12.0, 5.0])
                        .show(ui, |ui| {
                            ui.weak("Format");
                            ui.label(if fmt.is_empty() {
                                "image".to_string()
                            } else {
                                fmt
                            });
                            ui.end_row();
                            if let Some(m) = meta {
                                ui.weak("Dimensions");
                                ui.label(format!("{} × {}", m.w, m.h));
                                ui.end_row();
                                ui.weak("Colors");
                                ui.label(match m.colors {
                                    Some(c) => c.to_string(),
                                    None => "many".to_string(),
                                });
                                ui.end_row();
                            }
                            ui.weak("Size");
                            ui.label(human_size(entry.size));
                            ui.end_row();
                            ui.weak("Rating");
                            ui.label(stars_label(entry.rating));
                            ui.end_row();
                            // View history: count + last-viewed date (— when never viewed).
                            ui.weak("Views");
                            ui.label(match views {
                                Some(v) if v.count > 0 => v.count.to_string(),
                                _ => "—".to_string(),
                            });
                            ui.end_row();
                            ui.weak("Last viewed");
                            ui.label(match views {
                                Some(v) if v.last > 0 => date_ymd_unix(v.last),
                                _ => "—".to_string(),
                            });
                            ui.end_row();
                            if let Some(t) = entry.mtime {
                                ui.weak("Modified");
                                ui.label(fmt_time(t));
                                ui.end_row();
                            }
                        });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        want_download = ui
                            .button("⬇ Download…")
                            .on_hover_text(if mount_pack.is_some() {
                                "Save a copy of this art file from 16colo.rs to your disk"
                            } else {
                                "Save a copy of this file to another location"
                            })
                            .clicked();
                        // Inside a 16colo.rs pack: also offer the whole pack .zip.
                        if mount_pack.is_some()
                            && ui
                                .button("⬇ Pack .zip")
                                .on_hover_text("Download the entire pack archive")
                                .clicked()
                        {
                            want_pack = mount_pack.clone();
                        }
                    });
                    // SAUCE metadata — always shown for scene art / 16colo pieces so the
                    // panel doesn't vanish when a file has no record (it reads "no record"
                    // and the fields show "—"). Hidden for ordinary images (never SAUCE).
                    if is_textmode_ext(&entry.path) || self.colo_pieces.contains_key(&entry.path) {
                        // For a 16colo piece whose listing omitted SAUCE, kick off a
                        // background pack fetch so hovering it (not just opening) fills
                        // the record — it lands in `sauce_cache` a few frames later.
                        self.ensure_colo_sauce(&entry.path);
                        let sauce = self.cached_sauce(&entry.path);
                        let none = sauce.is_none();
                        // A 16colo piece with no SAUCE yet + a fetch in flight → show a
                        // spinner ("fetching…") rather than the misleading "no record".
                        let fetching = none
                            && self.colo_sauce_pending > 0
                            && self.colo_pieces.contains_key(&entry.path);
                        let sc = sauce.unwrap_or_default();
                        ui.add_space(8.0);
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.strong("SAUCE");
                            if fetching {
                                ui.add(egui::Spinner::new().size(13.0));
                                ui.weak("fetching…");
                            } else if none {
                                ui.weak("· no record");
                            }
                        });
                        ui.add_space(2.0);
                        egui::Grid::new("details_sauce")
                            .num_columns(2)
                            .spacing([12.0, 5.0])
                            .show(ui, |ui| {
                                // Absent field (or no record at all) → an em-dash, never blank.
                                let mut field = |k: &str, v: &str| {
                                    ui.weak(k);
                                    ui.label(if none || v.is_empty() { "—" } else { v });
                                    ui.end_row();
                                };
                                field("Title", &sc.title);
                                field("Author", &sc.author);
                                field("Group", &sc.group);
                                field("Date", &sc.date_pretty());
                                field("Kind", sc.kind_label());
                                field("Font", &sc.font);
                                // Character/binary art (ANSI/BIN/XBin/…) stores its size
                                // in *cells*: TInfo1 = columns, TInfo2 = lines.
                                if !none && matches!(sc.data_type, 1 | 5 | 6) {
                                    let cols = (sc.tinfo1 > 0).then(|| sc.tinfo1.to_string());
                                    let lines = (sc.tinfo2 > 0).then(|| sc.tinfo2.to_string());
                                    field("Columns", cols.as_deref().unwrap_or("—"));
                                    field("Lines", lines.as_deref().unwrap_or("—"));
                                }
                            });
                    }
                }
            });
        if want_download {
            // A 16colo.rs piece's bytes aren't a plain local file (they live in the
            // download cache, or aren't fetched yet) — grab the `raw` file instead of
            // trying to copy a virtual path.
            if self.colo_pieces.contains_key(&entry.path) {
                self.download_piece(&entry.path, false);
            } else {
                self.download_file(&entry.path);
            }
        }
        if let Some(vpath) = want_pack {
            self.download_pack(&vpath);
        }
    }

    /// Save a copy of `path` to a user-chosen location via a Save dialog. Works for any
    /// inspected file: a plain local file, an extracted archive member, or a 16colo.rs
    /// piece — all are real files on disk by the time they're shown (packs/archives are
    /// downloaded + unzipped to a temp dir first), so a byte copy is all it takes.
    fn download_file(&mut self, path: &Path) {
        let real = self.real_path(path);
        let name = real
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
            .to_string();
        let bytes = match std::fs::read(&real) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Couldn't read {name}: {e}");
                return;
            }
        };
        if let Some(dest) = rfd::FileDialog::new().set_file_name(&name).save_file() {
            match std::fs::write(&dest, &bytes) {
                Ok(()) => self.status = format!("Saved {name} → {}", dest.display()),
                Err(e) => self.status = format!("Couldn't save {name}: {e}"),
            }
        }
    }

    /// Download a whole 16colo.rs pack `.zip` to a user-chosen location. `vpath` is the
    /// virtual pack path (`<16colo.rs>/<year>/<pack>`). The zip is usually already in the
    /// temp cache (browsing the pack downloaded it), so [`sixteen::download`] returns
    /// instantly; otherwise it fetches it. Saved by copy via a Save dialog.
    fn download_pack(&mut self, vpath: &Path) {
        let parts = crate::sixteen::rel_parts(vpath);
        let (Some(year), Some(pack)) = (
            parts.first().and_then(|y| y.parse::<u32>().ok()),
            parts.get(1).cloned(),
        ) else {
            self.status = "Not a 16colo.rs pack".into();
            return;
        };
        let url = self
            .remote_urls
            .get(vpath)
            .cloned()
            .unwrap_or_else(|| crate::sixteen::pack_url(year, &pack));
        self.status = format!("Fetching {pack}.zip…");
        let zip = match crate::sixteen::download(&url) {
            Ok(z) => z,
            Err(e) => {
                self.status = format!("Couldn't download {pack}: {e}");
                return;
            }
        };
        if let Some(dest) = rfd::FileDialog::new()
            .set_file_name(format!("{pack}.zip"))
            .save_file()
        {
            match std::fs::copy(&zip, &dest) {
                Ok(_) => self.status = format!("Saved {pack}.zip → {}", dest.display()),
                Err(e) => self.status = format!("Couldn't save {pack}.zip: {e}"),
            }
        }
    }

    /// The Recolor dock: live recolored preview + palette-swap / reduce / dither /
    /// random controls + swatches + export/save, all acting on `inspected_entry`.
    fn ui_recolor(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.strong("Recolor");
            ui.checkbox(&mut self.recolor_grid, "Apply to grid")
                .on_hover_text("Recolor every grid thumbnail with the active palette");
            if ui
                .button("⟲ Reset all")
                .on_hover_text("Clear adjustments + balance + palette / reduce / dither / edits")
                .clicked()
            {
                self.adjust = Adjust::default();
                self.selected_palette = None;
                self.custom_palette = None;
                self.quantize_on = false;
                self.quantize_n = 16;
                self.dither_method = 0;
                self.dither_amount = 1.0;
                self.dither_custom_n = 4;
                self.dither_custom = crate::thumb::bayer_values(4);
                self.balance_color = [128, 128, 128];
                self.balance_strength = 0.0;
                self.balance_hex = "808080".into();
                self.flash = None;
                self.editing_color = None;
            }
        });
        ui.separator();
        let Some(entry) = self.inspected_entry() else {
            ui.weak("Hover an image to recolor it.");
            return;
        };
        if entry.is_dir {
            ui.weak("Select an image to recolor.");
            return;
        }
        let mut do_export: Option<(String, Vec<[u8; 4]>)> = None;
        let mut save_request: Option<bool> = None;
        egui::ScrollArea::vertical()
            .id_salt("recolor")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                let pal_state = self.palettes.get(&entry.path).cloned();
                let recolor = self.active_recolor(&entry.path);
                // The palette shown as swatches: the active recolor palette, else the
                // image's own extracted palette. Hoisted so the swatches can sit right
                // under the preview while Export/Save reuse it lower down.
                let display: Option<Vec<[u8; 4]>> = match &recolor {
                    Some((_, pal)) => Some(pal.clone()),
                    None => match &pal_state {
                        Some(Some(full)) => Some(full.clone()),
                        _ => None,
                    },
                };
                // Recolored preview thumbnail — or, while a swatch is held down, a
                // highlight of where that color is used (set by the swatch loop below).
                let mut tex = None;
                if let Some(fc) = self.flash {
                    let pal = recolor.as_ref().map(|(_, p)| p.clone());
                    tex = self.make_flash_tex(&ctx, &entry.path, pal, fc);
                    self.want_repaint = true;
                }
                if tex.is_none() && (self.pipeline_active() || recolor.is_some()) {
                    // Adjustments + (optional) palette map.
                    let rkey = recolor.as_ref().map(|(k, _)| k.as_str()).unwrap_or("orig");
                    let key = format!("{}|{rkey}", self.pipeline_key());
                    let pal = recolor.as_ref().map(|(_, p)| p.as_slice());
                    tex = self.make_preview(&ctx, &entry.path, &key, pal);
                }
                if tex.is_none() {
                    tex = self.thumb_tex.get(&entry.path).cloned();
                }
                if let Some(tex) = tex {
                    let tsz = tex.size_vec2();
                    // Fill the pane width so the preview grows as the dock is widened;
                    // only a very tall image is capped (by height) to keep it usable. Match
                    // the full viewer's CRT stretch (≈1.2× taller) for text-mode art, and
                    // take the aspect from the open image's full texture (not the preview
                    // texture's own dims) so it agrees with the main view + minimap.
                    let ar_y = if self.crt_aspect && is_textmode_ext(&entry.path) {
                        1.2
                    } else {
                        1.0
                    };
                    let (bw, bh) = self.preview_aspect(&entry.path, tsz);
                    let mut w = ui.available_width();
                    let mut h = w * (bh * ar_y) / bw;
                    let max_h = 600.0;
                    if h > max_h {
                        h = max_h;
                        w = h * bw / (bh * ar_y);
                    }
                    ui.vertical_centered(|ui| {
                        ui.image(egui::load::SizedTexture::new(tex.id(), egui::vec2(w, h)));
                    });
                    ui.add_space(6.0);
                } else {
                    self.thumbs.request(&entry.path, THUMB_PX);
                    self.want_repaint = true;
                }

                // ----- extracted palette swatches, kept directly under the preview so
                //       they're always visible (Export/Save stay further down) -----
                if let Some(d) = &display {
                    ui.horizontal(|ui| {
                        ui.strong("Palette");
                        ui.weak(format!("· {} colors", d.len()));
                    });
                    ui.add_space(4.0);
                    let prev = ui.spacing().item_spacing;
                    ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
                    let mut swatch_flash: Option<[u8; 4]> = None;
                    let mut swatch_delete: Option<usize> = None;
                    let mut swatch_edit: Option<usize> = None;
                    ui.horizontal_wrapped(|ui| {
                        for (i, &c) in d.iter().enumerate() {
                            let (r, resp) = ui
                                .allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
                            let col =
                                egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
                            ui.painter().rect_filled(r, 2.0, col);
                            ui.painter().rect_stroke(
                                r,
                                2.0,
                                egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                                egui::StrokeKind::Inside,
                            );
                            let resp = resp.on_hover_text(format!(
                                "#{:02X}{:02X}{:02X}\nhold: flash where it's used · right-click: edit / delete",
                                c[0], c[1], c[2]
                            ));
                            if resp.is_pointer_button_down_on() {
                                swatch_flash = Some(c);
                            }
                            resp.context_menu(|ui| {
                                if ui.button("✎ Edit color…").clicked() {
                                    swatch_edit = Some(i);
                                    ui.close();
                                }
                                if ui.button("🗑 Delete color").clicked() {
                                    swatch_delete = Some(i);
                                    ui.close();
                                }
                            });
                        }
                    });
                    // Apply swatch actions. Edits/deletes materialize the active palette
                    // into a live `custom_palette` — the .gpl files on disk are never
                    // touched. Flash is set only while a swatch is held.
                    self.flash = swatch_flash;
                    if swatch_flash.is_some() {
                        self.want_repaint = true;
                    }
                    if let Some(i) = swatch_delete {
                        if d.len() > 1 && i < d.len() {
                            let mut pal = d.clone();
                            pal.remove(i);
                            self.custom_palette = Some(pal);
                            self.selected_palette = None;
                            self.quantize_on = false;
                        }
                    }
                    if let Some(i) = swatch_edit {
                        if let Some(&c) = d.get(i) {
                            self.editing_color = Some((i, c));
                            self.custom_palette = Some(d.clone());
                            self.selected_palette = None;
                            self.quantize_on = false;
                        }
                    }
                    ui.spacing_mut().item_spacing = prev;
                } else if matches!(pal_state, Some(None)) {
                    ui.add_space(4.0);
                    ui.weak(format!(
                        "(image itself has > {} colors — pick a palette below)",
                        crate::thumb::SWATCH_CAP
                    ));
                }
                // ----- export / save, kept directly under the swatches so they stay
                //       reachable without scrolling on small screens -----
                if let Some(d) = &display {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Export .GPL…").clicked() {
                            do_export = Some((short_name(&entry.path), d.clone()));
                        }
                        // Save the processed image (recolor and/or adjustments).
                        if recolor.is_some() || self.pipeline_active() {
                            if ui
                                .button("💾 Save recolored")
                                .on_hover_text("Write to a 'recolored' subfolder next to the image")
                                .clicked()
                            {
                                save_request = Some(false);
                            }
                            if ui.button("Save As…").clicked() {
                                save_request = Some(true);
                            }
                        }
                    });
                }
                ui.add_space(6.0);
                ui.separator();

                // ----- Adjustments (applied before the palette map) -----
                {
                    let header = if self.adjust.is_identity() {
                        "Adjustments".to_string()
                    } else {
                        "Adjustments *".to_string() // * = active (font lacks ● U+25CF)
                    };
                    egui::CollapsingHeader::new(header)
                        .id_salt("adjustments")
                        .default_open(true)
                        .show(ui, |ui| {
                            let mut a = self.adjust;
                            let n = a.order.len();
                            // ⬆/⬇ rearrange the apply order; applied after the loop so
                            // we don't mutate `order` mid-iteration.
                            let mut mv: Option<(usize, usize)> = None;
                            // Layout: [label column][pad][slider+value][pad][⟲ ⬆ ⬇].
                            // The label column is sized to the widest label and the
                            // right cluster is a fixed reserve, so every slider ends up
                            // the *same* width and the columns line up.
                            let font = egui::TextStyle::Body.resolve(ui.style());
                            let label_col = a
                                .order
                                .iter()
                                .map(|op| {
                                    ui.painter()
                                        .layout_no_wrap(
                                            op.spec().0.to_string(),
                                            font.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                })
                                .fold(0.0_f32, f32::max);
                            const PAD: f32 = 10.0; // gap between every item (the padding)
                            const VALUE_W: f32 = 64.0; // slider's value box + its inner gap
                            const BTN_W: f32 = 24.0; // each fixed-width button (⟲ ⬆ ⬇)
                            const HANDLE_W: f32 = 16.0; // drag-reorder grip
                            // Reserve handle + label + value + 3 buttons + the inter-item
                            // gaps, so the leftover (the slider) is identical on every row.
                            let avail = ui.available_width();
                            let slider_w = (avail
                                - HANDLE_W
                                - label_col
                                - VALUE_W
                                - BTN_W * 3.0
                                - PAD * 6.0)
                                .max(48.0);
                            // Marker rows have no slider — let their label span the whole
                            // [label+slider+value] span so the ⬆/⬇ cluster still aligns.
                            let marker_w = label_col + slider_w + VALUE_W + PAD * 2.0;
                            let btn = |glyph: &str| {
                                egui::Button::new(glyph).small().min_size(egui::vec2(BTN_W, 0.0))
                            };
                            let mut row_rects: Vec<egui::Rect> = Vec::with_capacity(n);
                            for i in 0..n {
                                let op = a.order[i];
                                let (label, lo, hi, def, step) = op.spec();
                                let row_h = ui.spacing().interact_size.y;
                                // Reserve a background shape *now* so it paints behind the
                                // row content; filled in (zebra / hover) once we know the
                                // row's rect below.
                                let stripe = ui.painter().add(egui::Shape::Noop);
                                let row = ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = PAD; // even padding
                                    // Grip: drag to reorder (the ⬆/⬇ buttons still work too).
                                    if drag_handle(ui, HANDLE_W, row_h).drag_started() {
                                        self.adjust_drag = Some(i);
                                    }
                                    // Left: label (+ slider for value ops). The control
                                    // cluster is added afterwards, right-anchored.
                                    if op.is_marker() {
                                        // Marker ops (Palette / Color balance / Dither) have
                                        // no slider — the label shows active state; the real
                                        // controls live in a section lower in the pane.
                                        let active = ui.visuals().selection.bg_fill;
                                        let (txt, hover) = match op {
                                            OpKind::Palette => (
                                                if recolor.is_some() {
                                                    egui::RichText::new("Palette rematch").strong().color(active)
                                                } else {
                                                    egui::RichText::new("Palette rematch (none active)").weak()
                                                },
                                                "Where the selected palette / Reduce is applied — drag to reorder",
                                            ),
                                            OpKind::Dither => (
                                                if self.dither_method != 0 && self.dither_amount > 0.0 {
                                                    egui::RichText::new(format!(
                                                        "Dither · {}",
                                                        crate::thumb::DITHER_NAMES
                                                            .get(self.dither_method as usize)
                                                            .copied()
                                                            .unwrap_or("?")
                                                    ))
                                                    .strong()
                                                    .color(active)
                                                } else {
                                                    egui::RichText::new("Dither (off)").weak()
                                                },
                                                "Ordered/custom dither pattern — set method & amount in the Dither section; drag to reorder",
                                            ),
                                            OpKind::ColorBalance => (
                                                if self.balance_offset() != [0, 0, 0] {
                                                    egui::RichText::new("Color balance").strong().color(active)
                                                } else {
                                                    egui::RichText::new("Color balance (neutral)").weak()
                                                },
                                                "Per-channel R/G/B offset — set color & strength in the Color balance section; drag to reorder",
                                            ),
                                            _ => unreachable!("non-marker in marker branch"),
                                        };
                                        ui.add_sized(
                                            egui::vec2(marker_w, row_h),
                                            egui::Label::new(txt).truncate(),
                                        )
                                        .on_hover_text(hover);
                                    } else {
                                        let dec = if step > 0.0 { 0 } else { 2 };
                                        value_slider(
                                            ui, label, label_col, slider_w, VALUE_W,
                                            a.field_mut(op), lo, hi, def, step, dec,
                                        );
                                    }
                                    // Right-anchored cluster: ⟲ ⬆ ⬇. Right-to-left so the
                                    // arrows sit at the row's right edge on *every* row
                                    // (a Palette row keeps the ⟲ slot empty for alignment).
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.spacing_mut().item_spacing.x = PAD;
                                            if ui
                                                .add_enabled(i + 1 < n, btn("⬇"))
                                                .on_hover_text("move later in the pipeline")
                                                .clicked()
                                            {
                                                mv = Some((i, i + 1));
                                            }
                                            if ui
                                                .add_enabled(i > 0, btn("⬆"))
                                                .on_hover_text("move earlier in the pipeline")
                                                .clicked()
                                            {
                                                mv = Some((i, i - 1));
                                            }
                                            if op.is_marker() {
                                                ui.add_space(BTN_W); // align with the ⟲ column
                                            } else if ui
                                                .add(btn("⟲"))
                                                .on_hover_text("reset (or middle-click the slider)")
                                                .clicked()
                                            {
                                                *a.field_mut(op) = def;
                                            }
                                        },
                                    );
                                });
                                // Zebra stripe (odd rows) + hover highlight, painted behind
                                // the row so it never covers the controls. Spans the full
                                // pane width so the value→slider pairing reads clearly.
                                let mut bg = row.response.rect;
                                bg.min.x = ui.max_rect().min.x;
                                bg.max.x = ui.max_rect().max.x;
                                bg = bg.expand2(egui::vec2(0.0, ui.spacing().item_spacing.y * 0.5));
                                let hovered = ui.rect_contains_pointer(bg);
                                let fill = if hovered {
                                    ui.visuals().selection.bg_fill.gamma_multiply(0.30)
                                } else if i % 2 == 1 {
                                    ui.visuals().faint_bg_color
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                if fill != egui::Color32::TRANSPARENT {
                                    ui.painter().set(stripe, egui::Shape::rect_filled(bg, 2.0, fill));
                                }
                                if hovered {
                                    self.want_repaint = true;
                                }
                                row_rects.push(row.response.rect);
                            }
                            // Drag-reorder: while a grip is held, draw an insertion line
                            // where it'll land; drop moves the op to that slot.
                            if let Some(from) = self.adjust_drag {
                                self.want_repaint = true;
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                let ptr_y = ui.input(|i| i.pointer.interact_pos().map(|p| p.y));
                                let released = ui.input(|i| i.pointer.any_released());
                                if let (Some(py), Some(first), Some(last)) =
                                    (ptr_y, row_rects.first(), row_rects.last())
                                {
                                    // Target = first row whose center sits below the pointer.
                                    let mut to = n;
                                    let mut line_y = last.bottom();
                                    for (j, r) in row_rects.iter().enumerate() {
                                        if py < r.center().y {
                                            to = j;
                                            line_y = r.top();
                                            break;
                                        }
                                    }
                                    ui.painter().hline(
                                        first.x_range(),
                                        line_y,
                                        egui::Stroke::new(2.0, ui.visuals().selection.bg_fill),
                                    );
                                    if released {
                                        let mut v: Vec<OpKind> = a.order.to_vec();
                                        let item = v.remove(from);
                                        let at = if from < to { to - 1 } else { to };
                                        v.insert(at.min(v.len()), item);
                                        if let Ok(arr) = <[OpKind; 15]>::try_from(v) {
                                            a.order = arr;
                                        }
                                        self.adjust_drag = None;
                                    }
                                } else if released {
                                    self.adjust_drag = None;
                                }
                            }
                            if let Some((from, to)) = mv {
                                a.order.swap(from, to);
                            }
                            if ui.button("Reset all").clicked() {
                                a = Adjust::default();
                            }
                            self.adjust = a;
                        });
                }

                // ----- Color balance (per-channel R/G/B offset; positioned by the
                //       "Color balance" row in Adjustments above) -----
                {
                    let active = self.balance_offset() != [0, 0, 0];
                    let header = if active {
                        "Color balance *" // * = active (font lacks ● U+25CF)
                    } else {
                        "Color balance"
                    };
                    egui::CollapsingHeader::new(header)
                        .id_salt("color_balance")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.weak(
                                "Tints the image: 128 = neutral, >128 adds that channel, <128 removes it.",
                            );
                            // Same column geometry as Adjustments (label | wide slider |
                            // right-justified value | ⟲), minus the grip and move arrows.
                            const PAD: f32 = 10.0;
                            const VALUE_W: f32 = 64.0;
                            const BTN_W: f32 = 24.0;
                            let font = egui::TextStyle::Body.resolve(ui.style());
                            let label_col = ["Strength", "R", "G", "B"]
                                .iter()
                                .map(|s| {
                                    ui.painter()
                                        .layout_no_wrap(
                                            (*s).to_string(),
                                            font.clone(),
                                            egui::Color32::WHITE,
                                        )
                                        .size()
                                        .x
                                })
                                .fold(0.0_f32, f32::max);
                            let slider_w = (ui.available_width()
                                - label_col
                                - VALUE_W
                                - BTN_W
                                - PAD * 3.0)
                                .max(48.0);
                            let btn = |g: &str| {
                                egui::Button::new(g).small().min_size(egui::vec2(BTN_W, 0.0))
                            };
                            let reset_btn = |ui: &mut egui::Ui, tip: &str| {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| ui.add(btn("⟲")).on_hover_text(tip).clicked(),
                                )
                                .inner
                            };
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = PAD;
                                value_slider(
                                    ui, "Strength", label_col, slider_w, VALUE_W,
                                    &mut self.balance_strength, 0.0f32, 1.0, 0.0, 0.0, 2,
                                );
                                if reset_btn(ui, "reset to 0") {
                                    self.balance_strength = 0.0;
                                }
                            });
                            // R/G/B as full 0–255 sliders, each with its own reset.
                            let before = self.balance_color;
                            for (lbl, k) in [("R", 0usize), ("G", 1), ("B", 2)] {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = PAD;
                                    value_slider(
                                        ui, lbl, label_col, slider_w, VALUE_W,
                                        &mut self.balance_color[k], 0u8, 255, 128, 1.0, 0,
                                    );
                                    if reset_btn(ui, "reset to 128") {
                                        self.balance_color[k] = 128;
                                    }
                                });
                            }
                            // Keep the hex buffer in step when the sliders move the color
                            // (done *before* the hex field so typing there isn't clobbered).
                            if self.balance_color != before {
                                self.balance_hex = format!(
                                    "{:02X}{:02X}{:02X}",
                                    self.balance_color[0],
                                    self.balance_color[1],
                                    self.balance_color[2]
                                );
                            }
                            // Swatch + hex paste (applies live on a valid 3/6-digit code).
                            ui.horizontal(|ui| {
                                let c = self.balance_color;
                                let (r, _) = ui
                                    .allocate_exact_size(egui::vec2(28.0, 18.0), egui::Sense::hover());
                                ui.painter().rect_filled(
                                    r,
                                    2.0,
                                    egui::Color32::from_rgb(c[0], c[1], c[2]),
                                );
                                ui.label("#");
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut self.balance_hex)
                                        .desired_width(70.0)
                                        .hint_text("RRGGBB"),
                                );
                                if resp.changed() {
                                    if let Some(rgb) = parse_hex(&self.balance_hex) {
                                        self.balance_color = rgb;
                                    }
                                }
                            });
                            if ui.button("Reset balance").clicked() {
                                self.balance_color = [128, 128, 128];
                                self.balance_strength = 0.0;
                                self.balance_hex = "808080".into();
                            }
                        });
                }

                {
                    // ----- Recolor controls (palette-swap / reduce) -----
                    ui.add_space(8.0);
                    ui.separator();
                    let has_own_palette = matches!(pal_state, Some(Some(_)));
                    // Deferred so the favorites/list loops can borrow self.palette_*
                    // while we decide what to select. Some(None) = clear (Original).
                    let mut pick: Option<Option<PathBuf>> = None;
                    let mut toggle_fav: Option<PathBuf> = None; // star clicked in the list
                    ui.horizontal(|ui| {
                        ui.strong("Recolor");
                        let is_orig = self.selected_palette.is_none()
                            && !self.quantize_on
                            && self.custom_palette.is_none();
                        if ui.selectable_label(is_orig, "Original").clicked() {
                            pick = Some(None);
                        }
                    });

                    ui.add_space(2.0);
                    // ----- Reduce (median-cut the image's own colors to N) — above the
                    //       palette chooser so it's always visible -----
                    let active_reduce = self.quantize_on && self.selected_palette.is_none();
                    {
                        let mut on = active_reduce;
                        if ui
                            .add_enabled(
                                has_own_palette,
                                egui::Checkbox::new(
                                    &mut on,
                                    format!("Reduce to {} colors", self.quantize_n),
                                ),
                            )
                            .on_hover_text(if has_own_palette {
                                "Median-cut this image's own palette down to N colors"
                            } else {
                                "This image has too many colors to extract a palette"
                            })
                            .changed()
                        {
                            self.quantize_on = on;
                            if on {
                                self.selected_palette = None;
                                self.custom_palette = None;
                            }
                        }
                    }
                    // Full-width slider, geometry matching the other sliders.
                    {
                        const PAD: f32 = 10.0;
                        const VALUE_W: f32 = 64.0;
                        let font = egui::TextStyle::Body.resolve(ui.style());
                        let label_col = ui
                            .painter()
                            .layout_no_wrap("Colors".to_string(), font, egui::Color32::WHITE)
                            .size()
                            .x;
                        let slider_w =
                            (ui.available_width() - label_col - VALUE_W - PAD * 2.0).max(48.0);
                        ui.add_enabled_ui(active_reduce, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = PAD;
                                value_slider(
                                    ui, "Colors", label_col, slider_w, VALUE_W,
                                    &mut self.quantize_n, 2usize, 256, 16, 1.0, 0,
                                );
                            });
                        });
                    }
                    // Quick reduction presets.
                    ui.add_enabled_ui(has_own_palette, |ui| {
                        ui.horizontal(|ui| {
                            ui.weak("Quick");
                            for nq in [4usize, 8, 16, 32] {
                                if ui.button(nq.to_string()).clicked() {
                                    self.quantize_n = nq;
                                    self.quantize_on = true;
                                    self.selected_palette = None;
                                    self.custom_palette = None;
                                }
                            }
                        });
                    });
                    self.quantize_n = self.quantize_n.clamp(2, 256);

                    ui.add_space(4.0);
                    // ----- Dither (a movable pipeline op — the "Dither" row in
                    //       Adjustments picks *where* it applies). Above the palette
                    //       chooser, always visible. -----
                    ui.horizontal(|ui| {
                        ui.label("Dither");
                        let mut m = self.dither_method as usize;
                        let cr = egui::ComboBox::from_id_salt("dither_method")
                            .selected_text(
                                crate::thumb::DITHER_NAMES.get(m).copied().unwrap_or("None"),
                            )
                            .show_ui(ui, |ui| {
                                for (i, name) in crate::thumb::DITHER_NAMES.iter().enumerate() {
                                    ui.selectable_value(&mut m, i, *name);
                                }
                            });
                        wheel_cycle(ui, &cr.response, &mut m, crate::thumb::DITHER_NAMES.len());
                        self.dither_method = m as u8;
                    });
                    if self.dither_method != 0 {
                        ui.horizontal(|ui| {
                            ui.label("Amount");
                            let resp = ui.add(egui::Slider::new(&mut self.dither_amount, 0.0..=1.0));
                            middle_reset(ui, &resp, &mut self.dither_amount, 1.0f32);
                            wheel_adjust(ui, &resp, &mut self.dither_amount, 0.05, 0.0f32, 1.0f32);
                        });
                    }
                    if matches!(self.dither_method, 4 | 5) && recolor.is_none() {
                        ui.weak("(needs a palette / Reduce so it has colors to diffuse toward)");
                    }
                    if self.dither_method == crate::thumb::DITHER_CUSTOM {
                        ui.horizontal(|ui| {
                            ui.label("Matrix");
                            for sz in [2usize, 4, 8] {
                                if ui
                                    .selectable_label(self.dither_custom_n == sz, format!("{sz}×{sz}"))
                                    .clicked()
                                {
                                    self.dither_custom_n = sz;
                                    self.dither_custom = crate::thumb::bayer_values(sz);
                                }
                            }
                            if ui
                                .button("Bayer")
                                .on_hover_text("Reseed the cells with the Bayer pattern")
                                .clicked()
                            {
                                self.dither_custom = crate::thumb::bayer_values(self.dither_custom_n);
                            }
                        });
                        let n = self.dither_custom_n;
                        let hi = (n * n - 1) as u32;
                        ui.weak(format!("cell thresholds 0..={hi} — higher = brighter bias"));
                        if self.dither_custom.len() != n * n {
                            self.dither_custom = crate::thumb::bayer_values(n);
                        }
                        egui::Grid::new("dither_matrix")
                            .spacing([3.0, 3.0])
                            .show(ui, |ui| {
                                for y in 0..n {
                                    for x in 0..n {
                                        let cell = &mut self.dither_custom[y * n + x];
                                        ui.add(egui::DragValue::new(cell).range(0..=hi).speed(0.1));
                                    }
                                    ui.end_row();
                                }
                            });
                    }

                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.weak("Palettes");
                        if ui
                            .button("🎲 Random")
                            .on_hover_text("Pick a random palette from your library")
                            .clicked()
                            && !self.palette_files.is_empty()
                        {
                            let n = self.palette_files.len();
                            let mut idx = random_index(n);
                            // Avoid re-picking the one that's already selected.
                            if n > 1
                                && self.selected_palette.as_deref()
                                    == Some(self.palette_files[idx].as_path())
                            {
                                idx = (idx + 1) % n;
                            }
                            let chosen = self.palette_files[idx].clone();
                            self.status = format!("Random palette: {}", palette_label(&chosen));
                            self.selected_palette = Some(chosen);
                            self.custom_palette = None;
                            self.quantize_on = false;
                        }
                        // Show the active palette's name (incl. whatever Random rolled).
                        if let Some(pp) = &self.selected_palette {
                            ui.weak(palette_label(pp));
                        }
                    });
                    // Show the starred palettes alphabetically (by display name).
                    let mut favs = self.palette_favorites.clone();
                    favs.sort_by_key(|p| palette_label(p).to_lowercase());
                    ui.horizontal_wrapped(|ui| {
                        for fav in &favs {
                            let sel = self.selected_palette.as_deref() == Some(fav.as_path());
                            let resp = ui.selectable_label(sel, palette_label(fav));
                            if resp.clicked() {
                                pick = Some(Some(fav.clone()));
                            }
                            resp.context_menu(|ui| {
                                if ui.button("★ Unfavorite").clicked() {
                                    toggle_fav = Some(fav.clone());
                                    ui.close();
                                }
                            });
                        }
                    });

                    egui::CollapsingHeader::new(format!(
                        "All palettes ({})",
                        self.palette_files.len()
                    ))
                    .show(ui, |ui| {
                        for p in &self.palette_files {
                            ui.horizontal(|ui| {
                                // Star toggles favorite (gold = favorited, dim = not).
                                let is_fav = self.palette_favorites.contains(p);
                                let color = if is_fav {
                                    egui::Color32::from_rgb(255, 200, 60)
                                } else {
                                    ui.visuals().weak_text_color()
                                };
                                let star = egui::Button::new(egui::RichText::new("★").color(color))
                                    .frame(false);
                                if ui
                                    .add(star)
                                    .on_hover_text(if is_fav {
                                        "Unfavorite"
                                    } else {
                                        "Favorite (add a quick button)"
                                    })
                                    .clicked()
                                {
                                    toggle_fav = Some(p.clone());
                                }
                                let sel = self.selected_palette.as_deref() == Some(p.as_path());
                                if ui.selectable_label(sel, palette_label(p)).clicked() {
                                    pick = Some(Some(p.clone()));
                                }
                            });
                        }
                    });
                    if let Some(sel) = pick {
                        // Selecting a palette (or Original) clears Random + Reduce.
                        self.selected_palette = sel;
                        self.custom_palette = None;
                        self.quantize_on = false;
                    }
                    if let Some(fp) = toggle_fav {
                        if let Some(pos) = self.palette_favorites.iter().position(|x| x == &fp) {
                            self.palette_favorites.remove(pos);
                        } else {
                            self.palette_favorites.push(fp);
                        }
                    }
                }
            });
        if let Some((name, pal)) = do_export {
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(format!("{name}.gpl"))
                .add_filter("GIMP palette", &["gpl"])
                .save_file()
            {
                match std::fs::write(&path, to_gpl(&name, &pal)) {
                    Ok(()) => {
                        self.status =
                            format!("Exported {} colors to {}", pal.len(), short_name(&path))
                    }
                    Err(e) => self.status = format!("Export failed: {e}"),
                }
            }
        }
        if let Some(as_dialog) = save_request {
            self.save_recolored(&entry.path, as_dialog);
        }

        // Color-edit popup (right-click → Edit color…). Live: writes into the
        // custom palette only, never the .gpl on disk.
        if let Some((idx, mut c)) = self.editing_color {
            let mut open = true;
            let mut done = false;
            egui::Window::new("Edit color")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    let mut col = egui::Color32::from_rgb(c[0], c[1], c[2]);
                    egui::color_picker::color_picker_color32(
                        ui,
                        &mut col,
                        egui::color_picker::Alpha::Opaque,
                    );
                    c = [col.r(), col.g(), col.b(), 255];
                    ui.add_space(4.0);
                    if ui.button("Done").clicked() {
                        done = true;
                    }
                });
            // Live-apply the edit to the custom palette.
            if let Some(pal) = self.custom_palette.as_mut() {
                if let Some(slot) = pal.get_mut(idx) {
                    *slot = c;
                }
            }
            self.editing_color = if done || !open { None } else { Some((idx, c)) };
        }
    }

    /// A filename tag describing the active recolor, e.g. "EGA16", "EGA16_floyd",
    /// or "reduce16" — used when naming the saved PNG.
    fn recolor_tag(&self) -> String {
        let base = if let Some(p) = &self.selected_palette {
            palette_label(p).replace([' ', '(', ')', '='], "")
        } else {
            format!("reduce{}", self.quantize_n)
        };
        if self.dither_method != 0 {
            let dm = crate::thumb::DITHER_NAMES
                .get(self.dither_method as usize)
                .copied()
                .unwrap_or("")
                .split(['-', '–', ' '])
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            format!("{base}_{dm}")
        } else {
            base
        }
    }

    /// Save the full-resolution recolored image as a PNG: quick-save to a
    /// `recolored/` subfolder next to the source, or via a Save-As dialog.
    fn save_recolored(&mut self, path: &Path, as_dialog: bool) {
        let recolor = self.active_recolor(path);
        if recolor.is_none() && !self.pipeline_active() {
            self.status = "Pick a palette/Reduce or set an adjustment first".into();
            return;
        }
        let (size, mut rgba) = match &self.full_src {
            Some((p, sz, px)) if p == path => (*sz, px.clone()),
            _ => match self.registry.decode_path(path) {
                Ok(img) => ([img.width as usize, img.height as usize], img.rgba_bytes()),
                Err(e) => {
                    self.status = format!("decode failed: {e}");
                    return;
                }
            },
        };
        let (w, h) = (size[0], size[1]);
        let aux = self.pipe_aux(recolor.as_ref().map(|(_, p)| p.as_slice()));
        apply_pipeline(&mut rgba, w, h, &self.adjust, &aux);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
        let name = format!("{stem}_{}.png", self.recolor_tag());
        let dest = if as_dialog {
            rfd::FileDialog::new()
                .set_file_name(&name)
                .add_filter("PNG", &["png"])
                .save_file()
        } else {
            path.parent().map(|d| {
                let sub = d.join("recolored");
                let _ = std::fs::create_dir_all(&sub);
                sub.join(&name)
            })
        };
        let Some(dest) = dest else {
            return; // cancelled
        };
        match image::RgbaImage::from_raw(size[0] as u32, size[1] as u32, rgba) {
            Some(img) => match img.save(&dest) {
                Ok(()) => self.status = format!("Saved {}", short_name(&dest)),
                Err(e) => self.status = format!("Save failed: {e}"),
            },
            None => self.status = "Save failed (bad buffer)".into(),
        }
    }

    fn ui_grid(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if self.entries.is_empty() {
            // A 16colo listing / pack download in flight → spinner, else the empty hint.
            let loading = self.remote_rx.is_some() || self.colo_rx.is_some();
            ui.centered_and_justified(|ui| {
                if loading {
                    ui.add(egui::Spinner::new().size(40.0));
                } else {
                    ui.label("Nothing here. Open a folder.");
                }
            });
            return;
        }
        // Ctrl + mouse-wheel resizes the thumbnails. egui's `zoom_modifier` defaults
        // to Ctrl/Cmd, so a Ctrl+wheel is reported as a *zoom* gesture in `zoom_delta()`
        // (and never reaches `smooth_scroll_delta`) — read it there. Touchpad pinch
        // lands here too. This is independent of the whole-GUI zoom (Ctrl +/-).
        let zoom = ui.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            self.thumb_size = (self.thumb_size * zoom).clamp(MIN_TILE, MAX_TILE);
        }

        let tile = self.thumb_size;
        let gap = self.grid_gap; // horizontal gap between tiles
        let gap_y = self.grid_gap_y; // independent vertical gap between rows
                                     // Reserve a caption strip below each thumbnail: folders always get one line
                                     // (their name); images get one line per enabled field.
        let any_dir = self.entries.iter().any(|e| e.is_dir);
        // Search results show one extra caption line: the folder the match lives in.
        let in_search = self.search_results.is_some();
        let cap_lines =
            (self.caption_fields.count_ones() as usize).max(if any_dir { 1 } else { 0 })
                + usize::from(in_search);
        let caption_h = if cap_lines == 0 {
            0.0
        } else {
            cap_lines as f32 * CAP_LINE_H + 3.0
        };
        let cell_h = tile + caption_h;
        // Fit a WHOLE number of columns. Two things have to agree or the last column
        // clips off the right edge: (1) the spacing the formula assumes must be the
        // spacing the row actually uses — egui's default item_spacing.x is 8, but we
        // lay tiles out with `gap`, so set item_spacing.x = gap below; (2) leave a
        // gutter for the (floating) scrollbar so it never sits on top of a tile.
        let bar = ui.spacing().scroll.bar_width + 2.0;
        let avail = (ui.available_width() - bar).max(tile);
        let per_row = (((avail + gap) / (tile + gap)).floor() as usize).max(1);
        let rows = self.entries.len().div_ceil(per_row);
        let row_h = cell_h + gap_y;

        // Hover-to-play: decode the hovered GIF's frames (capped) and advance them.
        let hov = self
            .hovered
            .and_then(|i| self.entries.get(i))
            .filter(|e| !e.is_dir)
            .map(|e| e.path.clone())
            .filter(|p| is_gif(p));
        match hov {
            Some(p)
                if self
                    .hover_anim
                    .as_ref()
                    .map(|a| a.path != p)
                    .unwrap_or(true) =>
            {
                self.hover_anim = build_anim(ctx, &p, 8_000_000);
            }
            None => self.hover_anim = None,
            _ => {}
        }
        if let Some(a) = self.hover_anim.as_mut() {
            let n = a.frames.len();
            if n > 1 {
                let dt = ctx.input(|i| i.stable_dt) * 1000.0;
                a.acc_ms += dt;
                let mut g = 0;
                while a.acc_ms >= f32::from(a.delays_ms[a.current]) && g < n {
                    a.acc_ms -= f32::from(a.delays_ms[a.current]);
                    a.current = (a.current + 1) % n;
                    g += 1;
                }
            }
            self.want_repaint = true;
        }

        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut hovered: Option<usize> = None;
        // Right-click file ops are deferred out of the row closure (which holds
        // `&mut self`) and applied afterward. `can_paste` is snapshotted here so
        // the menu closure needn't touch `self`.
        let mut ctx_action: Option<(usize, FileAction)> = None;
        let mut pin_dir: Option<usize> = None; // "Pin to Places" on a folder tile
        let mut smart_on: Option<(usize, SmartCriterion)> = None; // "Smart filter on…"
        let mut toggle_viewed: Option<(usize, bool)> = None; // "Mark as (not) viewed"
        let mut rate_to: Option<(usize, u8)> = None; // "Rating ▸ N" context-menu choice
        let mut pin_current = false; // "Pin <artist/group/search>" in a flat listing
        let mut dl: Option<(usize, bool)> = None; // 16colo download (idx, want_pack)
        let can_paste = self.clipboard.is_some();
        // In a 16colo flat listing the rows are pieces, so offer pinning the whole
        // listing (the artist/group/search) — its virtual path re-runs the search.
        let colo_pin_label = self
            .colo_flat
            .then(|| self.folder.as_ref().map(|f| short_name(f)))
            .flatten();
        let colo_pin_pinned = self
            .folder
            .as_ref()
            .is_some_and(|f| self.favorites.contains(f));
        // When "Apply to grid" is on, tiles render with the active recolor.
        let grid_key = self.grid_recolor_key();

        // A stable, unique id so egui persists the grid's scroll offset across grid →
        // single → grid (a bare ScrollArea's auto-id can shift or collide with the
        // Details/Recolor ScrollAreas, so the position came back arbitrary).
        let mut scroll_area = egui::ScrollArea::vertical()
            .id_salt("grid")
            .auto_shrink([false; 2]);
        // We force the offset this frame either to land on a target index (Home/End/
        // select) or to restore a folder's remembered position — in both cases skip the
        // capture below, since the returned state can lag the value we just set.
        let mut forced_scroll = false;
        if let Some(idx) = self.scroll_target.take() {
            scroll_area = scroll_area.vertical_scroll_offset((idx / per_row) as f32 * row_h);
            forced_scroll = true;
        } else if let Some(off) = self.grid_scroll_pending.take() {
            scroll_area = scroll_area.vertical_scroll_offset(off);
            forced_scroll = true;
        }
        let grid_out = scroll_area.show_rows(ui, row_h, rows, |ui, row_range| {
            for row in row_range {
                ui.horizontal(|ui| {
                    // Match the spacing the per_row math assumed (else the row's real
                    // width drifts from `avail` and the last column clips).
                    ui.spacing_mut().item_spacing.x = gap;
                    for col in 0..per_row {
                        let idx = row * per_row + col;
                        if idx >= self.entries.len() {
                            break;
                        }
                        let entry = self.entries[idx].clone();
                        let path = &entry.path;
                        let is_selected = self.selection.contains(path);
                        let meta = self.img_meta.get(path).copied();
                        let viewed = self.is_viewed(path); // visited check badge
                        let (cell_rect, resp) =
                            ui.allocate_exact_size(egui::vec2(tile, cell_h), egui::Sense::click());
                        // The thumbnail occupies the top square; any caption sits
                        // in the strip below it. The rest of the tile code keeps
                        // using `rect` (the square) unchanged.
                        let rect = egui::Rect::from_min_size(cell_rect.min, egui::vec2(tile, tile));

                        let bg = if is_selected {
                            ui.visuals().selection.bg_fill
                        } else if resp.hovered() {
                            ui.visuals().widgets.hovered.bg_fill
                        } else {
                            ui.visuals().extreme_bg_color
                        };
                        ui.painter().rect_filled(rect, 4.0, bg);

                        if entry.is_dir {
                            let info = self
                                .folder_info
                                .entry(path.clone())
                                .or_insert_with(|| scan_folder_info(path))
                                .clone();
                            if info.previews.is_empty() {
                                ui.painter_at(rect).text(
                                    rect.center() - egui::vec2(0.0, tile * 0.10),
                                    egui::Align2::CENTER_CENTER,
                                    "📁",
                                    egui::FontId::proportional(tile * 0.42),
                                    ui.visuals().text_color(),
                                );
                            } else {
                                // 2×2 montage (previews may come from subdirs — request #4).
                                let inner = rect.shrink(7.0);
                                let half = inner.size() * 0.5;
                                let origins = [
                                    inner.min,
                                    inner.min + egui::vec2(half.x, 0.0),
                                    inner.min + egui::vec2(0.0, half.y),
                                    inner.min + half,
                                ];
                                for (qi, ppath) in info.previews.iter().enumerate().take(4) {
                                    let cell =
                                        egui::Rect::from_min_size(origins[qi], half).shrink(2.0);
                                    if let Some(tex) = self.thumb_tex.get(ppath) {
                                        let fit = fit_centered(cell, tex.size_vec2());
                                        ui.painter().image(
                                            tex.id(),
                                            fit,
                                            egui::Rect::from_min_max(
                                                egui::pos2(0.0, 0.0),
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            egui::Color32::WHITE,
                                        );
                                    } else {
                                        self.thumbs.request(ppath, THUMB_PX);
                                        self.want_repaint = true;
                                    }
                                }
                            }
                            // Count badge (top-left): direct images and subfolders, so
                            // even an image-less folder shows there's content (request #4).
                            let mut parts: Vec<String> = Vec::new();
                            if info.images > 0 {
                                parts.push(format!("{}", info.images));
                            }
                            if info.subdirs > 0 {
                                parts.push(format!("📁{}", info.subdirs));
                            }
                            if !parts.is_empty() {
                                let p = ui.painter_at(rect);
                                let pos = rect.left_top() + egui::vec2(5.0, 4.0);
                                let galley = p.layout_no_wrap(
                                    parts.join("  "),
                                    egui::FontId::proportional(11.0),
                                    egui::Color32::WHITE,
                                );
                                p.rect_filled(
                                    egui::Rect::from_min_size(pos, galley.size()).expand(2.0),
                                    3.0,
                                    egui::Color32::from_black_alpha(150),
                                );
                                p.galley(pos, galley, egui::Color32::WHITE);
                            }
                            // (folder name is rendered in the caption strip below)
                        } else if entry.is_archive {
                            // Archive = a virtual folder: folder glyph + a format badge
                            // so it reads as enterable but distinct from a real folder.
                            ui.painter_at(rect).text(
                                rect.center() - egui::vec2(0.0, tile * 0.10),
                                egui::Align2::CENTER_CENTER,
                                "📁",
                                egui::FontId::proportional(tile * 0.42),
                                ui.visuals().text_color(),
                            );
                            let fmt = path
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("")
                                .to_ascii_uppercase();
                            if !fmt.is_empty() {
                                let p = ui.painter_at(rect);
                                let pos = rect.left_top() + egui::vec2(5.0, 4.0);
                                let galley = p.layout_no_wrap(
                                    fmt,
                                    egui::FontId::proportional(11.0),
                                    egui::Color32::WHITE,
                                );
                                p.rect_filled(
                                    egui::Rect::from_min_size(pos, galley.size()).expand(2.0),
                                    3.0,
                                    egui::Color32::from_rgba_unmultiplied(90, 78, 30, 210),
                                );
                                p.galley(pos, galley, egui::Color32::WHITE);
                            }
                        } else if self.hover_anim.as_ref().is_some_and(|a| &a.path == path) {
                            // Play the hovered GIF in its own tile.
                            let a = self.hover_anim.as_ref().unwrap();
                            let fit = fit_centered(
                                rect.shrink(8.0),
                                egui::vec2(a.size[0] as f32, a.size[1] as f32),
                            );
                            ui.painter().image(
                                a.frames[a.current].id(),
                                fit,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                        } else {
                            // Recolored thumbnail when "Apply to grid" is on, else plain.
                            let mut tex = None;
                            if let Some(k) = grid_key.as_deref() {
                                tex = self.grid_recolored_tex(ctx, path, k);
                            }
                            if tex.is_none() {
                                tex = self.thumb_tex.get(path).cloned();
                            }
                            if let Some(tex) = tex {
                                let fit = fit_centered(rect.shrink(8.0), tex.size_vec2());
                                ui.painter().image(
                                    tex.id(),
                                    fit,
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    egui::Color32::WHITE,
                                );
                            } else {
                                // 16colo piece → fetch its pre-rendered PNG over HTTP;
                                // any other file → decode locally.
                                if let Some(p) = self.colo_pieces.get(path) {
                                    self.colo_thumbs.request(path, &p.tn_url, THUMB_PX);
                                } else {
                                    self.thumbs.request(path, THUMB_PX);
                                }
                                // Spinner while the thumbnail decodes / downloads.
                                let t = ui.input(|i| i.time);
                                paint_spinner(
                                    &ui.painter_at(rect),
                                    rect.center(),
                                    (tile * 0.11).clamp(8.0, 18.0),
                                    t,
                                    egui::Color32::from_gray(130),
                                );
                                self.want_repaint = true;
                            }
                        }

                        // Star-rating overlay (bottom-left of the tile).
                        if !entry.is_dir && entry.rating > 0 {
                            ui.painter_at(rect).text(
                                rect.left_bottom() + egui::vec2(4.0, -3.0),
                                egui::Align2::LEFT_BOTTOM,
                                "★".repeat(entry.rating as usize),
                                egui::FontId::proportional(13.0),
                                egui::Color32::from_rgb(255, 200, 60),
                            );
                        }

                        // Visited check badge (top-right) for files + folders/packs.
                        if viewed {
                            let r = (tile * 0.085).clamp(7.0, 12.0);
                            let c = rect.right_top() + egui::vec2(-(r + 4.0), r + 4.0);
                            paint_check_badge(&ui.painter_at(rect), c, r);
                        }

                        // Configurable caption strip below the thumbnail.
                        if caption_h > 0.0 {
                            let folder = (in_search && !entry.is_dir)
                                .then(|| self.result_folder_label(&entry.path));
                            let lines =
                                caption_lines(&entry, meta, self.caption_fields, folder.as_deref());
                            let color = if is_selected {
                                ui.visuals().strong_text_color()
                            } else if ui.visuals().dark_mode {
                                // weak_text_color() is dim against the dark grid; lift it
                                // to a clearly readable grey (light mode is already fine).
                                egui::Color32::from_gray(190)
                            } else {
                                ui.visuals().weak_text_color()
                            };
                            let max_chars = ((tile - 6.0) / 6.5).max(3.0) as usize;
                            let p = ui.painter_at(cell_rect);
                            for (li, line) in lines.iter().take(cap_lines).enumerate() {
                                p.text(
                                    egui::pos2(
                                        cell_rect.center().x,
                                        rect.bottom() + 2.0 + li as f32 * CAP_LINE_H,
                                    ),
                                    egui::Align2::CENTER_TOP,
                                    elide(line, max_chars),
                                    egui::FontId::proportional(11.0),
                                    color,
                                );
                            }
                        }

                        if resp.hovered() {
                            hovered = Some(idx);
                        }
                        let resp = resp.on_hover_ui(|ui| hover_details(ui, &entry, meta));
                        if resp.clicked() {
                            clicked = Some((idx, ui.input(|i| i.modifiers)));
                        }
                        resp.context_menu(|ui| {
                            let pinned = self.favorites.iter().any(|f| f == &entry.path);
                            let colo_pin =
                                colo_pin_label.as_deref().map(|l| (l, colo_pin_pinned));
                            let colo_piece = self.colo_pieces.contains_key(&entry.path);
                            if let Some(pick) = entry_context_menu(
                                ui, &entry, can_paste, pinned, viewed, colo_pin, colo_piece,
                            ) {
                                match pick {
                                    TilePick::Pin => pin_dir = Some(idx),
                                    TilePick::PinFolder => pin_current = true,
                                    TilePick::Smart(c) => smart_on = Some((idx, c)),
                                    TilePick::File(a) => ctx_action = Some((idx, a)),
                                    TilePick::ToggleViewed(b) => toggle_viewed = Some((idx, b)),
                                    TilePick::Download(pack) => dl = Some((idx, pack)),
                                    TilePick::Rate(stars) => rate_to = Some((idx, stars)),
                                }
                            }
                        });
                        ui.add_space(gap);
                    }
                });
            }
        });
        // Remember this folder's scroll position so navigating back restores it (skip the
        // frame we forced an offset — the returned state lags the value we just applied).
        if !forced_scroll {
            if let Some(f) = self.folder.clone() {
                self.grid_scroll.insert(f, grid_out.state.offset.y);
            }
        }

        self.hovered = hovered;
        if let Some(i) = hovered {
            // Remember the last-hovered image so the Details/Recolor panes keep
            // showing it once the pointer moves off the grid onto a pane.
            self.last_inspected = self.entries.get(i).map(|e| e.path.clone());
        }
        if let Some((idx, mods)) = clicked {
            self.handle_click(ctx, idx, mods);
        }
        if let Some((idx, a)) = ctx_action {
            // Right-clicking a tile outside the current selection retargets the op
            // to just that tile (Copy/Cut/Rename/Delete). Paste/New folder act on
            // the folder regardless.
            if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
                if !self.selection.contains(&p)
                    && !matches!(a, FileAction::Paste | FileAction::NewFolder)
                {
                    self.selection.clear();
                    self.selection.insert(p);
                    self.hovered = Some(idx);
                }
            }
            self.do_file_action(a);
        }
        if let Some(idx) = pin_dir {
            if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
                if !self.favorites.contains(&p) {
                    self.favorites.push(p);
                    self.status = "Pinned to Places".into();
                }
            }
        }
        if let Some((idx, crit)) = smart_on {
            if let Some(e) = self.entries.get(idx).cloned() {
                self.smart_filter_from(&e, crit);
            }
        }
        if let Some((idx, b)) = toggle_viewed {
            if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
                self.set_viewed(&p, b);
            }
        }
        if let Some((idx, stars)) = rate_to {
            self.rate_entry(idx, stars);
        }
        if pin_current {
            self.pin_current_folder();
        }
        if let Some((idx, want_pack)) = dl {
            if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
                self.download_piece(&p, want_pack);
            }
        }
    }

    /// Sortable table renderer for the browse mode (toggled via `table_view`). It
    /// shares the grid's data (`entries`, `selection`, thumbnails) and click
    /// semantics (`handle_click`) — `Mode` stays `Grid` — but lays each entry out as
    /// a row with clickable column headers (click to sort, click again to reverse).
    /// In a 16colo.rs flat listing (`colo_flat`) it shows scene columns (artist /
    /// year / group / pack + a per-row download menu); elsewhere, file columns.
    fn ui_table(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if self.entries.is_empty() {
            // A 16colo listing in flight (artist/group/search streams in) → spinner.
            let loading = self.remote_rx.is_some() || self.colo_rx.is_some();
            ui.centered_and_justified(|ui| {
                if loading {
                    ui.add(egui::Spinner::new().size(40.0));
                } else {
                    ui.label("Nothing here. Open a folder.");
                }
            });
            return;
        }
        let scene = self.colo_flat;
        let row_h = 46.0_f32;
        let cell_pad = 12.0_f32; // horizontal breathing room inside every cell

        // Column set + widths. The name/filename (and scene artist) columns flex to
        // absorb leftover width so the table always spans the panel; the rest are fixed.
        struct Col {
            kind: ColKind,
            label: &'static str,
            sort: Option<SortKey>,
            w: f32,
            flex: bool,
            num: bool, // right-align (numeric / short value columns)
        }
        let col = |kind, label, sort, w, flex, num| Col {
            kind,
            label,
            sort,
            w,
            flex,
            num,
        };
        let thumb_w = row_h; // a square thumbnail cell
        let mut cols: Vec<Col> = Vec::new();
        cols.push(col(ColKind::Thumb, "", None, thumb_w, false, false));
        if scene {
            // 16colo flat-piece layout. Filename + Download always show; the rest are
            // toggled from the header right-click menu (CS_* bitmask).
            let cs = self.colo_columns;
            cols.push(col(
                ColKind::Name,
                "Filename",
                Some(SortKey::Name),
                0.0,
                true,
                false,
            ));
            if cs & CS_ARTIST != 0 {
                cols.push(col(
                    ColKind::Artist,
                    "Artist",
                    Some(SortKey::Artist),
                    0.0,
                    true,
                    false,
                ));
            }
            if cs & CS_TYPE != 0 {
                cols.push(col(ColKind::Type, "Type", Some(SortKey::Type), 56.0, false, false));
            }
            if cs & CS_YEAR != 0 {
                cols.push(col(ColKind::Year, "Year", Some(SortKey::Year), 52.0, false, true));
            }
            if cs & CS_GROUP != 0 {
                cols.push(col(ColKind::Group, "Group", Some(SortKey::Group), 130.0, false, false));
            }
            if cs & CS_PACK != 0 {
                cols.push(col(ColKind::Pack, "Pack", Some(SortKey::Pack), 130.0, false, false));
            }
            if cs & CS_RATING != 0 {
                cols.push(col(ColKind::Rating, "Rating", Some(SortKey::Rating), 72.0, false, false));
            }
            cols.push(col(ColKind::Download, "", None, 96.0, false, false));
        } else {
            // File layout — Name is always shown; the rest are user-toggled (TC_* mask).
            cols.push(col(
                ColKind::Name,
                "Name",
                Some(SortKey::Name),
                0.0,
                true,
                false,
            ));
            let tc = self.table_columns;
            if tc & TC_TYPE != 0 {
                cols.push(col(
                    ColKind::Type,
                    "Type",
                    Some(SortKey::Type),
                    64.0,
                    false,
                    false,
                ));
            }
            if tc & TC_SIZE != 0 {
                cols.push(col(
                    ColKind::Size,
                    "Size",
                    Some(SortKey::Size),
                    84.0,
                    false,
                    true,
                ));
            }
            if tc & TC_DIMENSIONS != 0 {
                cols.push(col(
                    ColKind::Dims,
                    "Dimensions",
                    Some(SortKey::Dimensions),
                    96.0,
                    false,
                    true,
                ));
            }
            if tc & TC_COLORS != 0 {
                cols.push(col(
                    ColKind::Colors,
                    "Colors",
                    Some(SortKey::Colors),
                    68.0,
                    false,
                    true,
                ));
            }
            if tc & TC_RATING != 0 {
                cols.push(col(
                    ColKind::Rating,
                    "Rating",
                    Some(SortKey::Rating),
                    72.0,
                    false,
                    false,
                ));
            }
            if tc & TC_MODIFIED != 0 {
                cols.push(col(
                    ColKind::Modified,
                    "Modified",
                    Some(SortKey::Modified),
                    110.0,
                    false,
                    false,
                ));
            }
            if tc & TC_CREATED != 0 {
                cols.push(col(
                    ColKind::Created,
                    "Created",
                    Some(SortKey::Created),
                    110.0,
                    false,
                    false,
                ));
            }
        }
        // Apply the user's drag-to-reorder column order (data columns only — the
        // thumbnail stays first, the scene Download menu stays last). Unknown / newly
        // shown kinds keep their default relative position (sorted to the end).
        let order = if scene { &self.colo_order } else { &self.table_order };
        if !order.is_empty() {
            let head = 1; // after the thumbnail
            let tail = usize::from(cols.last().map(|c| c.kind) == Some(ColKind::Download));
            let end = cols.len() - tail;
            if head < end {
                cols[head..end].sort_by_key(|c| {
                    order
                        .iter()
                        .position(|&k| k == c.kind as u8)
                        .unwrap_or(usize::MAX)
                });
            }
        }
        // Apply persisted drag-to-resize width overrides to the fixed-width columns
        // (the flex columns — Filename/Artist — absorb whatever's left).
        for c in cols
            .iter_mut()
            .filter(|c| !c.flex && !matches!(c.kind, ColKind::Thumb | ColKind::Download))
        {
            if let Some(&w) = self.col_widths.get(&(c.kind as u8)) {
                c.w = w.clamp(40.0, 600.0);
            }
        }
        let bar = ui.spacing().scroll.bar_width + 2.0;
        let avail = (ui.available_width() - bar).max(200.0);
        let fixed: f32 = cols.iter().filter(|c| !c.flex).map(|c| c.w).sum();
        let flex_n = cols.iter().filter(|c| c.flex).count().max(1);
        let flex_w = ((avail - fixed) / flex_n as f32).max(90.0);
        for c in cols.iter_mut().filter(|c| c.flex) {
            c.w = flex_w;
        }

        // Deferred actions (the body closure borrows `&mut self`).
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let mut hovered: Option<usize> = None;
        let mut header_sort: Option<SortKey> = None;
        let mut menu_sort: Option<(SortKey, bool)> = None; // right-click: (key, descending)
        let mut toggle_col: Option<u16> = None; // right-click: show/hide a column bit
        let mut resize: Option<(u8, f32)> = None; // drag a column border: (ColKind, width)
        let (tc_cur, cs_cur) = (self.table_columns, self.colo_columns);
        // Drag-to-reorder: collect the visible data columns' header rects, the column
        // being dragged, and where it was dropped (pointer x).
        let mut data_rects: Vec<(ColKind, egui::Rect)> = Vec::new();
        let mut drag_kind: Option<ColKind> = None;
        let mut drop_at: Option<(ColKind, f32)> = None;
        let mut ctx_action: Option<(usize, FileAction)> = None;
        let mut pin_dir: Option<usize> = None;
        let mut smart_on: Option<(usize, SmartCriterion)> = None;
        let mut toggle_viewed: Option<(usize, bool)> = None; // "Mark as (not) viewed"
        let mut rate_to: Option<(usize, u8)> = None; // "Rating ▸ N" context-menu choice
        let mut pin_current = false; // "Pin <artist/group/search>" in a flat listing
        let mut dl: Option<(usize, bool)> = None; // (idx, want_pack)
        let mut colo_link: Option<(usize, ColKind)> = None; // pack/year/group link click
        let can_paste = self.clipboard.is_some();
        // In a flat listing the rows are pieces — offer pinning the whole listing.
        let colo_pin_label = self
            .colo_flat
            .then(|| self.folder.as_ref().map(|f| short_name(f)))
            .flatten();
        let colo_pin_pinned = self
            .folder
            .as_ref()
            .is_some_and(|f| self.favorites.contains(f));
        let (sort_key, sort_desc) = (self.sort_key, self.sort_desc);

        // Header row (above the scroll area so it stays put), sharing the body widths.
        // Left-click a header sorts (re-click reverses); right-click → sort asc/desc +
        // show/hide columns; drag a column's right border to resize it.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for c in &cols {
                // A fixed (non-flex) data column ends in a thin drag-to-resize border.
                let resizable = !c.flex && !matches!(c.kind, ColKind::Thumb | ColKind::Download);
                // Data columns (not the thumbnail / download menu) can be dragged to
                // reorder; that needs the cell to sense a drag as well as a click.
                let movable = !matches!(c.kind, ColKind::Thumb | ColKind::Download);
                let handle_w = if resizable { 5.0 } else { 0.0 };
                let cell_w = (c.w - handle_w).max(1.0);
                let sense = if movable {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::click()
                };
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(cell_w, 22.0), sense);
                if movable {
                    data_rects.push((c.kind, rect));
                    if resp.dragged() {
                        drag_kind = Some(c.kind);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                    if resp.drag_stopped() {
                        if let Some(p) = ui.ctx().pointer_interact_pos() {
                            drop_at = Some((c.kind, p.x));
                        }
                    }
                }
                let dragging_this = drag_kind == Some(c.kind);
                let bg = if dragging_this {
                    ui.visuals().widgets.active.bg_fill
                } else {
                    ui.visuals().faint_bg_color
                };
                ui.painter().rect_filled(rect, 0.0, bg);
                if !c.label.is_empty() {
                    let active = c.sort == Some(sort_key);
                    let arrow = if active {
                        if sort_desc {
                            " ⬇"
                        } else {
                            " ⬆"
                        }
                    } else {
                        ""
                    };
                    let fg = if active {
                        ui.visuals().strong_text_color()
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    let (anchor, pos) = if c.num {
                        (
                            egui::Align2::RIGHT_CENTER,
                            rect.right_center() - egui::vec2(cell_pad, 0.0),
                        )
                    } else {
                        (
                            egui::Align2::LEFT_CENTER,
                            rect.left_center() + egui::vec2(cell_pad, 0.0),
                        )
                    };
                    let p = ui
                        .painter()
                        .with_clip_rect(rect.shrink2(egui::vec2(cell_pad * 0.5, 0.0)));
                    p.text(
                        pos,
                        anchor,
                        format!("{}{arrow}", c.label),
                        egui::FontId::proportional(12.5),
                        fg,
                    );
                }
                if resp.clicked() {
                    header_sort = c.sort;
                }
                resp.context_menu(|ui| {
                    if let Some(sk) = c.sort {
                        if ui.button("Sort ascending").clicked() {
                            menu_sort = Some((sk, false));
                            ui.close();
                        }
                        if ui.button("Sort descending").clicked() {
                            menu_sort = Some((sk, true));
                            ui.close();
                        }
                        ui.separator();
                    }
                    ui.weak("Show columns");
                    let (list, mask) = if scene {
                        (COLO_COLUMNS, cs_cur)
                    } else {
                        (TABLE_COLUMNS, tc_cur)
                    };
                    for &(bit, label) in list {
                        let mut on = mask & bit != 0;
                        if ui.checkbox(&mut on, label).changed() {
                            toggle_col = Some(bit);
                        }
                    }
                });
                // Resize border: drag to set this column's width.
                if resizable {
                    let (hrect, hresp) =
                        ui.allocate_exact_size(egui::vec2(handle_w, 22.0), egui::Sense::drag());
                    let hot = hresp.hovered() || hresp.dragged();
                    ui.painter().vline(
                        hrect.center().x,
                        hrect.y_range(),
                        egui::Stroke::new(
                            1.0,
                            if hot {
                                ui.visuals().strong_text_color()
                            } else {
                                ui.visuals().weak_text_color().gamma_multiply(0.4)
                            },
                        ),
                    );
                    if hot {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if hresp.dragged() {
                        let w = (c.w + hresp.drag_delta().x).clamp(40.0, 600.0);
                        resize = Some((c.kind as u8, w));
                    }
                }
            }
            // While reordering, mark where the dragged column will drop — a vertical
            // insertion line at the boundary nearest the pointer.
            if drag_kind.is_some() {
                if let (Some(p), Some((_, first))) =
                    (ui.ctx().pointer_interact_pos(), data_rects.first())
                {
                    let yr = first.y_range();
                    let line_x = data_rects
                        .iter()
                        .filter(|(k, r)| Some(*k) != drag_kind && r.center().x < p.x)
                        .map(|(_, r)| r.right())
                        .next_back()
                        .unwrap_or_else(|| first.left());
                    ui.painter().vline(
                        line_x,
                        yr,
                        egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
                    );
                }
            }
        });
        ui.separator();

        let dark = ui.visuals().dark_mode;
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("table")
            .auto_shrink([false; 2]);
        if let Some(idx) = self.scroll_target.take() {
            scroll = scroll.vertical_scroll_offset(idx as f32 * row_h);
        }
        scroll.show_rows(ui, row_h, self.entries.len(), |ui, range| {
            // Rows must be exactly `row_h` tall to align with `show_rows`' virtualization.
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            for idx in range {
                let entry = self.entries[idx].clone();
                let path = entry.path.clone();
                let is_selected = self.selection.contains(&path);
                let meta = self.img_meta.get(&path).copied();
                let piece = self.colo_pieces.get(&path).cloned();
                let viewed = self.is_viewed(&path); // browser-style visited filename link

                // Request the thumbnail once: a remote piece via the HTTP pool (its `tn`
                // PNG), any other file via the local decoder; both land in `thumb_tex`.
                let tex = self.thumb_tex.get(&path).cloned();
                if tex.is_none() && !entry.is_dir {
                    if let Some(p) = &piece {
                        self.colo_thumbs.request(&path, &p.tn_url, THUMB_PX);
                    } else {
                        self.thumbs.request(&path, THUMB_PX);
                    }
                    self.want_repaint = true;
                }

                // Predict the row rect to highlight on hover the same frame it's drawn.
                let row_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(avail, row_h));
                let hover_row = ui.rect_contains_pointer(row_rect);
                let bg = if is_selected {
                    ui.visuals().selection.bg_fill
                } else if hover_row {
                    ui.visuals().widgets.hovered.bg_fill
                } else if idx % 2 == 1 {
                    ui.visuals().faint_bg_color
                } else {
                    egui::Color32::TRANSPARENT
                };
                let fg = if is_selected {
                    ui.visuals().strong_text_color()
                } else if dark {
                    egui::Color32::from_gray(205)
                } else {
                    ui.visuals().text_color()
                };

                let mut row_resp: Option<egui::Response> = None;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for c in cols.iter() {
                        // The scene download column is an interactive menu, not a cell.
                        if c.kind == ColKind::Download && !entry.is_dir {
                            ui.allocate_ui_with_layout(
                                egui::vec2(c.w, row_h),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_min_size(egui::vec2(c.w, row_h));
                                    ui.painter().rect_filled(ui.max_rect(), 0.0, bg);
                                    ui.add_space(8.0);
                                    ui.menu_button("⬇", |ui| {
                                        if ui.button("Download file").clicked() {
                                            dl = Some((idx, false));
                                            ui.close();
                                        }
                                        if ui.button("Download pack .zip").clicked() {
                                            dl = Some((idx, true));
                                            ui.close();
                                        }
                                    });
                                },
                            );
                            continue;
                        }
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(c.w, row_h), egui::Sense::click());
                        ui.painter().rect_filled(rect, 0.0, bg);
                        if c.kind == ColKind::Thumb {
                            // Folders + archives get the folder glyph; archives also get a
                            // format badge (e.g. ZIP) so they read as distinct + enterable.
                            // Any other file shows its thumbnail once decoded.
                            if entry.is_dir || entry.is_archive {
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "📁",
                                    egui::FontId::proportional(row_h * 0.46),
                                    fg,
                                );
                                if entry.is_archive {
                                    let fmt = entry
                                        .path
                                        .extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("")
                                        .to_ascii_uppercase();
                                    if !fmt.is_empty() {
                                        let p = ui.painter_at(rect);
                                        let galley = p.layout_no_wrap(
                                            fmt,
                                            egui::FontId::proportional(8.0),
                                            egui::Color32::WHITE,
                                        );
                                        let pos = egui::pos2(
                                            rect.center().x - galley.size().x * 0.5,
                                            rect.bottom() - galley.size().y - 3.0,
                                        );
                                        p.rect_filled(
                                            egui::Rect::from_min_size(pos, galley.size())
                                                .expand(2.0),
                                            2.0,
                                            egui::Color32::from_rgba_unmultiplied(90, 78, 30, 210),
                                        );
                                        p.galley(pos, galley, egui::Color32::WHITE);
                                    }
                                }
                            } else if let Some(tex) = &tex {
                                let fit = fit_centered(rect.shrink(4.0), tex.size_vec2());
                                ui.painter().image(
                                    tex.id(),
                                    fit,
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
                                    egui::Color32::WHITE,
                                );
                            } else if !entry.is_dir {
                                // Spinner while the row's thumbnail loads (the request was
                                // already issued above when tex was None).
                                let t = ui.input(|i| i.time);
                                paint_spinner(
                                    &ui.painter_at(rect),
                                    rect.center(),
                                    (row_h * 0.26).clamp(6.0, 11.0),
                                    t,
                                    egui::Color32::from_gray(120),
                                );
                            }
                        } else if c.kind == ColKind::Rating {
                            if entry.rating > 0 {
                                ui.painter().text(
                                    rect.left_center() + egui::vec2(6.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    "★".repeat(entry.rating as usize),
                                    egui::FontId::proportional(13.0),
                                    egui::Color32::from_rgb(255, 200, 60),
                                );
                            }
                        } else {
                            let txt = table_cell_text(&entry, meta, piece.as_ref(), c.kind);
                            if !txt.is_empty() {
                                // Pack / Year / Group are clickable links into the 16colo
                                // browser (tinted + underlined on hover).
                                let linkable = matches!(
                                    c.kind,
                                    ColKind::Pack | ColKind::Year | ColKind::Group
                                ) && piece.is_some();
                                let hot = linkable && resp.hovered();
                                // The filename is a browser-style visited link: unvisited
                                // = accent colour + underline, visited = muted, no line.
                                let name_link = c.kind == ColKind::Name;
                                let name_unvisited = name_link && !viewed;
                                let col = if hot {
                                    egui::Color32::from_rgb(120, 180, 255)
                                } else if linkable {
                                    egui::Color32::from_rgb(95, 155, 235)
                                } else if name_unvisited {
                                    ui.visuals().hyperlink_color
                                } else if name_link {
                                    ui.visuals().weak_text_color()
                                } else {
                                    fg
                                };
                                // Budget chars to the *padded* inner width with a
                                // conservative px/char so a proportional glyph string
                                // can't overrun into the next column.
                                let inner = (c.w - cell_pad * 2.0).max(8.0);
                                let max_chars = (inner / 7.0).max(1.0) as usize;
                                let galley = ui.painter().layout_no_wrap(
                                    elide(&txt, max_chars),
                                    egui::FontId::proportional(12.5),
                                    col,
                                );
                                let y = rect.center().y - galley.size().y * 0.5;
                                let tp = if c.num {
                                    egui::pos2(rect.right() - cell_pad - galley.size().x, y)
                                } else {
                                    egui::pos2(rect.left() + cell_pad, y)
                                };
                                // Clip to the cell so text can never touch the neighbour,
                                // even if an estimate is slightly off.
                                let p = ui
                                    .painter()
                                    .with_clip_rect(rect.shrink2(egui::vec2(cell_pad * 0.5, 0.0)));
                                if hot || name_unvisited {
                                    let uy = tp.y + galley.size().y - 1.0;
                                    p.line_segment(
                                        [
                                            egui::pos2(tp.x, uy),
                                            egui::pos2(tp.x + galley.size().x, uy),
                                        ],
                                        egui::Stroke::new(1.0, col),
                                    );
                                }
                                if resp.hovered() && (linkable || name_link) {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                p.galley(tp, galley, col);
                                // A click on a link cell navigates instead of opening the art.
                                if linkable && resp.clicked() {
                                    colo_link = Some((idx, c.kind));
                                }
                            }
                        }
                        row_resp = Some(match row_resp.take() {
                            Some(r) => r.union(resp),
                            None => resp,
                        });
                    }
                });

                if let Some(resp) = row_resp {
                    let resp = resp.on_hover_ui(|ui| hover_details(ui, &entry, meta));
                    if resp.hovered() {
                        hovered = Some(idx);
                    }
                    if resp.clicked() {
                        clicked = Some((idx, ui.input(|i| i.modifiers)));
                    }
                    let pinned = self.favorites.iter().any(|f| f == &entry.path);
                    let colo_pin = colo_pin_label.as_deref().map(|l| (l, colo_pin_pinned));
                    let colo_piece = piece.is_some();
                    resp.context_menu(|ui| {
                        if let Some(pick) = entry_context_menu(
                            ui, &entry, can_paste, pinned, viewed, colo_pin, colo_piece,
                        ) {
                            match pick {
                                TilePick::Pin => pin_dir = Some(idx),
                                TilePick::PinFolder => pin_current = true,
                                TilePick::Smart(c) => smart_on = Some((idx, c)),
                                TilePick::File(a) => ctx_action = Some((idx, a)),
                                TilePick::ToggleViewed(b) => toggle_viewed = Some((idx, b)),
                                TilePick::Download(pack) => dl = Some((idx, pack)),
                                TilePick::Rate(stars) => rate_to = Some((idx, stars)),
                            }
                        }
                    });
                }
            }
        });

        // Apply the deferred actions (now that the body's `&mut self` borrow is done).
        self.hovered = hovered;
        if let Some(i) = hovered {
            self.last_inspected = self.entries.get(i).map(|e| e.path.clone());
        }
        if let Some(k) = header_sort {
            // Click the active column again to reverse; a new column starts ascending.
            if self.sort_key == k {
                self.sort_desc = !self.sort_desc;
            } else {
                self.sort_key = k;
                self.sort_desc = false;
            }
            self.rebuild_view();
        }
        // Right-click menu: explicit ascending / descending sort.
        if let Some((k, desc)) = menu_sort {
            self.sort_key = k;
            self.sort_desc = desc;
            self.rebuild_view();
        }
        // Right-click menu: show/hide a column (toggle the layout's bitmask).
        if let Some(bit) = toggle_col {
            if self.colo_flat {
                self.colo_columns ^= bit;
            } else {
                self.table_columns ^= bit;
            }
        }
        // Drag-to-resize: remember this column's new width (used next frame).
        if let Some((kind, w)) = resize {
            self.col_widths.insert(kind, w);
        }
        // Drag-to-reorder: drop the dragged column at the boundary under the pointer.
        if let Some((dragged, ptr_x)) = drop_at {
            let others: Vec<(ColKind, f32)> = data_rects
                .iter()
                .filter(|(k, _)| *k != dragged)
                .map(|(k, r)| (*k, r.center().x))
                .collect();
            let idx = others.iter().filter(|(_, cx)| *cx < ptr_x).count();
            let mut new: Vec<u8> = others.iter().map(|(k, _)| *k as u8).collect();
            new.insert(idx.min(new.len()), dragged as u8);
            if self.colo_flat {
                self.colo_order = new;
            } else {
                self.table_order = new;
            }
        }
        // A pack/year/group link click navigates the 16colo browser; it takes priority
        // over the row's open-the-art click (the link cell is part of the row response).
        if let Some((idx, kind)) = colo_link {
            let dest = self.entries.get(idx).map(|e| e.path.clone()).and_then(|p| {
                self.colo_pieces.get(&p).map(|pc| {
                    let root = Path::new(crate::sixteen::ROOT);
                    match kind {
                        ColKind::Pack => root.join(pc.year.to_string()).join(&pc.pack),
                        ColKind::Year => root.join(pc.year.to_string()),
                        ColKind::Group => root.join(crate::sixteen::GROUPS).join(&pc.group),
                        _ => root.to_path_buf(),
                    }
                })
            });
            if let Some(dest) = dest {
                self.open_folder(dest);
            }
        } else if let Some((idx, mods)) = clicked {
            self.handle_click(ctx, idx, mods);
        }
        if let Some((idx, a)) = ctx_action {
            if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
                if !self.selection.contains(&p)
                    && !matches!(a, FileAction::Paste | FileAction::NewFolder)
                {
                    self.selection.clear();
                    self.selection.insert(p);
                    self.hovered = Some(idx);
                }
            }
            self.do_file_action(a);
        }
        if let Some(idx) = pin_dir {
            if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
                if !self.favorites.contains(&p) {
                    self.favorites.push(p);
                    self.status = "Pinned to Places".into();
                }
            }
        }
        if let Some((idx, crit)) = smart_on {
            if let Some(e) = self.entries.get(idx).cloned() {
                self.smart_filter_from(&e, crit);
            }
        }
        if let Some((idx, b)) = toggle_viewed {
            if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
                self.set_viewed(&p, b);
            }
        }
        if let Some((idx, stars)) = rate_to {
            self.rate_entry(idx, stars);
        }
        if pin_current {
            self.pin_current_folder();
        }
        if let Some((idx, want_pack)) = dl {
            if let Some(p) = self.entries.get(idx).map(|e| e.path.clone()) {
                self.download_piece(&p, want_pack);
            }
        }
    }

    /// "Download file/pack" from a 16colo.rs flat-listing row: ask the user where to
    /// save, then fetch on a background thread (the file's `raw` URL, or its pack zip)
    /// and report completion via `colo_save_rx`. No-op for a non-piece path.
    fn download_piece(&mut self, path: &Path, want_pack: bool) {
        let Some(piece) = self.colo_pieces.get(path) else {
            return;
        };
        let (url, default_name) = if want_pack {
            (
                crate::sixteen::pack_url(piece.year, &piece.pack),
                format!("{}.zip", piece.pack),
            )
        } else {
            (
                piece.raw_url.clone(),
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("download")
                    .to_string(),
            )
        };
        let Some(dest) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .save_file()
        else {
            return; // user cancelled
        };
        self.status = format!("Downloading {default_name}…");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let msg = match crate::sixteen::download_to(&url, &dest) {
                Ok(()) => format!("Saved {}", short_name(&dest)),
                Err(e) => format!("Download failed: {e}"),
            };
            let _ = tx.send(msg);
        });
        self.colo_save_rx = Some(rx);
    }

    fn ui_single(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let prev_key = self.key_for(Action::PrevImage);
        let next_key = self.key_for(Action::NextImage);
        let (prev, next, fit, tile) = ui.input(|i| {
            (
                i.key_pressed(prev_key),
                i.key_pressed(next_key),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::T),
            )
        });
        // Any user input during baud playback finishes the animation *now* and hands
        // control back — the auto-scroll stops following the cursor so the user can
        // scroll/pan freely. Triggered by a scroll, a zoom gesture, or any key press;
        // that input's own action is suppressed this frame (it just aborts the playback).
        let playing = self.player.as_ref().is_some_and(|p| p.playing);
        let interrupt = playing
            && ctx.input(|i| {
                i.smooth_scroll_delta != egui::Vec2::ZERO
                    || (i.zoom_delta() - 1.0).abs() > 0.001
                    || i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
            });
        if interrupt {
            if let Some(p) = self.player.as_mut() {
                p.pos = p.len; // draw everything immediately
                p.playing = false;
                p.tex = None;
            }
            self.play_autoscroll = None; // release the cursor-follow so the user can scroll
        }

        // Auto-pause: while the slideshow is running, any deliberate user interaction
        // (scroll, zoom, key, or a drag-to-pan) hands control back — pause auto-advance
        // and flag it (the status-bar "auto ▶" turns yellow). Passive mouse *movement*
        // is intentionally excluded so a hands-off screensaver isn't paused by a nudge.
        if self.auto_next && !self.auto_paused {
            let touched = ctx.input(|i| {
                i.smooth_scroll_delta != egui::Vec2::ZERO
                    || (i.zoom_delta() - 1.0).abs() > 0.001
                    || i.pointer.is_decidedly_dragging()
                    || i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
            });
            if touched {
                self.auto_paused = true;
            }
        }
        if self.rebinding.is_none() && !interrupt {
            if prev {
                self.step_image(ctx, false);
            }
            if next {
                self.step_image(ctx, true);
            }
            if self.path_edit.is_none() {
                if fit {
                    // Toggle sticky fit; fit immediately when turning it on.
                    self.fit_mode = !self.fit_mode;
                    if self.fit_mode {
                        self.fit_requested = true;
                    }
                }
                if tile {
                    self.tile_mode = !self.tile_mode;
                }
            }
        }

        // Slideshow: auto-advance to the next file after it has "settled" (a baud
        // transmission, if any, finished typing) plus the chosen delay — great for
        // flipping through a whole pack hands-free.
        if self.auto_next && !self.auto_paused && self.path_edit.is_none() && self.rebinding.is_none() {
            let busy = self.player.as_ref().is_some_and(|p| p.playing);
            if busy {
                self.auto_next_dwell = 0.0; // wait for the art to finish drawing
            } else {
                self.auto_next_dwell += ctx.input(|i| i.stable_dt);
                if self.auto_next_dwell >= self.auto_next_secs as f32 {
                    self.auto_next_dwell = 0.0;
                    let before = self.selected;
                    self.step_image(ctx, true); // next file
                                                // End of the pack (step was a no-op) + Shuffle → jump to another
                                                // random pack and keep going: an endless real-art screensaver.
                    if self.selected == before && self.shuffle && self.random_rx.is_none() {
                        self.start_random_pack();
                    }
                }
            }
            self.want_repaint = true; // keep the dwell timer ticking
        }

        // In immersive (F11) mode the controls row is hidden too, for a fully black screen.
        let immersive = self.immersive;

        // Animated GIF: a controls row (play/pause, seek, frame info) + the frame.
        if self.anim.is_some() {
            let tex = {
                let anim = self.anim.as_mut().unwrap();
                let n = anim.frames.len();
                if anim.playing && n > 1 {
                    let dt_ms = ctx.input(|i| i.stable_dt) * 1000.0;
                    anim.acc_ms += dt_ms;
                    let mut guard = 0;
                    while anim.acc_ms >= f32::from(anim.delays_ms[anim.current]) && guard < n {
                        anim.acc_ms -= f32::from(anim.delays_ms[anim.current]);
                        anim.current = (anim.current + 1) % n;
                        guard += 1;
                    }
                }
                if !immersive {
                    ui.horizontal(|ui| {
                        if ui
                            .button(if anim.playing {
                                "⏸ Pause"
                            } else {
                                "▶ Play"
                            })
                            .clicked()
                        {
                            anim.playing = !anim.playing;
                        }
                        let mut cur = anim.current;
                        if ui
                            .add(egui::Slider::new(&mut cur, 0..=n.saturating_sub(1)).text("frame"))
                            .changed()
                        {
                            anim.current = cur;
                            anim.acc_ms = 0.0;
                            anim.playing = false;
                        }
                        let delay = anim.delays_ms.get(anim.current).copied().unwrap_or(0);
                        let total: u32 = anim.delays_ms.iter().map(|&d| u32::from(d)).sum();
                        ui.label(format!(
                            "frame {}/{}  ·  {}ms  ·  {} frames  ·  {}ms loop",
                            anim.current + 1,
                            n,
                            delay,
                            n,
                            total
                        ));
                    });
                }
                TiledTexture::single(anim.frames[anim.current].clone(), anim.size)
            };
            if self.anim.as_ref().is_some_and(|a| a.playing) {
                self.want_repaint = true; // keep animating
            }
            if let Some(forward) = self.draw_image_view(ui, &tex) {
                self.step_image(ctx, forward);
            }
            return;
        }

        // Baud-rate playback (ANSImation / "watch RIP draw"): a controls row + the
        // progressively-rendered frame. When fully played out (or paused at the end) it
        // falls through to the static path below, so recolor / minimap keep working on
        // the complete image. The baud picker itself lives in the status bar.
        if self.player.is_some() {
            let dt = ctx.input(|i| i.stable_dt);
            let baud = self.current_baud();
            let (active, just_finished) = {
                let p = self.player.as_mut().unwrap();
                let was_playing = p.playing;
                p.advance(baud, dt);
                let len = p.len;
                if !immersive {
                    ui.horizontal(|ui| {
                        if ui
                            .button(if p.playing { "⏸ Pause" } else { "▶ Play" })
                            .clicked()
                        {
                            if p.pos >= p.len {
                                p.pos = 0; // at the end → replay from the start
                                p.acc = 0.0;
                            }
                            p.playing = !p.playing;
                        }
                        if ui.button("⏮ Replay").clicked() {
                            p.pos = 0;
                            p.acc = 0.0;
                            p.playing = true;
                        }
                        let mut pos = p.pos;
                        if ui
                            .add(egui::Slider::new(&mut pos, 0..=len).text("byte"))
                            .changed()
                        {
                            p.pos = pos;
                            p.acc = 0.0;
                            p.playing = false;
                        }
                        let pct = if len > 0 { p.pos * 100 / len } else { 100 };
                        ui.label(format!(
                            "{pct}%  ·  {} / {len} bytes  ·  {} baud",
                            p.pos,
                            baud.label()
                        ));
                    });
                }
                // Render one extra frame the moment it finishes, so the auto-scroll lands
                // exactly at the bottom (without it the last drawn frame is a tick short).
                let just_finished = was_playing && !p.playing;
                (p.playing || p.pos < p.len || just_finished, just_finished)
            };
            if active {
                let tex = self.player.as_mut().unwrap().frame(ctx);
                let (playing, cursor_px) = {
                    let p = self.player.as_ref().unwrap();
                    (p.playing, p.cursor_px)
                };
                // Follow the typing cursor down a long ANSImation (BBS-style), and snap to
                // the bottom on the finishing frame; short art that fits the viewport never
                // scrolls (the pan clamp keeps it put). Paused/seeking leaves panning free.
                self.play_autoscroll =
                    ((playing || just_finished) && cursor_px > 0).then_some(cursor_px as f32);
                if playing {
                    self.want_repaint = true; // keep the transmission animating
                }
                if let Some(forward) = self.draw_image_view(ui, &tex) {
                    self.step_image(ctx, forward);
                }
                self.play_autoscroll = None;
                return;
            }
            // else: at rest → fall through to the static view (recolor / minimap).
        }

        let Some((path, tex)) = self.full_tex.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label("Nothing loaded.");
            });
            return;
        };
        // Recolor view: apply the adjustments + swap palette / Reduce (shared with
        // the details pane) to the full image.
        let recolor = self.active_recolor(&path);
        let proc = if !self.pipeline_active() && recolor.is_none() {
            None
        } else {
            let rkey = recolor.as_ref().map(|(k, _)| k.as_str()).unwrap_or("orig");
            let key = format!("{}|{rkey}", self.pipeline_key());
            let pal = recolor.as_ref().map(|(_, p)| p.clone());
            Some((key, pal))
        };
        let tex = match &proc {
            Some((key, pal)) => self
                .make_full_reduced(ctx, &path, key, pal.as_deref())
                .unwrap_or(tex),
            None => tex,
        };
        if let Some(forward) = self.draw_image_view(ui, &tex) {
            // `interrupt` aborted a playback this frame → its scroll/key just stops the
            // animation, it doesn't also step to the next image.
            if !interrupt {
                self.step_image(ctx, forward);
            }
        }
    }

    /// Step the viewer zoom by one whole **device** pixel per source pixel — the
    /// pixel-perfect ladder. `n` is the current device scale (the "N×" the status bar
    /// shows); we move it ±1 and convert back to logical zoom via `ppp`. Keeping the
    /// step in device units means a Snap/Z step is always exactly one crisp level,
    /// even on a fractionally-scaled display (where logical 100% steps would stick).
    fn step_device_zoom(&mut self, ppp: f32, up: bool) {
        let n = (self.zoom * ppp).round().max(1.0);
        let n = if up { n + 1.0 } else { (n - 1.0).max(1.0) };
        self.zoom = (n / ppp).clamp(0.05, 64.0);
    }

    /// A crisp minimap texture for the navigator overview. The shared thumbnail is
    /// capped at ~512px on its long side, so the tall viewer strip would upscale it
    /// blurry; instead area-average the full-res CPU pixels (`full_src`) down to the
    /// strip's *device* resolution (`dpx`) — sharp, and no moiré on dithered art.
    /// Cached by (path, size); returns None when the full pixels aren't this image's
    /// (e.g. a GIF, or not decoded yet) so the caller can fall back to the thumbnail.
    fn minimap_texture(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        dpx: [usize; 2],
    ) -> Option<egui::TextureId> {
        if let Some((p, sz, tex)) = &self.minimap {
            if p == path && *sz == dpx {
                return Some(tex.id());
            }
        }
        let (sp, ssz, rgba) = self.full_src.as_ref()?;
        if sp.as_path() != path {
            return None;
        }
        let down = crate::thumb::box_downscale(rgba, ssz[0], ssz[1], dpx[0], dpx[1]);
        let color = egui::ColorImage::from_rgba_unmultiplied(dpx, &down);
        let tex = ctx.load_texture("minimap", color, egui::TextureOptions::LINEAR);
        let id = tex.id();
        self.minimap = Some((path.to_path_buf(), dpx, tex));
        Some(id)
    }

    /// Blit the art (panned, at `art_rect`) into the `viewport` and lay scanlines over the
    /// *window* (darkness from the slider, period from `scanline_period_dpx`).
    fn paint_crt(
        &mut self,
        painter: &egui::Painter,
        viewport: egui::Rect,
        art_rect: egui::Rect,
        tex: egui::TextureId,
        ppp: f32,
        src_px_dpx: f32,
    ) {
        let ctx = painter.ctx().clone();
        let full = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(tex, art_rect, full, egui::Color32::WHITE); // flat, panned
                                                                  // Phosphor glow: redraw the art a few times, offset in a ring, with ADDITIVE
                                                                  // blending — a premultiplied tint with alpha 0 turns egui's "over" blend into
                                                                  // src+dst. Dark areas add ~nothing; bright glyphs bloom into a soft halo (that
                                                                  // late-night-BBS phosphor vibe). The radius tracks the zoom so it stays visible.
        if self.glow && self.glow_amt > 0.0 {
            let r = (src_px_dpx * 0.5).clamp(1.0, 4.0) / ppp; // glow radius in points
            let k = (self.glow_amt * 32.0).round() as u8;
            let add = egui::Color32::from_rgba_premultiplied(k, k, k, 0);
            for (dx, dy) in [
                (-1.0, 0.0),
                (1.0, 0.0),
                (0.0, -1.0),
                (0.0, 1.0),
                (-1.0, -1.0),
                (1.0, -1.0),
                (-1.0, 1.0),
                (1.0, 1.0),
            ] {
                painter.image(
                    tex,
                    art_rect.translate(egui::vec2(dx * r, dy * r)),
                    full,
                    add,
                );
            }
        }
        if self.crt_scanline_dark > 0.0 {
            let st = self.scanline_texture(&ctx);
            // Tile the 1×3 dark-line pattern down the whole viewport; the slider sets the
            // line darkness via the tint alpha (the texture stays fixed/opaque).
            let period = self.scanline_period_dpx(src_px_dpx);
            let lines = (viewport.height() * ppp / period).max(1.0);
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, lines));
            let alpha = (self.crt_scanline_dark * 255.0).round() as u8;
            painter.image(st, viewport, uv, egui::Color32::from_white_alpha(alpha));
        }
    }

    /// Device-pixel period of the scanline pattern (one dark line per `period` px). Fixed
    /// fine lines by default; with "scale" on it tracks one line per source-pixel row
    /// (`src_px_dpx` = device px per source pixel), so the lines grow as you zoom in.
    fn scanline_period_dpx(&self, src_px_dpx: f32) -> f32 {
        if self.crt_scanline_scale {
            src_px_dpx.max(2.0)
        } else {
            3.0
        }
    }

    /// A 1×3 scanline pattern — two clear rows, one opaque-black row — tiled vertically.
    /// Opaque so the draw-time tint alpha (the darkness slider) controls intensity without
    /// rebuilding the texture. Cached.
    fn scanline_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.scanline_tex.is_none() {
            let px: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255]; // clear, clear, black
            let img = egui::ColorImage::from_rgba_unmultiplied([1, 3], &px);
            self.scanline_tex =
                Some(ctx.load_texture("crt_scanline", img, egui::TextureOptions::NEAREST_REPEAT));
        }
        self.scanline_tex.as_ref().unwrap().id()
    }

    /// Single-view image painter: wheel-zoom + pinch + drag-pan (persisted zoom),
    /// then blit `tt` at its logical size. Shared by static images and GIF frames.
    /// Returns `Some(forward)` when a plain wheel turn should step the image
    /// (`true` = next, `false` = previous) — the caller has the `ctx` to do it.
    fn draw_image_view(&mut self, ui: &mut egui::Ui, tt: &TiledTexture) -> Option<bool> {
        let (sw, sh) = (tt.size[0] as f32, tt.size[1] as f32);
        let avail = ui.available_size();
        // The open image's path — used to fetch its area-averaged thumbnail for the
        // navigator (a NEAREST-downscaled full-res blit would alias into moiré).
        let cur_path = self.entries.get(self.selected).map(|e| e.path.clone());
        // Text-mode art optionally gets the non-square VGA aspect (≈1.2× taller), so an
        // 80×25 screen lands at the 4:3 shape a CRT showed. Computed before Fit because
        // the fit math must use the *stretched* height (else Fit overflows the viewport).
        let aspect_y = if self.viewing_textmode && self.crt_aspect {
            1.2
        } else {
            1.0
        };
        // Fit-to-window (requested by '.' or the Fit button).
        if self.fit_requested {
            self.fit_requested = false;
            if sw > 0.0 && sh > 0.0 {
                self.zoom = (avail.x / sw)
                    .min(avail.y / (sh * aspect_y))
                    .clamp(0.05, 64.0);
                self.offset = egui::Vec2::ZERO;
            }
        }
        let (resp, painter) = ui.allocate_painter(avail, egui::Sense::click_and_drag());
        // Standardized with the grid: Ctrl+wheel (and pinch) zoom; a plain wheel
        // turn steps to the previous/next image (wheel up = previous).
        let (zoom_delta, zoom_dir, wheel, scroll) = ui.input(|i| {
            let mut wheel = 0.0;
            let mut zoom_dir = 0i32; // net *discrete* Ctrl+wheel notches this frame
            for e in &i.events {
                if let egui::Event::MouseWheel {
                    delta, modifiers, ..
                } = e
                {
                    if modifiers.command {
                        zoom_dir += delta.y.signum() as i32;
                    } else {
                        wheel += delta.y;
                    }
                }
            }
            // `smooth_scroll_delta` is DPI-normalized points with Ctrl+wheel already
            // excluded (it's a zoom gesture) — the right signal for panning a long file.
            (i.zoom_delta(), zoom_dir, wheel, i.smooth_scroll_delta.y)
        });
        // Pixel-perfect mode (text-mode art, or Snap on) renders/zooms in whole
        // *device* pixels per source pixel — the only scale nearest-neighbor keeps
        // undistorted (see the blit below). `ppp` folds in fractional desktop scaling.
        let ppp = ui.ctx().pixels_per_point();
        let pixel_perfect = self.viewing_textmode || self.zoom_lock;
        if resp.hovered() {
            if pixel_perfect {
                // One device-pixel ladder step per *physical notch*. `zoom_delta()` is
                // smoothed and fires across several frames per scroll, so counting raw
                // wheel events avoids the old 100%→900% jump — and stepping the device
                // scale (not logical zoom) avoids fighting the snap on fractional DPI.
                for _ in 0..zoom_dir.unsigned_abs() {
                    self.step_device_zoom(ppp, zoom_dir > 0);
                }
            } else if zoom_delta != 1.0 {
                self.zoom = (self.zoom * zoom_delta).clamp(0.05, 64.0);
            }
        }
        // One-shot on opening text-mode art: cap the zoom so the FULL width fits the
        // viewport. A wide/landscape ANSI opened at the default text zoom (e.g. 3×) would
        // overflow and clip off the sides; tall art is unaffected (its width already
        // fits, so the cap is a no-op there). Capped in whole device pixels so the
        // pixel-perfect snap below keeps it fitting.
        if self.fit_width_on_open {
            self.fit_width_on_open = false;
            if sw > 0.0 && avail.x > 0.0 {
                let fit_nx = (avail.x * ppp / sw).floor().max(1.0);
                let cur_nx = (self.zoom * ppp).round().max(1.0);
                self.zoom = cur_nx.min(fit_nx) / ppp;
            }
        }
        // Pixel-perfect blit: nearest-neighbor only stays undistorted when one source
        // pixel maps to a *whole* number of device pixels. So for text-mode art (and
        // whenever Snap is on) we round to an integer device-pixel scale per axis —
        // `scale` is then points-per-source-pixel. Without this, fractional display
        // scaling / zoom / the CRT stretch duplicate some pixels more than others and
        // warp the dither. `self.zoom` is re-aligned to the value we can show crisply.
        let scale = if pixel_perfect {
            let nx = (self.zoom * ppp).round().max(1.0); // device px per source px, X
            // Keep X pixel-perfect for dither crispness. For Y, *rounding* the CRT's ≈1.2×
            // to whole device pixels makes it vanish at low zoom (round(2·1.2)=2, no change
            // — why the main view "didn't update" on a fit-to-screen tall ANSI while the
            // linear-sampled previews did). So round Y when that still leaves a visible
            // stretch (high zoom → uniform & crisp), but fall back to the exact fractional
            // ratio when rounding would erase it (low zoom → the stretch always shows).
            // aspect_y == 1.0 → ny == nx, so a non-CRT image stays perfectly crisp.
            let ny_round = (nx * aspect_y).round().max(1.0);
            let ny = if ny_round > nx { ny_round } else { nx * aspect_y }; // …Y
            self.zoom = nx / ppp; // idempotent: round((nx/ppp)·ppp) == nx
            egui::vec2(nx / ppp, ny / ppp)
        } else {
            egui::vec2(self.zoom, self.zoom * aspect_y)
        };
        let img_px = egui::vec2(sw, sh) * scale;
        let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let bg = if self.black_bg {
            egui::Color32::BLACK
        } else {
            egui::Color32::from_gray(28)
        };
        painter.rect_filled(resp.rect, 0.0, bg);

        // Wallpaper/tile test mode: infinite pan, no scroll clamp or navigator.
        if self.tile_mode && tt.tiles.len() == 1 && img_px.x >= 1.0 && img_px.y >= 1.0 {
            if resp.dragged() {
                self.offset += resp.drag_delta();
            }
            let uv = egui::Rect::from_min_size(
                egui::pos2(-self.offset.x / img_px.x, -self.offset.y / img_px.y),
                egui::vec2(resp.rect.width() / img_px.x, resp.rect.height() / img_px.y),
            );
            painter.image(tt.tiles[0].tex.id(), resp.rect, uv, egui::Color32::WHITE);
            return None;
        }

        // How far the image overflows the viewport per axis (0 if it fits): the pan
        // limit, and the trigger for wheel-scroll + the navigator/minimap.
        let ox_max = ((img_px.x - resp.rect.width()) / 2.0).max(0.0);
        let oy_max = ((img_px.y - resp.rect.height()) / 2.0).max(0.0);
        let overflow_x = ox_max > 0.5;
        let overflow_y = oy_max > 0.5;

        // A freshly opened image starts at its top-left corner (not centered), so tall
        // art opens at the top. Now that the on-screen size is known, snap there once;
        // an image that fits an axis stays centered (its max is 0).
        if self.view_to_top {
            self.view_to_top = false;
            self.offset = egui::vec2(ox_max, oy_max);
        }

        // Navigator (GIMP/VSCode-style minimap), flush right, only when overflowing.
        // Use the *stretched* height (`sh * aspect_y`) so the minimap matches the main
        // view's CRT aspect — otherwise text-mode art looks squished in the strip
        // relative to what it's previewing.
        let nav = (!self.immersive && (overflow_x || overflow_y)).then(|| {
            const M: f32 = 8.0;
            const MAX_W: f32 = 120.0;
            let max_h = (resp.rect.height() - 2.0 * M).max(16.0);
            let sh_a = sh * aspect_y;
            let s = (MAX_W / sw).min(max_h / sh_a);
            let nsz = egui::vec2(sw * s, sh_a * s);
            let ntl = egui::pos2(resp.rect.max.x - M - nsz.x, resp.rect.min.y + M);
            egui::Rect::from_min_size(ntl, nsz)
        });

        // --- pan / scroll interaction ---
        let mut advance: Option<bool> = None;
        let ptr = resp.interact_pointer_pos();
        let on_nav = nav.zip(ptr).is_some_and(|(r, p)| r.contains(p));
        // A drag that STARTS on the navigator stays in navigator mode for its whole
        // duration, even after the cursor leaves the strip. Otherwise, crossing the
        // top/bottom edge switched to image-pan — which scrolls the opposite way — so
        // dragging off the top while already at the top scrolled the image *down*.
        if resp.drag_started() {
            self.nav_drag = on_nav;
        }
        if self.nav_drag || (on_nav && resp.clicked()) {
            // Click/drag the navigator → center that point of the image in the viewport.
            // Off-strip cursor positions clamp to [0,1], so the red box pins to the edge
            // and the view stops at the matching corner instead of inverting.
            if let (Some(r), Some(p)) = (nav, ptr) {
                let nx = ((p.x - r.min.x) / r.width()).clamp(0.0, 1.0);
                let ny = ((p.y - r.min.y) / r.height()).clamp(0.0, 1.0);
                self.offset = egui::vec2(img_px.x * (0.5 - nx), img_px.y * (0.5 - ny));
            }
        } else if resp.dragged() {
            self.offset += resp.drag_delta();
        }
        if !resp.dragged() {
            self.nav_drag = false;
        }
        if resp.hovered() {
            if overflow_y && scroll != 0.0 {
                self.offset.y += scroll; // wheel scrolls a long file (clamped below)
            } else if !overflow_y && wheel != 0.0 {
                advance = Some(wheel < 0.0); // a file that fits: wheel = prev/next
            }
        }
        // Up/Down arrows scroll a long file (Left/Right are prev/next). Auto-repeat while
        // held; one keypress nudges ~⅛ of the viewport. Only when there's overflow to pan.
        if overflow_y {
            let (up, down, home, end, pgup, pgdn) = ui.input(|i| {
                use egui::Key::*;
                (
                    i.key_pressed(ArrowUp),
                    i.key_pressed(ArrowDown),
                    i.key_pressed(Home),
                    i.key_pressed(End),
                    i.key_pressed(PageUp),
                    i.key_pressed(PageDown),
                )
            });
            let step = resp.rect.height() * 0.125;
            if down {
                self.offset.y -= step; // scroll down → reveal lower content
            }
            if up {
                self.offset.y += step;
            }
            // Home/End jump to top/bottom; PageUp/PageDown move a "page" — 25 character
            // rows for scene art (an old 80×25 DOS screen), else ~a screenful for raster.
            // (+offset.y = toward the top; the clamp below keeps it in bounds.)
            let page = if self.viewing_textmode {
                let cell_h = self
                    .full_tex
                    .as_ref()
                    .map(|(p, _)| textmode_cell(p).1)
                    .unwrap_or(16);
                25.0 * cell_h as f32 * scale.y
            } else {
                resp.rect.height() * 0.9
            };
            if home {
                self.offset.y = oy_max;
            }
            if end {
                self.offset.y = -oy_max;
            }
            if pgdn {
                self.offset.y -= page;
            }
            if pgup {
                self.offset.y += page;
            }
        }
        // Baud playback: keep the typing cursor at the bottom of the viewport so a long
        // ANSImation scrolls as it draws, like a BBS terminal. The clamp below pins it to
        // the top until the content actually overflows (short art never scrolls).
        if let Some(cursor_src_y) = self.play_autoscroll {
            self.offset.y = resp.rect.height() / 2.0 + img_px.y / 2.0 - cursor_src_y * scale.y;
        }
        // Clamp the pan to the overflow bounds (a fitting axis stays centered). Scrolling
        // a long file simply stops at the top/bottom — it never steps to the prev/next
        // image. (Only a file that *fits* uses the wheel to advance, handled above.)
        self.offset.x = self.offset.x.clamp(-ox_max, ox_max);
        self.offset.y = self.offset.y.clamp(-oy_max, oy_max);

        // --- blit the image at the (clamped) pan ---
        // In pixel-perfect mode snap the origin to the device-pixel grid so every texel
        // boundary lands on a whole device pixel (a sub-pixel origin re-warps the dither).
        let mut img_tl = resp.rect.center() + self.offset - img_px / 2.0;
        if pixel_perfect {
            img_tl = egui::pos2(
                (img_tl.x * ppp).round() / ppp,
                (img_tl.y * ppp).round() / ppp,
            );
        }
        // Scanlines hug the *viewport* (the monitor), not the art, so a scrolling
        // ANSImation isn't distorted. They need one texture to overlay, so they only kick
        // in for an untiled image; huge tiled images fall back to a plain blit.
        let crt_fx = self.crt_scanline_dark > 0.0 || (self.glow && self.glow_amt > 0.0);
        if crt_fx && tt.tiles.len() == 1 {
            let t = &tt.tiles[0];
            let dst = egui::Rect::from_min_size(
                img_tl + egui::vec2(t.x as f32, t.y as f32) * scale,
                egui::vec2(t.w as f32, t.h as f32) * scale,
            );
            self.paint_crt(&painter, resp.rect, dst, t.tex.id(), ppp, scale.y * ppp);
        } else {
            for t in &tt.tiles {
                let dst = egui::Rect::from_min_size(
                    img_tl + egui::vec2(t.x as f32, t.y as f32) * scale,
                    egui::vec2(t.w as f32, t.h as f32) * scale,
                );
                painter.image(t.tex.id(), dst, full_uv, egui::Color32::WHITE);
            }
        }

        // --- navigator: scaled-down overview + the red "you are here" box ---
        if let Some(r) = nav {
            painter.rect_filled(r.expand(2.0), 2.0, egui::Color32::from_black_alpha(160));
            // A minimap built from the full-res pixels at the strip's *device* size is
            // crisp; the shared ~512px thumbnail would upscale blurry on a tall strip.
            // Fall back to that thumbnail (e.g. GIFs have no full_src), then to the raw
            // tiles — a NEAREST-downscale of those aliases dithered art into moiré, so
            // it's the last resort while the better sources warm up.
            let ctx = painter.ctx().clone();
            let dpx = [
                (r.width() * ppp).round().max(1.0) as usize,
                (r.height() * ppp).round().max(1.0) as usize,
            ];
            let mini_id = cur_path
                .as_deref()
                .and_then(|p| self.minimap_texture(&ctx, p, dpx))
                .or_else(|| {
                    cur_path
                        .as_ref()
                        .and_then(|p| self.thumb_tex.get(p))
                        .map(|t| t.id())
                });
            if let Some(id) = mini_id {
                painter.image(id, r, full_uv, egui::Color32::WHITE);
            } else {
                if let Some(p) = &cur_path {
                    self.thumbs.request(p, THUMB_PX);
                    self.want_repaint = true;
                }
                let (msx, msy) = (r.width() / sw, r.height() / sh);
                for t in &tt.tiles {
                    let dst = egui::Rect::from_min_size(
                        r.min + egui::vec2(t.x as f32 * msx, t.y as f32 * msy),
                        egui::vec2(t.w as f32 * msx, t.h as f32 * msy),
                    );
                    painter.image(t.tex.id(), dst, full_uv, egui::Color32::WHITE);
                }
            }
            painter.rect_stroke(
                r,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
                egui::StrokeKind::Inside,
            );
            // Visible region in image-normalized coords → the box on the minimap.
            let vx0 = ((resp.rect.min.x - img_tl.x) / img_px.x).clamp(0.0, 1.0);
            let vx1 = ((resp.rect.max.x - img_tl.x) / img_px.x).clamp(0.0, 1.0);
            let vy0 = ((resp.rect.min.y - img_tl.y) / img_px.y).clamp(0.0, 1.0);
            let vy1 = ((resp.rect.max.y - img_tl.y) / img_px.y).clamp(0.0, 1.0);
            let box_rect = egui::Rect::from_min_max(
                r.min + egui::vec2(vx0 * r.width(), vy0 * r.height()),
                r.min + egui::vec2(vx1 * r.width(), vy1 * r.height()),
            );
            painter.rect_stroke(
                box_rect,
                0.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(255, 64, 64)),
                egui::StrokeKind::Inside,
            );
        }

        // Raster art remembers its zoom across images (persisted). Text-mode art
        // instead opens at its Preferences zoom (`textmode_zoom`) every time, so
        // manual zoom on an ANSI is transient — the preference stays authoritative.
        if !self.fit_mode && !self.viewing_textmode {
            self.raster_zoom = self.zoom;
        }

        // Metadata OSD: fade in on a freshly opened image, hold, fade out — but hovering
        // it (even mid-fade-out) pins it open at full opacity, and clicking a field
        // (path / artist / pack / group / year) jumps there. Use `hover_pos` (not just
        // `interact_pos`) so *passive* hovering — no button held — re-pins it too, which
        // is what makes catching it during the fade-out reliable. Hit-tests against last
        // frame's rect/links; the only ways out are the [×] or letting it fade un-hovered.
        const FADE_IN: f32 = 0.35;
        const FADE_OUT: f32 = 0.7;
        if self.osd_enabled && !self.osd_dismissed {
            let pointer = ui.input(|i| i.pointer.hover_pos().or(i.pointer.interact_pos()));
            let over = self.osd_rect.zip(pointer).is_some_and(|(r, p)| r.contains(p));
            if over {
                self.osd_t = FADE_IN; // pin at full opacity while hovered
                self.want_repaint = true;
                // Hovering the OSD counts as taking control: pause the slideshow.
                if self.auto_next {
                    self.auto_paused = true;
                }
                if let Some(p) = pointer {
                    let on_close = self.osd_close.is_some_and(|r| r.contains(p));
                    let link = (!on_close)
                        .then(|| self.osd_links.iter().find(|(r, _)| r.contains(p)).map(|(_, t)| t.clone()))
                        .flatten();
                    if on_close {
                        // [×] dismisses the OSD for this image only (load_full re-shows it).
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        if ui.input(|i| i.pointer.primary_clicked()) {
                            self.osd_dismissed = true;
                            self.osd_rect = None;
                            self.osd_links.clear();
                            self.osd_close = None;
                            return advance;
                        }
                    } else if let Some(target) = link {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        if ui.input(|i| i.pointer.primary_clicked()) {
                            self.open_folder(target); // local dir or 16colo virtual path
                            return advance; // navigated away; nothing more to draw
                        }
                    }
                }
            } else {
                self.osd_t += ui.input(|i| i.stable_dt);
            }
            let total = FADE_IN + self.osd_secs + FADE_OUT;
            let t = self.osd_t;
            let alpha = if t >= total {
                0.0
            } else if t < FADE_IN {
                t / FADE_IN
            } else if t < FADE_IN + self.osd_secs {
                1.0
            } else {
                (1.0 - (t - FADE_IN - self.osd_secs) / FADE_OUT).max(0.0)
            };
            if alpha > 0.004 {
                let p = self
                    .full_tex
                    .as_ref()
                    .map(|(p, _)| p.clone())
                    .or_else(|| self.entries.get(self.selected).map(|e| e.path.clone()));
                if let Some(p) = p {
                    let (rect, links, close) =
                        self.paint_osd(&painter, resp.rect, alpha, &p, over.then_some(pointer).flatten());
                    self.osd_rect = Some(rect);
                    self.osd_links = links;
                    self.osd_close = Some(close);
                }
                self.want_repaint = true;
            } else {
                self.osd_rect = None;
                self.osd_links.clear();
                self.osd_close = None;
            }
        } else {
            self.osd_rect = None;
            self.osd_links.clear();
            self.osd_close = None;
        }
        advance
    }

    /// The metadata OSD's content for `path`: the headline `title` + hue, then a list of
    /// `(gap_before, fields)` **rows** (each field `(label, value, hue, link)`; a one-field
    /// row reads as its own line, a multi-field row flows). 16colo.rs pieces: artist line,
    /// SAUCE-comment line, then an attribute row (type/columns/lines/font/group/pack/year/
    /// ★). Local files: path line, optional comment, then type/size/dimensions/colors/
    /// created/★. `link` jumps to a local directory or a 16colo artist/group/pack/year path.
    #[allow(clippy::type_complexity)]
    fn osd_content(
        &self,
        path: &Path,
    ) -> (String, [u8; 3], Vec<(f32, Vec<(&'static str, String, [u8; 3], Option<PathBuf>)>)>) {
        use crate::sixteen::{ARTISTS, GROUPS, ROOT};
        type Field = (&'static str, String, [u8; 3], Option<PathBuf>);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let sauce = self.sauce_cache.get(path).and_then(|s| s.clone());
        let comment = sauce
            .as_ref()
            .map(|s| s.comment.trim().to_string())
            .filter(|c| !c.is_empty());
        let rating = self.read_rating(path);
        let mut rows: Vec<(f32, Vec<Field>)> = Vec::new();

        if let Some(piece) = self.colo_pieces.get(path) {
            let root = Path::new(ROOT);
            let title = sauce
                .as_ref()
                .map(|s| s.title.trim().to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| name.clone());
            // A collab piece lists several artists (`", "`-joined); give each its own
            // link so any one can be opened. One field per artist flows `a · b · c`.
            let artists: Vec<Field> = piece
                .artist
                .split(',')
                .map(|a| a.trim())
                .filter(|a| !a.is_empty())
                .map(|a| ("", a.to_string(), [150, 222, 255], Some(root.join(ARTISTS).join(a))))
                .collect();
            if !artists.is_empty() {
                rows.push((7.0, artists));
            }
            if let Some(c) = &comment {
                rows.push((9.0, vec![("", c.clone(), [205, 205, 214], None)]));
            }
            let mut attrs: Vec<Field> = Vec::new();
            // Keep the filename visible when the headline is a distinct SAUCE title.
            if title != name {
                attrs.push(("File", name.clone(), [228, 228, 234], None));
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_uppercase();
            if !ext.is_empty() {
                attrs.push(("Type", ext, [210, 236, 255], None));
            }
            if let Some(s) = &sauce {
                if s.tinfo1 > 0 {
                    attrs.push(("Columns", s.tinfo1.to_string(), [255, 236, 210], None));
                }
                if s.tinfo2 > 0 {
                    attrs.push(("Lines", s.tinfo2.to_string(), [236, 255, 210], None));
                }
                if !s.font.trim().is_empty() {
                    attrs.push(("Font", s.font.trim().to_string(), [236, 222, 255], None));
                }
            }
            if !piece.group.is_empty() {
                attrs.push(("Group", piece.group.clone(), [240, 222, 255], Some(root.join(GROUPS).join(&piece.group))));
            }
            if !piece.pack.is_empty() {
                attrs.push(("Pack", piece.pack.clone(), [218, 255, 224], Some(root.join(piece.year.to_string()).join(&piece.pack))));
            }
            attrs.push(("Year", piece.year.to_string(), [255, 250, 210], Some(root.join(piece.year.to_string()))));
            if rating > 0 {
                attrs.push(("Rating", "★".repeat(rating as usize), [255, 210, 96], None));
            }
            rows.push((11.0, attrs));
            (title, [255, 246, 232], rows)
        } else {
            if let Some(parent) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                rows.push((7.0, vec![(
                    "",
                    elide(&parent.display().to_string(), 64),
                    [205, 205, 214],
                    Some(parent.to_path_buf()),
                )]));
            }
            if let Some(c) = &comment {
                rows.push((9.0, vec![("", c.clone(), [205, 205, 214], None)]));
            }
            let mut attrs: Vec<Field> = Vec::new();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_uppercase();
            if !ext.is_empty() {
                attrs.push(("Type", ext, [210, 236, 255], None));
            }
            let entry = self.entries.iter().find(|e| e.path == path);
            if let Some(sz) = entry.map(|e| e.size).filter(|&s| s > 0) {
                attrs.push(("Size", human_size(sz), [255, 234, 222], None));
            }
            if let Some((w, h)) = self
                .full_tex
                .as_ref()
                .filter(|(fp, _)| fp == path)
                .map(|(_, tt)| (tt.size[0], tt.size[1]))
            {
                attrs.push(("Dimensions", format!("{w} × {h}"), [222, 255, 236], None));
            }
            if let Some(c) = self.img_meta.get(path).and_then(|m| m.colors) {
                attrs.push(("Colors", c.to_string(), [244, 226, 255], None));
            }
            if let Some(t) = entry.and_then(|e| e.ctime) {
                attrs.push(("Created", fmt_time(t), [255, 248, 214], None));
            }
            if rating > 0 {
                attrs.push(("Rating", "★".repeat(rating as usize), [255, 210, 96], None));
            }
            rows.push((9.0, attrs));
            (name, [255, 246, 232], rows)
        }
    }

    /// Paint the fading metadata OSD and return `(bounds, link rects, close-button rect)`.
    /// Line 1 is the name/title (larger + faux-bold); below it the artist, comment, and a
    /// flowing attribute row, each section gapped. Links open their `open_folder` target; the
    /// top-right `×` dismisses the OSD for this view. `pointer` (when hovered) underlines the
    /// link / highlights the `×` under the cursor. `osd_position` anchors the bar top / bottom
    /// (h-centered) or left / right (v-centered). Colors fade by scaling each one's alpha.
    fn paint_osd(
        &self,
        painter: &egui::Painter,
        viewport: egui::Rect,
        alpha: f32,
        path: &Path,
        pointer: Option<egui::Pos2>,
    ) -> (egui::Rect, Vec<(egui::Rect, PathBuf)>, egui::Rect) {
        let (title, title_c, rows) = self.osd_content(path);
        if title.is_empty() && rows.is_empty() {
            return (egui::Rect::NOTHING, Vec::new(), egui::Rect::NOTHING);
        }
        let av = |base: f32| (base * alpha).clamp(0.0, 255.0) as u8;
        let hue = |c: [u8; 3]| egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], av(255.0));
        let white = |a: f32| egui::Color32::from_rgba_unmultiplied(255, 255, 255, av(a));
        let label_col = egui::Color32::from_rgba_unmultiplied(150, 150, 162, av(220.0));
        let sep_col = egui::Color32::from_rgba_unmultiplied(120, 120, 132, av(170.0));
        let header_font = egui::FontId::proportional(18.0);
        let body_font = egui::FontId::proportional(14.0);
        let (pad, lv_gap, line_h, margin) = (12.0_f32, 4.0_f32, 20.0_f32, 16.0_f32);
        let max_w = (viewport.width() - 2.0 * (margin + pad)).max(160.0);

        // Line 1: the name/title (larger; faux-bold via a 0.7px double-draw).
        let header = painter.layout_no_wrap(title.clone(), header_font, hue(title_c));
        let header_h = header.size().y;
        let mut content_w = header.size().x + 0.7;

        // Each row sits below the header, separated by its own `gap`. A one-field row
        // (artist / comment / path) reads as its own line; a multi-field row flows
        // `label value · label value …`, wrapping when it would overrun `max_w`.
        let sep = painter.layout_no_wrap("   ·   ".to_string(), body_font.clone(), sep_col);
        let sep_w = sep.size().x;
        let mut placed: Vec<(egui::Pos2, std::sync::Arc<egui::Galley>)> = Vec::new();
        let mut links_rel: Vec<(egui::Rect, PathBuf)> = Vec::new();
        let mut y = header_h;
        for (gap, fields) in &rows {
            if fields.is_empty() {
                continue;
            }
            y += gap;
            let mut x = 0.0_f32;
            for (label, value, c, link) in fields {
                let lg = (!label.is_empty())
                    .then(|| painter.layout_no_wrap(label.to_string(), body_font.clone(), label_col));
                let vg = painter.layout_no_wrap(value.clone(), body_font.clone(), hue(*c));
                let group_w = lg.as_ref().map_or(0.0, |g| g.size().x + lv_gap) + vg.size().x;
                if x > 0.0 {
                    if x + sep_w + group_w > max_w {
                        y += line_h;
                        x = 0.0;
                    } else {
                        placed.push((egui::pos2(x, y), sep.clone()));
                        x += sep_w;
                    }
                }
                if let Some(lg) = lg {
                    placed.push((egui::pos2(x, y), lg.clone()));
                    x += lg.size().x + lv_gap;
                }
                if let Some(t) = link {
                    let r = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(vg.size().x, line_h));
                    links_rel.push((r, t.clone()));
                }
                placed.push((egui::pos2(x, y), vg.clone()));
                x += vg.size().x;
                content_w = content_w.max(x);
            }
            y += line_h;
        }
        let content_h = y;
        // Reserve room at the top-right for the [×] dismiss button so it never sits over
        // the title (rows below are clear of the corner regardless).
        let close_sz = 16.0_f32;
        content_w = content_w.max(header.size().x + 0.7 + close_sz + 8.0);
        // Cap to the viewport (long values are clipped rather than overflowing).
        let box_w = (content_w + 2.0 * pad).min(viewport.width() - 2.0 * margin);
        let box_h = content_h + 2.0 * pad;
        // Anchor at one of the 8 perimeter spots (3×3 grid, center unused). The flat index
        // decodes to a horizontal third (left / center / right) and vertical third (top /
        // middle / bottom); each axis resolves independently so e.g. top-left and bottom-right
        // are both reachable as a single choice.
        let (hcol, vrow) = match self.osd_position {
            0 => (0u8, 0u8), // top-left
            1 => (1, 0),     // top-center
            2 => (2, 0),     // top-right
            3 => (0, 1),     // middle-left
            4 => (2, 1),     // middle-right
            5 => (0, 2),     // bottom-left
            6 => (1, 2),     // bottom-center
            _ => (2, 2),     // bottom-right
        };
        let left = match hcol {
            0 => viewport.left() + margin,
            1 => viewport.center().x - box_w / 2.0,
            _ => viewport.right() - margin - box_w,
        };
        let top = match vrow {
            0 => viewport.top() + margin,
            1 => viewport.center().y - box_h / 2.0,
            _ => viewport.bottom() - margin - box_h,
        };
        let left = left.max(viewport.left() + margin);
        let top = top.max(viewport.top() + margin);
        let rect = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(box_w, box_h));
        painter.rect_filled(rect, 8.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, av(190.0)));
        painter.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, white(40.0)),
            egui::StrokeKind::Inside,
        );

        // Clip the text to the panel so an over-long value can't bleed past the edge.
        let tp = painter.with_clip_rect(rect.shrink(pad * 0.5));
        let origin = rect.min + egui::vec2(pad, pad);
        tp.galley(origin, header.clone(), white(255.0));
        tp.galley(origin + egui::vec2(0.7, 0.0), header, white(255.0)); // faux-bold
        for (rel, g) in &placed {
            let py = origin.y + rel.y + (line_h - g.size().y) / 2.0;
            tp.galley(egui::pos2(origin.x + rel.x, py), g.clone(), white(255.0));
        }
        // Absolute link rects (clipped to the panel) + underline the hovered one.
        let mut links = Vec::with_capacity(links_rel.len());
        for (r, t) in links_rel {
            let abs = r.translate(origin.to_vec2()).intersect(rect);
            if abs.width() < 1.0 {
                continue;
            }
            if pointer.is_some_and(|p| abs.contains(p)) {
                tp.hline(abs.x_range(), abs.bottom() - 2.0, egui::Stroke::new(1.0, white(220.0)));
            }
            links.push((abs, t));
        }

        // Top-right [×] dismiss button (drawn last, unclipped, so it's always crisp on top).
        let close_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - pad - close_sz, rect.top() + pad * 0.5),
            egui::vec2(close_sz, close_sz),
        );
        let close_hover = pointer.is_some_and(|p| close_rect.contains(p));
        if close_hover {
            painter.rect_filled(close_rect, 3.0, white(45.0));
        }
        painter.text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::FontId::proportional(16.0),
            white(if close_hover { 255.0 } else { 165.0 }),
        );
        (rect, links, close_rect)
    }

    /// Apply a click with modifiers: Ctrl toggles, Shift range-selects, plain
    /// click opens (image -> viewer, folder -> descend) and selects just that one.
    fn handle_click(&mut self, ctx: &egui::Context, idx: usize, mods: egui::Modifiers) {
        if mods.command {
            if let Some(p) = self
                .entries
                .get(idx)
                .filter(|e| !e.is_dir)
                .map(|e| e.path.clone())
            {
                if !self.selection.insert(p.clone()) {
                    self.selection.remove(&p);
                }
            }
            self.anchor = Some(idx);
        } else if mods.shift {
            let a = self.anchor.unwrap_or(idx);
            let (lo, hi) = (a.min(idx), a.max(idx));
            for i in lo..=hi {
                if let Some(p) = self
                    .entries
                    .get(i)
                    .filter(|e| !e.is_dir)
                    .map(|e| e.path.clone())
                {
                    self.selection.insert(p);
                }
            }
            self.anchor = Some(idx);
        } else {
            self.selection.clear();
            if let Some(p) = self
                .entries
                .get(idx)
                .filter(|e| !e.is_dir)
                .map(|e| e.path.clone())
            {
                self.selection.insert(p);
            }
            self.anchor = Some(idx);
            self.activate(ctx, idx);
        }
    }

    /// Write a star rating to the target(s): the current image in the single view,
    /// or every selected image (else the hovered one) in the grid.
    fn apply_rating(&mut self, stars: u8) {
        let targets: Vec<PathBuf> = match self.mode {
            Mode::Single => self
                .entries
                .get(self.selected)
                .filter(|e| !e.is_dir)
                .map(|e| vec![e.path.clone()])
                .unwrap_or_default(),
            Mode::Grid => {
                // The tile under the cursor wins when it isn't part of the selection —
                // so "point at a piece and press 3" rates *that* piece, even if an
                // earlier-opened one is still selected (the 16colo flat-listing case).
                // Hovering one of the selected tiles still rates the whole selection.
                let hovered_path = self
                    .hovered
                    .and_then(|i| self.entries.get(i))
                    .filter(|e| !e.is_dir)
                    .map(|e| e.path.clone());
                match hovered_path {
                    Some(p) if !self.selection.contains(&p) => vec![p],
                    _ => self.selection.iter().cloned().collect(),
                }
            }
        };
        let mut n = 0;
        for path in &targets {
            // Routes to xattr and/or the cross-platform sidecar as appropriate; the
            // sidecar always persists, so virtual (archive / 16colo.rs) art is ratable.
            self.set_rating(path, stars);
            if let Some(e) = self.all_entries.iter_mut().find(|e| &e.path == path) {
                e.rating = stars;
            }
            n += 1;
        }
        if n > 0 {
            self.rebuild_view();
            self.status = format!("Rated {n}: {}", stars_label(stars));
        }
    }

    /// Set one entry's rating from the context-menu "Rating ▸" submenu (files only),
    /// mirroring the hotkey path: write through to xattr/sidecar, update the model, refresh.
    fn rate_entry(&mut self, idx: usize, stars: u8) {
        let Some(path) = self
            .entries
            .get(idx)
            .filter(|e| !e.is_dir)
            .map(|e| e.path.clone())
        else {
            return;
        };
        self.set_rating(&path, stars);
        if let Some(e) = self.all_entries.iter_mut().find(|e| e.path == path) {
            e.rating = stars;
        }
        self.rebuild_view();
        self.status = format!("Rated: {}", stars_label(stars));
    }

    /// Bottom status bar: counts + total size, and the current selection.
    /// VSCode-style activity rail on the far left: icon toggles for the side docks
    /// plus quick actions. Deliberately narrow with room reserved for more buttons.
    fn ui_rail(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(6.0);
            let big = egui::FontId::proportional(19.0);
            let icon = |ui: &mut egui::Ui, glyph: &str, on: bool, tip: &str| -> bool {
                let txt = egui::RichText::new(glyph).font(big.clone());
                ui.add(egui::Button::selectable(on, txt))
                    .on_hover_text(tip)
                    .clicked()
            };
            // Glyphs chosen from what egui's bundled emoji font actually renders
            // (🗂/▤/▦ are tofu; 🗁/☰/ℹ/🔍/⚙ render). Tooltips disambiguate.
            if icon(ui, "🗁", false, "Open folder…") {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.open_folder(dir);
                }
            }
            ui.add_space(10.0);
            if icon(ui, "☰", self.show_explorer, "Explorer pane") {
                self.show_explorer = !self.show_explorer;
            }
            ui.add_space(2.0);
            if icon(ui, "ℹ", self.show_details, "Details pane") {
                self.show_details = !self.show_details;
            }
            ui.add_space(2.0);
            if icon(ui, "🎨", self.show_recolor, "Recolor pane") {
                self.show_recolor = !self.show_recolor;
            }
        });
    }

    /// Screensaver controls (status bar): a random-pack launcher + the endless Shuffle
    /// toggle. Pairs with auto ▶ + F11 for a real-scene-art screensaver.
    fn ui_shuffle_controls(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("🔀 Random pack")
            .on_hover_text(
                "Open a random 16colo.rs pack (any year). With Shuffle + auto ▶ (+ F11) it \
                 plays packs endlessly — a screensaver of real scene art.",
            )
            .clicked()
        {
            self.start_random_pack();
        }
        if ui
            .checkbox(&mut self.shuffle, "shuffle")
            .on_hover_text("At a pack's end, jump to another random 16colo.rs pack — endless")
            .changed()
            && self.shuffle
        {
            self.auto_next = true; // shuffle needs auto-advance to reach the next pack
        }
    }

    /// Bottom status row. Left: folder counts (grid) or viewer actions (single).
    /// Right (flush): the zoom readout + hint that used to live in the top toolbar.
    fn ui_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Reserve the flush-right group FIRST (right-to-left), then fill the
            // remaining space with the left group — so a long left line truncates
            // instead of colliding with the zoom readout (the old overlap bug).
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match self.mode {
                    Mode::Single => {
                        ui.weak("Z+1-0 / Ctrl+wheel zoom · drag to pan")
                            .on_hover_text(
                                "Zoom: Ctrl+wheel, or hold Z and press 1-9/0 or Z + +/- to \
                                 step. Text-mode art (and 'Snap') step in whole device \
                                 pixels per source pixel — shown as N×.",
                            );
                        ui.separator();
                        // Pixel-perfect art reads in device-pixels-per-source-pixel
                        // ("N×", always a whole step) — clearer than a fractional % on a
                        // scaled display (e.g. 308%). Free zoom still reads as a %.
                        let ppp = ui.ctx().pixels_per_point();
                        if self.viewing_textmode || self.zoom_lock {
                            let n = (self.zoom * ppp).round().max(1.0) as i32;
                            ui.label(format!("{n}×")).on_hover_text(
                                "Device pixels per source pixel — pixel-perfect whole steps",
                            );
                        } else {
                            ui.label(format!("{:.0}%", self.zoom * 100.0));
                        }
                        if ui
                            .checkbox(&mut self.zoom_lock, "Snap")
                            .on_hover_text(
                                "Lock zoom to whole device-pixel steps (N×) — \
                                 pixel-perfect nearest-neighbor scaling",
                            )
                            .changed()
                            && self.zoom_lock
                        {
                            self.zoom = snap_zoom_nearest(self.zoom);
                        }
                        // Snap the zoom so the art's full width fills the viewport (tall art
                        // then scrolls). Text-mode art already does this on open; this
                        // re-applies it after you've zoomed away.
                        if ui
                            .button("Fit W")
                            .on_hover_text("Fit the art to the viewport width")
                            .clicked()
                        {
                            self.fit_width_on_open = true;
                        }
                        // Only meaningful for text-mode art, so only shown for it.
                        if self.viewing_textmode {
                            ui.checkbox(&mut self.crt_aspect, "CRT").on_hover_text(
                                "Stretch text-mode art ≈1.2× vertically to match the \
                                 non-square pixels of a 4:3 VGA monitor (off = exact pixels)",
                            );
                            // 9-dot VGA cell width: the real DOS text cell was 9px
                            // wide for the 8px font (independent of the CRT stretch).
                            if ui
                                .checkbox(&mut self.font_9px, "9px")
                                .on_hover_text(
                                    "Render the VGA font in a 9-dot-wide cell, like real \
                                     DOS text mode — adds the inter-character gap and \
                                     joins box-draw rules (off = exact 8-pixel cells)",
                                )
                                .changed()
                            {
                                crate::decode::set_font_9px(self.font_9px);
                                let ctx = ui.ctx().clone();
                                self.redecode_full(&ctx);
                                if let Some(p) = &mut self.player {
                                    p.tex = None; // re-render the frame at the new cell width
                                }
                            }
                        }
                        // Baud picker — simulate a modem typing out the art. Shown for
                        // stream-playable art; RIP and ANSI remember their own speed.
                        // Changing it restarts the open file.
                        if self.player.is_some() {
                            ui.separator();
                            let is_rip = self.player.as_ref().is_some_and(|p| p.stream.is_rip());
                            let mut baud = if is_rip {
                                self.baud_rip
                            } else {
                                self.baud_ansi
                            };
                            let old = baud;
                            egui::ComboBox::from_id_salt("baud_pick")
                                .selected_text(format!("⚡ {}", baud.label()))
                                .show_ui(ui, |ui| {
                                    for b in Baud::ALL {
                                        ui.selectable_value(&mut baud, b, b.label());
                                    }
                                })
                                .response
                                .on_hover_text(
                                    "Simulated modem baud rate — replay the art as it would \
                                     have typed out over a dial-up connection (None = instant). \
                                     RIP and ANSI keep separate speeds.",
                                );
                            if baud != old {
                                if is_rip {
                                    self.baud_rip = baud;
                                } else {
                                    self.baud_ansi = baud;
                                }
                                if let Some(p) = &mut self.player {
                                    if baud == Baud::None {
                                        p.pos = p.len;
                                        p.playing = false;
                                    } else {
                                        p.pos = 0;
                                        p.acc = 0.0;
                                        p.playing = true;
                                    }
                                    p.tex = None;
                                }
                            }
                        }
                        // Retro-monitor effects — apply to any image in the viewer.
                        ui.separator();
                        ui.label("📺").on_hover_text("Retro monitor effects");
                        ui.add(
                            egui::Slider::new(&mut self.crt_scanline_dark, 0.0..=1.0)
                                .show_value(false)
                                .text("scanlines"),
                        )
                        .on_hover_text("Scanline darkness (0 = off)");
                        ui.checkbox(&mut self.crt_scanline_scale, "scale")
                            .on_hover_text(
                                "Scale the scanline spacing with the zoom — one line per \
                                 source-pixel row, so the lines grow as you zoom in \
                                 (off = fixed fine lines)",
                            );
                        ui.checkbox(&mut self.glow, "glow").on_hover_text(
                            "Phosphor glow — a soft bloom around bright pixels, like a \
                             late-night CRT",
                        );
                        ui.add_enabled(
                            self.glow,
                            egui::Slider::new(&mut self.glow_amt, 0.0..=1.0)
                                .show_value(false)
                                .text("amt"),
                        )
                        .on_hover_text("Phosphor glow intensity");
                        ui.checkbox(&mut self.black_bg, "black bg")
                            .on_hover_text("Fill the viewer background black (off = dark grey)");
                        // Slideshow: auto-advance through the folder, with a delay picker.
                        // When a user interaction has paused it, the label goes yellow and a
                        // click *resumes* (rather than toggling the slideshow off).
                        ui.separator();
                        let resumed = self.auto_paused;
                        let label = if self.auto_paused {
                            egui::RichText::new("auto ▶").color(egui::Color32::from_rgb(255, 210, 70))
                        } else {
                            egui::RichText::new("auto ▶")
                        };
                        let hover = if self.auto_paused {
                            "Slideshow paused — you took control. Click to resume."
                        } else {
                            "Auto-advance to the next file after it finishes drawing \
                             plus the chosen delay — flip through a whole pack hands-free"
                        };
                        if ui.checkbox(&mut self.auto_next, label).on_hover_text(hover).changed() {
                            if resumed {
                                self.auto_next = true; // clicking while paused resumes, not off
                            }
                            self.auto_paused = false;
                            self.auto_next_dwell = 0.0;
                        }
                        egui::ComboBox::from_id_salt("auto_next_secs")
                            .selected_text(format!("{}s", self.auto_next_secs))
                            .show_ui(ui, |ui| {
                                for s in [1u8, 3, 5, 10] {
                                    ui.selectable_value(
                                        &mut self.auto_next_secs,
                                        s,
                                        format!("{s}s"),
                                    );
                                }
                            })
                            .response
                            .on_hover_text("Seconds to wait before advancing");
                        ui.separator();
                        self.ui_shuffle_controls(ui);
                    }
                    Mode::Grid => {
                        if !self.entries.is_empty() {
                            ui.weak("Ctrl + wheel to zoom");
                            ui.separator();
                            // Thumbnail zoom, where 100% = DEFAULT_TILE. Three ways to
                            // set it: drag/wheel the number, click it to type a value,
                            // or pick a preset from the ▾ dropdown (egui has no native
                            // editable combobox, so a DragValue + menu covers both).
                            let min_pct = (MIN_TILE / DEFAULT_TILE * 100.0).round();
                            let max_pct = (MAX_TILE / DEFAULT_TILE * 100.0).round();
                            let mut new_size: Option<f32> = None;
                            ui.menu_button("▾", |ui| {
                                for &p in &[50.0f32, 75.0, 100.0, 150.0, 200.0, 300.0, 400.0] {
                                    if p < min_pct || p > max_pct {
                                        continue;
                                    }
                                    if ui.button(format!("{p:.0}%")).clicked() {
                                        new_size = Some(p / 100.0 * DEFAULT_TILE);
                                        ui.close();
                                    }
                                }
                            });
                            let mut pct = self.thumb_size / DEFAULT_TILE * 100.0;
                            let resp = ui
                                .add(
                                    egui::DragValue::new(&mut pct)
                                        .range(min_pct..=max_pct)
                                        .speed(0.5)
                                        .fixed_decimals(0)
                                        .suffix("%"),
                                )
                                .on_hover_text(
                                    "Thumbnail zoom — drag or wheel to scrub, click to type",
                                );
                            wheel_adjust(ui, &resp, &mut pct, 5.0, min_pct, max_pct);
                            self.thumb_size =
                                (pct / 100.0 * DEFAULT_TILE).clamp(MIN_TILE, MAX_TILE);
                            if let Some(sz) = new_size {
                                self.thumb_size = sz.clamp(MIN_TILE, MAX_TILE);
                            }
                            if ui
                                .small_button("1:1")
                                .on_hover_text("Reset thumbnail size")
                                .clicked()
                            {
                                self.thumb_size = DEFAULT_TILE;
                            }
                            ui.label("Thumbs:");
                        }
                        // Screensaver launcher — available even with no folder open.
                        ui.separator();
                        self.ui_shuffle_controls(ui);
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    // Global "working" spinner: any network request (16colo listing /
                    // download / piece open / SAUCE fetch / random pack) or a running
                    // recursive search. The status text alongside says what it's doing.
                    if self.net_busy() || self.search_running {
                        ui.add(egui::Spinner::new().size(15.0))
                            .on_hover_text("Working… (network / search in progress)");
                        self.want_repaint = true;
                    }
                    match self.mode {
                        Mode::Single => {
                            if ui.button("⬅ Grid").clicked() {
                                self.mode = Mode::Grid;
                            }
                            if ui.button("1:1").clicked() {
                                self.zoom = 1.0;
                                self.offset = egui::Vec2::ZERO;
                            }
                            if ui
                                .selectable_label(self.fit_mode, "Fit")
                                .on_hover_text("Fit to window + keep fitting new images (F)")
                                .clicked()
                            {
                                self.fit_mode = !self.fit_mode;
                                if self.fit_mode {
                                    self.fit_requested = true;
                                }
                            }
                            if ui
                                .selectable_label(self.tile_mode, "Tile")
                                .on_hover_text("Tile the image across the window (T)")
                                .clicked()
                            {
                                self.tile_mode = !self.tile_mode;
                            }
                            // (Reduce-to-N-colors lives in the Recolor pane, next to
                            // the palette/adjustment controls it belongs with.)
                        }
                        Mode::Grid => {
                            let imgs = self.entries.iter().filter(|e| !e.is_dir).count();
                            let dirs = self.entries.iter().filter(|e| e.is_dir).count();
                            let total: u64 = self
                                .entries
                                .iter()
                                .filter(|e| !e.is_dir)
                                .map(|e| e.size)
                                .sum();
                            let mut s =
                                format!("{imgs} images · {dirs} folders · {}", human_size(total));
                            if !self.selection.is_empty() {
                                let sel: u64 = self
                                    .all_entries
                                    .iter()
                                    .filter(|e| self.selection.contains(&e.path))
                                    .map(|e| e.size)
                                    .sum();
                                s.push_str(&format!(
                                    "   ·   {} selected · {}",
                                    self.selection.len(),
                                    human_size(sel)
                                ));
                            }
                            if !self.status.is_empty() {
                                s.push_str("   ·   ");
                                s.push_str(&self.status); // transient op feedback
                            }
                            ui.add(egui::Label::new(s).truncate());
                        }
                    }
                });
            });
        });
    }

    /// Top menu bar. The nested menu closures only *read* `self` and stash a
    /// deferred `MenuAction`, applied afterward — avoiding nested `&mut self`.
    fn ui_menubar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut action: Option<MenuAction> = None;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open folder…").clicked() {
                    action = Some(MenuAction::Open);
                    ui.close();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    action = Some(MenuAction::Quit);
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                let has_sel = !self.selection.is_empty() || self.hovered.is_some();
                let can_paste = self.clipboard.is_some();
                if ui
                    .add_enabled(!self.undo_stack.is_empty(), egui::Button::new("↩ Undo"))
                    .clicked()
                {
                    action = Some(MenuAction::Undo);
                    ui.close();
                }
                ui.separator();
                if ui.add_enabled(has_sel, egui::Button::new("Copy")).clicked() {
                    action = Some(MenuAction::File(FileAction::Copy));
                    ui.close();
                }
                if ui.add_enabled(has_sel, egui::Button::new("Cut")).clicked() {
                    action = Some(MenuAction::File(FileAction::Cut));
                    ui.close();
                }
                if ui
                    .add_enabled(can_paste, egui::Button::new("Paste"))
                    .clicked()
                {
                    action = Some(MenuAction::File(FileAction::Paste));
                    ui.close();
                }
                ui.separator();
                if ui.button("New folder").clicked() {
                    action = Some(MenuAction::File(FileAction::NewFolder));
                    ui.close();
                }
                if ui
                    .add_enabled(has_sel, egui::Button::new("Rename…"))
                    .clicked()
                {
                    action = Some(MenuAction::File(FileAction::Rename));
                    ui.close();
                }
                if ui
                    .add_enabled(has_sel, egui::Button::new("Move to trash"))
                    .clicked()
                {
                    action = Some(MenuAction::File(FileAction::Delete));
                    ui.close();
                }
                ui.separator();
                if ui
                    .button("Find images… (Ctrl+F)")
                    .on_hover_text("Recursive search by name, type, dimensions, SAUCE")
                    .clicked()
                {
                    action = Some(MenuAction::Search);
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                if ui
                    .selectable_label(self.table_view, "Table view")
                    .on_hover_text("Show the current folder as a sortable table")
                    .clicked()
                {
                    action = Some(MenuAction::ToggleTable);
                    ui.close();
                }
                ui.separator();
                if ui
                    .selectable_label(self.show_explorer, "Explorer pane")
                    .clicked()
                {
                    action = Some(MenuAction::ToggleExplorer);
                    ui.close();
                }
                if ui
                    .selectable_label(self.show_details, "Details pane")
                    .clicked()
                {
                    action = Some(MenuAction::ToggleDetails);
                    ui.close();
                }
                if ui
                    .selectable_label(self.show_recolor, "Recolor pane")
                    .clicked()
                {
                    action = Some(MenuAction::ToggleRecolor);
                    ui.close();
                }
                if ui.button("Reset thumbnail size").clicked() {
                    action = Some(MenuAction::ResetThumb);
                    ui.close();
                }
                ui.separator();
                if ui.button("Preferences…").clicked() {
                    action = Some(MenuAction::Prefs);
                    ui.close();
                }
            });
            ui.menu_button("Sort", |ui| {
                for k in SortKey::COMMON {
                    if ui.selectable_label(self.sort_key == k, k.label()).clicked() {
                        action = Some(MenuAction::Sort(k));
                        ui.close();
                    }
                }
                ui.separator();
                if ui.selectable_label(self.sort_desc, "Descending").clicked() {
                    action = Some(MenuAction::ToggleDesc);
                    ui.close();
                }
                if ui
                    .selectable_label(self.dirs_first, "Directories first")
                    .clicked()
                {
                    action = Some(MenuAction::ToggleDirsFirst);
                    ui.close();
                }
            });
            ui.menu_button("Go", |ui| {
                if ui.button("⬆ Up").clicked() {
                    action = Some(MenuAction::Up);
                    ui.close();
                }
                if ui.button("🏠 Home").clicked() {
                    action = Some(MenuAction::Home);
                    ui.close();
                }
                if !self.favorites.is_empty() {
                    ui.separator();
                    for fav in &self.favorites {
                        if ui.button(format!("📁 {}", short_name(fav))).clicked() {
                            action = Some(MenuAction::Nav(fav.clone()));
                            ui.close();
                        }
                    }
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard shortcuts").clicked() {
                    action = Some(MenuAction::Hotkeys);
                    ui.close();
                }
            });
        });
        if let Some(a) = action {
            self.do_menu(ctx, a);
        }
    }

    fn do_menu(&mut self, ctx: &egui::Context, a: MenuAction) {
        match a {
            MenuAction::Open => {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.open_folder(d);
                }
            }
            MenuAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            MenuAction::ToggleExplorer => self.show_explorer = !self.show_explorer,
            MenuAction::ToggleDetails => self.show_details = !self.show_details,
            MenuAction::ToggleRecolor => self.show_recolor = !self.show_recolor,
            MenuAction::ToggleTable => self.table_view = !self.table_view,
            MenuAction::Up => {
                // Compute the parent in *display* space so "up" from an archive root
                // lands in the archive's real parent folder, not its temp dir.
                if let Some(folder) = self.folder.clone() {
                    let disp = self.to_display(&folder);
                    if let Some(parent) = disp.parent() {
                        let real = self.real_path(parent);
                        self.open_folder(real);
                    }
                }
            }
            MenuAction::Home => {
                if let Some(h) = home_dir() {
                    self.open_folder(h);
                }
            }
            MenuAction::Nav(p) => self.open_folder(p),
            MenuAction::Sort(k) => {
                self.sort_key = k;
                self.rebuild_view();
            }
            MenuAction::ToggleDesc => {
                self.sort_desc = !self.sort_desc;
                self.rebuild_view();
            }
            MenuAction::ToggleDirsFirst => {
                self.dirs_first = !self.dirs_first;
                self.rebuild_view();
            }
            MenuAction::ResetThumb => self.thumb_size = DEFAULT_TILE,
            MenuAction::Hotkeys => self.show_hotkeys = true,
            MenuAction::Prefs => self.show_prefs = true,
            MenuAction::Search => {
                self.show_search = true;
                self.focus_adv_search = true;
            }
            MenuAction::File(fa) => self.do_file_action(fa),
            MenuAction::Undo => self.undo(),
        }
    }

    /// Left dock: Places (Home + favorites) and a real, expandable folder tree
    /// (disclosure triangles) for the current folder, with a name filter on top.
    fn ui_explorer(&mut self, ui: &mut egui::Ui) {
        let mut nav: Option<PathBuf> = None;
        let mut recall: Option<usize> = None; // smart filter to run
        let mut remove_filter: Option<usize> = None;
        // Tabs: Places | Folders (one at a time, to save vertical room).
        ui.horizontal(|ui| {
            if ui
                .selectable_label(self.explorer_tab == 0, "Places")
                .clicked()
            {
                self.explorer_tab = 0;
            }
            if ui
                .selectable_label(self.explorer_tab == 1, "Folders")
                .clicked()
            {
                self.explorer_tab = 1;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                if self.explorer_tab == 0 {
                    // Sub-tabs split favorites/pins by kind: Local folders vs 16colo.rs.
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(self.places_tab == 0, "Local")
                            .clicked()
                        {
                            self.places_tab = 0;
                        }
                        if ui
                            .selectable_label(self.places_tab == 1, "16colo.rs")
                            .clicked()
                        {
                            self.places_tab = 1;
                        }
                    });
                    if self.places_tab == 0 {
                        // Local: Home + on-disk favorites + smart filters (local searches).
                        if ui.button("🏠 Home").clicked() {
                            nav = home_dir();
                        }
                        if let Some(p) =
                            self.favorites_buttons(ui, "📁", |p| !crate::sixteen::is_remote(p))
                        {
                            nav = Some(p);
                        }
                        // Smart filters: saved searches. Click recalls + runs; right-click
                        // removes. (Deferred out of the closure — can't borrow self twice.)
                        if !self.saved_filters.is_empty() {
                            ui.separator();
                            ui.weak("Smart filters");
                            for (i, (name, _)) in self.saved_filters.iter().enumerate() {
                                let r = ui.button(format!("🔍 {name}")).on_hover_text(
                                    "Run this saved search · right-click to remove",
                                );
                                if r.clicked() {
                                    recall = Some(i);
                                }
                                r.context_menu(|ui| {
                                    if ui.button("Remove").clicked() {
                                        remove_filter = Some(i);
                                        ui.close();
                                    }
                                });
                            }
                        }
                    } else {
                        // 16colo.rs: the browse entry + pinned artists/groups/searches/packs.
                        if ui
                            .button("🌐 16colo.rs")
                            .on_hover_text("Browse the 16colo.rs ANSI art archive online")
                            .clicked()
                        {
                            nav = Some(PathBuf::from(crate::sixteen::ROOT));
                        }
                        if let Some(p) =
                            self.favorites_buttons(ui, "🌐", crate::sixteen::is_remote)
                        {
                            nav = Some(p);
                        }
                    }
                } else {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.explorer_filter)
                            .hint_text("🔍 filter folders")
                            .desired_width(f32::INFINITY),
                    );
                    if let Some(folder) = self.folder.clone() {
                        if let Some(parent) = folder.parent() {
                            if ui.button("⬆ ..").on_hover_text("Parent folder").clicked() {
                                nav = Some(parent.to_path_buf());
                            }
                        }
                        let filt = self.explorer_filter.to_lowercase();
                        for child in subdirs_sorted(&folder) {
                            if !filt.is_empty()
                                && !short_name(&child).to_lowercase().contains(&filt)
                            {
                                continue;
                            }
                            folder_tree_node(ui, &child, &mut nav);
                        }
                    }
                }
            });
        if let Some(i) = remove_filter {
            if i < self.saved_filters.len() {
                self.saved_filters.remove(i);
            }
        }
        if let Some(i) = recall {
            self.recall_filter(i);
        } else if let Some(p) = nav {
            self.open_folder(p);
        }
    }
}

impl eframe::App for PixelView {
    // eframe 0.34.3 made `ui` the required method (`update` is now deprecated).
    // We're handed a root `Ui` instead of a `Context`, so panels mount via
    // `show_inside(ui, ..)` and we clone the context off the `Ui` for the rest.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Track the live zoom factor (changed by Ctrl +/-) so `save` can persist it.
        self.ui_zoom = ctx.zoom_factor();
        self.want_repaint = false;

        // F11 — immersive mode: hide every bar/dock, drop the window decorations, and go
        // OS-fullscreen, showing only the art. Bars reveal when the mouse reaches the
        // matching screen edge; the cursor auto-hides after a moment of stillness.
        // True while the user is typing into *any* text field — incl. the 16colo.rs
        // search box, the '/' filter, and the advanced-search fields, which the explicit
        // flags below don't cover. Without this, a Backspace to fix a typo in the search
        // box fired ParentDir (jumping the artists list back to Years), and an 'r' in a
        // query triggered the random-pack hotkey. `wants_keyboard_input` reflects focus
        // (held across frames), so it's reliable even though this runs before the panels.
        let typing = self.path_edit.is_some()
            || self.renaming.is_some()
            || self.rebinding.is_some()
            || ctx.egui_wants_keyboard_input();
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            self.immersive = !self.immersive;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.immersive));
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(!self.immersive));
        }
        // Which chrome edges are visible: all of them normally; in immersive mode only the
        // edge the mouse is hovering (within EDGE px of the window border).
        let mut hide_cursor = false;
        let (show_top, show_bottom, show_left, show_right) = if self.immersive {
            const EDGE: f32 = 48.0;
            const CURSOR_HIDE_SECS: f32 = 1.5;
            let win = ctx.content_rect();
            let (p, moved, dt) = ctx.input(|i| {
                let moved = i.pointer.delta() != egui::Vec2::ZERO
                    || i.smooth_scroll_delta != egui::Vec2::ZERO
                    || i.pointer.any_down();
                (i.pointer.latest_pos(), moved, i.stable_dt)
            });
            // Auto-hide the cursor after a still moment; any movement shows it instantly.
            if moved {
                self.idle_t = 0.0;
            } else {
                self.idle_t += dt;
            }
            hide_cursor = self.idle_t > CURSOR_HIDE_SECS;
            self.want_repaint = true; // keep polling the pointer for edge reveal + idle
            (
                p.is_some_and(|p| p.y - win.min.y < EDGE),
                p.is_some_and(|p| win.max.y - p.y < EDGE),
                p.is_some_and(|p| p.x - win.min.x < EDGE),
                p.is_some_and(|p| win.max.x - p.x < EDGE),
            )
        } else {
            self.idle_t = 0.0;
            (true, true, true, true)
        };

        // R — jump to a new random 16colo.rs pack (skip a dud). Not while typing/searching.
        if !typing
            && self.search.is_none()
            && !self.show_search
            && ctx.input(|i| i.key_pressed(egui::Key::R))
        {
            self.start_random_pack();
        }

        // Apply any finished 16colo.rs fetch/download (keeps repainting while pending).
        self.poll_remote();
        self.poll_search();
        self.poll_random();
        self.poll_colo_pieces();
        self.poll_colo_open(&ctx);
        self.poll_colo_save();
        self.poll_colo_sauce();
        // Screensaver: once a (random) pack has finished downloading + mounting, open its
        // first art file. Both async ops idle ⇒ the listing has settled.
        if self.pending_autoplay && self.random_rx.is_none() && self.remote_rx.is_none() {
            self.pending_autoplay = false;
            if let Some(idx) = self.entries.iter().position(|e| !e.is_dir && !e.is_archive) {
                self.activate(&ctx, idx);
            } else if self.shuffle {
                self.start_random_pack(); // empty/failed pack → try the next one
            }
        }

        // Upload finished thumbnails (must happen on the UI thread).
        let mut new_meta = false;
        for r in self.thumbs.drain() {
            let color = egui::ColorImage::from_rgba_unmultiplied([r.width, r.height], &r.rgba);
            // A thumb the worker *downscaled* (area-averaged) is shown LINEAR so its
            // faithful tones aren't re-aliased by a second nearest pass at tile size;
            // a source-res sprite stays NEAREST so it upscales crisply (pixel art).
            let downscaled = r.src_w as usize > r.width || r.src_h as usize > r.height;
            let opts = if downscaled {
                egui::TextureOptions::LINEAR
            } else {
                egui::TextureOptions::NEAREST
            };
            let tex = ctx.load_texture(r.path.to_string_lossy(), color, opts);
            self.img_meta.insert(
                r.path.clone(),
                ImgMeta {
                    w: r.src_w,
                    h: r.src_h,
                    colors: r.colors,
                },
            );
            self.palettes.insert(r.path.clone(), r.palette);
            // Keep the thumb's CPU pixels so the grid can be recolored without a
            // re-decode (only matters for pixel-art-sized thumbnails).
            self.thumb_rgba
                .insert(r.path.clone(), (r.width, r.height, r.rgba));
            self.thumb_tex.insert(r.path, tex);
            self.want_repaint = true;
            new_meta = true;
        }
        // Upload finished 16colo.rs piece thumbnails (the pre-rendered `tn` PNGs fetched
        // over HTTP). They're keyed by the piece's virtual path, so the grid/table find
        // them in `thumb_tex` like any thumb. Always LINEAR — they're rendered previews,
        // not pixel-art sprites, so a nearest pass at tile size would shimmer.
        for r in self.colo_thumbs.drain() {
            let color = egui::ColorImage::from_rgba_unmultiplied([r.width, r.height], &r.rgba);
            let tex = ctx.load_texture(
                r.path.to_string_lossy(),
                color,
                egui::TextureOptions::LINEAR,
            );
            self.thumb_tex.insert(r.path, tex);
            self.want_repaint = true;
        }
        // Sorting by Colors needs every entry's count, which the grid would only
        // request for *visible* tiles — so eagerly decode the rest (request() dedupes,
        // so this is cheap after the first pass), and re-sort as counts stream in.
        if self.sort_key == SortKey::Colors {
            for e in &self.all_entries {
                if !e.is_dir && !self.img_meta.contains_key(&e.path) {
                    self.thumbs.request(&e.path, THUMB_PX);
                }
            }
            if new_meta {
                self.rebuild_view();
            }
        }

        // Hotkey rebinding: the next key press becomes the binding (Esc cancels).
        if let Some(a) = self.rebinding {
            let pressed = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => Some(*key),
                    _ => None,
                })
            });
            if let Some(k) = pressed {
                if k != egui::Key::Escape {
                    self.keymap.insert(a, k);
                }
                self.rebinding = None;
            }
        }

        // Keyboard: ratings (1-5 / 0) + rebindable nav (defaults: Esc->grid,
        // Backspace->parent). Suppressed while typing into any text field (path /
        // search / rename) or capturing a rebind — else a typed digit rates an image.
        if !typing
            && self.search.is_none()
            && !self.show_search
        // typing into any field (path, rename, search box, criteria) must not rate
        // images or trigger nav keys — `typing` now also covers focused text fields.
        {
            let back_key = self.key_for(Action::BackToGrid);
            let parent_key = self.key_for(Action::ParentDir);
            let view_key = self.key_for(Action::ToggleView);
            let (rate, esc, back, z_held, zoom_set, zoom_step, toggle_view) = ctx.input(|i| {
                use egui::Key::*;
                let z_held = i.key_down(Z); // DRAW-style zoom chord modifier
                let rate = [
                    (Num0, 0u8),
                    (Num1, 1),
                    (Num2, 2),
                    (Num3, 3),
                    (Num4, 4),
                    (Num5, 5),
                ]
                .into_iter()
                .find(|&(k, _)| i.key_pressed(k))
                .map(|(_, s)| s);
                // Z + 1..9 → 100..900%, Z + 0 → 1000%.
                let zoom_set = [
                    (Num1, 1.0f32),
                    (Num2, 2.0),
                    (Num3, 3.0),
                    (Num4, 4.0),
                    (Num5, 5.0),
                    (Num6, 6.0),
                    (Num7, 7.0),
                    (Num8, 8.0),
                    (Num9, 9.0),
                    (Num0, 10.0),
                ]
                .into_iter()
                .find(|&(k, _)| i.key_pressed(k))
                .map(|(_, v)| v);
                let zoom_step = i32::from(i.key_pressed(Plus) || i.key_pressed(Equals))
                    - i32::from(i.key_pressed(Minus));
                (
                    rate,
                    i.key_pressed(back_key),
                    i.key_pressed(parent_key),
                    z_held,
                    zoom_set,
                    zoom_step,
                    i.key_pressed(view_key),
                )
            });
            // Toggle grid/table only in the browse view (in the single view the same
            // default key 'T' is the tile-mode toggle — they never overlap by mode).
            if toggle_view && self.mode == Mode::Grid {
                self.table_view = !self.table_view;
            }
            // Hold Z in the viewer to set (1..0) or step (±) the image zoom — DRAW
            // style; this suppresses the 1-5/0 star ratings while Z is down.
            if z_held {
                if self.mode == Mode::Single {
                    // Pixel-perfect art reads Z+N as N *device* pixels per source pixel
                    // (so Z+3 → "3×" exactly, matching the readout); free zoom keeps
                    // N = logical 100·N%. Steps use the same device ladder.
                    let ppp = ctx.pixels_per_point();
                    let pixel_perfect = self.viewing_textmode || self.zoom_lock;
                    if let Some(z) = zoom_set {
                        self.zoom = if pixel_perfect { z / ppp } else { z };
                        self.offset = egui::Vec2::ZERO;
                    }
                    for _ in 0..zoom_step.unsigned_abs() {
                        if pixel_perfect {
                            self.step_device_zoom(ppp, zoom_step > 0);
                        } else {
                            self.zoom = snap_zoom(self.zoom, zoom_step > 0);
                        }
                    }
                }
            } else if let Some(stars) = rate {
                self.apply_rating(stars);
            }
            if esc && self.mode == Mode::Single {
                self.mode = Mode::Grid;
            }
            if back {
                // Parent in display space (see MenuAction::Up) so leaving an archive
                // returns to its real parent folder.
                if let Some(folder) = self.folder.clone() {
                    let disp = self.to_display(&folder);
                    if let Some(parent) = disp.parent() {
                        let real = self.real_path(parent);
                        self.open_folder(real);
                    }
                }
            }
        }

        // Grid: Home/End jump+scroll to first/last; mouse back/forward = folder
        // history. Single view: mouse back/forward = previous/next image.
        let (home, end) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
            )
        });
        let (mouse_back, mouse_fwd) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1),
                i.pointer.button_pressed(egui::PointerButton::Extra2),
            )
        });
        if self.mode == Mode::Grid
            && !typing
            && self.search.is_none()
            && !self.entries.is_empty()
        {
            if home {
                self.select_index(0);
            }
            if end {
                self.select_index(self.entries.len() - 1);
            }
        }
        // '/' opens the grid filename filter (vim-style).
        if self.mode == Mode::Grid
            && self.path_edit.is_none()
            && self.rebinding.is_none()
            && self.search.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Slash))
        {
            self.search = Some(String::new());
            self.focus_search = true;
        }
        // Ctrl+F opens the advanced recursive search; Esc closes it.
        if self.path_edit.is_none()
            && self.rebinding.is_none()
            && ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F))
        {
            self.show_search = true;
            self.focus_adv_search = true;
        }
        if self.show_search
            && self.path_edit.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.close_search();
        }
        // Esc closes the '/' filename filter no matter which widget holds focus. The
        // in-field check (TextEdit lost_focus) only fired when the box itself was
        // focused, so pressing Esc after clicking away did nothing. This runs before
        // the searchbar panel renders, so it covers both focused and unfocused cases.
        if self.search.is_some()
            && self.path_edit.is_none()
            && self.renaming.is_none()
            && self.rebinding.is_none()
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.search = None;
            self.rebuild_view();
        }
        if mouse_back {
            match self.mode {
                Mode::Grid => self.go_history(true),
                Mode::Single => self.step_image(&ctx, false),
            }
        }
        if mouse_fwd {
            match self.mode {
                Mode::Grid => self.go_history(false),
                Mode::Single => self.step_image(&ctx, true),
            }
        }

        // File operations (Trash + Undo). Grid only, and never while typing into
        // the path bar, the search filter, the rename box, or capturing a rebind.
        if self.mode == Mode::Grid
            && self.path_edit.is_none()
            && self.search.is_none()
            && self.renaming.is_none()
            && self.rebinding.is_none()
        {
            let fa = ctx.input(|i| {
                use egui::Key::*;
                let c = i.modifiers.command; // Ctrl on Linux/Win, Cmd on macOS
                if c && i.key_pressed(C) {
                    Some(FileAction::Copy)
                } else if c && i.key_pressed(X) {
                    Some(FileAction::Cut)
                } else if c && i.key_pressed(V) {
                    Some(FileAction::Paste)
                } else if c && i.key_pressed(N) {
                    Some(FileAction::NewFolder)
                } else if i.key_pressed(F2) {
                    Some(FileAction::Rename)
                } else if i.key_pressed(Delete) {
                    Some(FileAction::Delete)
                } else {
                    None
                }
            });
            // Ctrl+Z is its own thing (no FileAction variant).
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
                self.undo();
            } else if let Some(a) = fa {
                self.do_file_action(a);
            }
        }

        if show_top {
            egui::Panel::top("menubar").show_inside(ui, |ui| self.ui_menubar(&ctx, ui));
        }

        // VSCode-style activity rail, far left, full height below the menu bar.
        if show_left {
            egui::Panel::left("rail")
                .exact_size(46.0)
                .resizable(false)
                .show_inside(ui, |ui| self.ui_rail(ui));
        }

        // Favorites bar (top), then the path/breadcrumb bar directly under it.
        // Favorites stay visible even before a folder is open, for quick jumps.
        let in_colo = self
            .folder
            .as_deref()
            .is_some_and(crate::sixteen::is_remote)
            || self
                .archive_mount
                .as_ref()
                .is_some_and(|m| crate::sixteen::is_remote(&m.archive));
        if show_top {
            egui::Panel::top("favorites").show_inside(ui, |ui| self.ui_favorites(ui));
            if self.folder.is_some() {
                egui::Panel::top("crumbs").show_inside(ui, |ui| self.ui_breadcrumbs(ui));
            }
            // 16colo.rs quick-jump nav bar — shown while browsing the archive (also inside
            // a mounted pack, whose display path is the virtual one).
            if in_colo {
                egui::Panel::top("colo_nav").show_inside(ui, |ui| self.ui_colo_nav(ui));
            }
            if self.search.is_some() {
                egui::Panel::top("searchbar").show_inside(ui, |ui| self.ui_searchbar(ui));
            }
            if self.show_search {
                egui::Panel::top("advsearch").show_inside(ui, |ui| self.ui_search(ui));
            }
        }

        // Bottom: status row at the very bottom, the sort/filter row just above it
        // (status is mounted first so it sits below the sort bar).
        if show_bottom {
            egui::Panel::bottom("status").show_inside(ui, |ui| self.ui_status(ui));
            if self.folder.is_some() {
                egui::Panel::bottom("sortbar").show_inside(ui, |ui| self.ui_sortbar(ui));
            }
        }

        // Left dock: Details on top (takes the bulk of the height), Explorer
        // (Places/Folders) a compact panel under it. Mounting Explorer as a *bottom*
        // panel with a modest default makes Details the larger pane by default.
        if show_left && (self.show_details || self.show_explorer) {
            egui::Panel::left("leftdock")
                .resizable(true)
                .default_size(300.0)
                .show_inside(ui, |ui| {
                    if self.show_details && self.show_explorer {
                        egui::Panel::bottom("ld_explorer")
                            .resizable(true)
                            .default_size(240.0)
                            .show_inside(ui, |ui| self.ui_explorer(ui));
                        egui::CentralPanel::default().show_inside(ui, |ui| self.ui_details(ui));
                    } else if self.show_details {
                        self.ui_details(ui);
                    } else {
                        self.ui_explorer(ui);
                    }
                });
        }
        // Right dock: Recolor.
        if show_right && self.show_recolor {
            egui::Panel::right("recolor")
                .resizable(true)
                .default_size(340.0)
                .show_inside(ui, |ui| self.ui_recolor(ui));
        }

        // In immersive mode the central panel fills pure black (no grey frame/margin
        // around the art); normally it uses the themed central-panel frame.
        let central = egui::CentralPanel::default();
        let central = if self.immersive {
            central.frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
        } else {
            central
        };
        central.show_inside(ui, |ui| match self.mode {
            // The table is an alternate renderer for the browse mode (not a third
            // `Mode`), so selection/ratings/keyboard-nav all keep working unchanged.
            Mode::Grid if self.table_view => self.ui_table(&ctx, ui),
            Mode::Grid => self.ui_grid(&ctx, ui),
            Mode::Single => self.ui_single(&ctx, ui),
        });

        if self.show_hotkeys {
            let mut open = true;
            egui::Window::new("Keyboard shortcuts")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    egui::Grid::new("hotkeys_grid")
                        .striped(true)
                        .spacing([18.0, 6.0])
                        .show(ui, |ui| {
                            // Rebindable nav actions reflect the current keymap.
                            for a in Action::ALL {
                                ui.strong(self.key_for(a).symbol_or_name());
                                ui.label(a.label());
                                ui.end_row();
                            }
                            for (k, d) in HOTKEYS {
                                ui.strong(*k);
                                ui.label(*d);
                                ui.end_row();
                            }
                        });
                });
            self.show_hotkeys = open;
        }

        if self.show_prefs {
            let mut open = true;
            egui::Window::new("Preferences")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| {
                    let mut theme = self.theme;
                    let mut gap = self.grid_gap;
                    ui.label("Theme");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut theme, 0, "Dark");
                        ui.selectable_value(&mut theme, 1, "Light");
                    });
                    ui.add_space(8.0);
                    ui.label("Grid spacing (horizontal)");
                    let resp = ui.add(egui::Slider::new(&mut gap, 0.0..=40.0).suffix(" pt"));
                    wheel_adjust(ui, &resp, &mut gap, 1.0, 0.0f32, 40.0f32);
                    let mut gap_y = self.grid_gap_y;
                    ui.label("Grid spacing (vertical)");
                    let resp = ui.add(egui::Slider::new(&mut gap_y, 0.0..=80.0).suffix(" pt"));
                    wheel_adjust(ui, &resp, &mut gap_y, 1.0, 0.0f32, 80.0f32);
                    if theme != self.theme {
                        self.theme = theme;
                        ctx.set_visuals(if theme == 1 {
                            egui::Visuals::light()
                        } else {
                            egui::Visuals::dark()
                        });
                    }
                    self.grid_gap = gap;
                    self.grid_gap_y = gap_y;

                    ui.add_space(10.0);
                    // Text-mode/scene art renders at a tiny native 8×16 px per cell, so
                    // it opens at this zoom instead of an unreadable 1:1. Measured in
                    // *device* pixels per source pixel (N×) — the pixel-perfect unit the
                    // viewer shows; applied to every ANSI/scene file as it loads.
                    ui.label("Text-mode (ANSI/scene) zoom");
                    let mut tz = self.textmode_zoom.round() as i32;
                    egui::ComboBox::from_id_salt("textmode_zoom")
                        .selected_text(format!("{tz}×"))
                        .show_ui(ui, |ui| {
                            for n in [1, 2, 3, 4, 5, 6, 8] {
                                ui.selectable_value(&mut tz, n, format!("{n}×"));
                            }
                        });
                    if (tz as f32 - self.textmode_zoom).abs() > f32::EPSILON {
                        self.textmode_zoom = tz as f32;
                        // Reflect the change immediately on an already-open ANSI.
                        if self.viewing_textmode && !self.fit_mode {
                            self.zoom = self.textmode_zoom / ctx.pixels_per_point();
                        }
                    }

                    ui.add_space(10.0);
                    ui.label("Show under thumbnails");
                    let mut fields = self.caption_fields;
                    ui.horizontal_wrapped(|ui| {
                        for &(mask, label) in CAPTION_FIELDS {
                            let mut on = fields & mask != 0;
                            if ui.checkbox(&mut on, label).changed() {
                                if on {
                                    fields |= mask;
                                } else {
                                    fields &= !mask;
                                }
                            }
                        }
                    });
                    self.caption_fields = fields;

                    ui.add_space(10.0);
                    ui.label("Table columns (file view)");
                    let mut tcols = self.table_columns;
                    ui.horizontal_wrapped(|ui| {
                        for &(mask, label) in TABLE_COLUMNS {
                            let mut on = tcols & mask != 0;
                            if ui.checkbox(&mut on, label).changed() {
                                if on {
                                    tcols |= mask;
                                } else {
                                    tcols &= !mask;
                                }
                            }
                        }
                    });
                    self.table_columns = tcols;

                    ui.add_space(10.0);
                    ui.label("Hotkeys");
                    let mut new_rebind: Option<Option<Action>> = None;
                    egui::Grid::new("prefs_keys")
                        .num_columns(3)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            for a in Action::ALL {
                                ui.label(a.label());
                                let cur = self
                                    .keymap
                                    .get(&a)
                                    .copied()
                                    .unwrap_or_else(|| a.default_key());
                                ui.strong(cur.symbol_or_name());
                                let waiting = self.rebinding == Some(a);
                                let btn = if waiting {
                                    "press a key… (Esc cancels)"
                                } else {
                                    "Rebind"
                                };
                                if ui.button(btn).clicked() {
                                    new_rebind = Some(if waiting { None } else { Some(a) });
                                }
                                ui.end_row();
                            }
                        });
                    if let Some(r) = new_rebind {
                        self.rebinding = r;
                    }

                    ui.add_space(10.0);
                    ui.label("Viewer info OSD");
                    ui.checkbox(&mut self.osd_enabled, "Show metadata overlay on open")
                        .on_hover_text("A fading panel with the piece's details");
                    ui.add_enabled_ui(self.osd_enabled, |ui| {
                        ui.label("Position");
                        // A spatial 3×3 picker (center unused) — the button's place in the
                        // grid is where the OSD lands, so a corner is one click.
                        egui::Grid::new("osd_pos_grid")
                            .spacing([4.0, 4.0])
                            .show(ui, |ui| {
                                ui.selectable_value(&mut self.osd_position, 0, "Top L");
                                ui.selectable_value(&mut self.osd_position, 1, "Top");
                                ui.selectable_value(&mut self.osd_position, 2, "Top R");
                                ui.end_row();
                                ui.selectable_value(&mut self.osd_position, 3, "Left");
                                ui.label(""); // center: unused
                                ui.selectable_value(&mut self.osd_position, 4, "Right");
                                ui.end_row();
                                ui.selectable_value(&mut self.osd_position, 5, "Bot L");
                                ui.selectable_value(&mut self.osd_position, 6, "Bot");
                                ui.selectable_value(&mut self.osd_position, 7, "Bot R");
                                ui.end_row();
                            });
                        ui.add(
                            egui::Slider::new(&mut self.osd_secs, 0.5..=15.0)
                                .suffix(" s")
                                .text("Hold"),
                        )
                        .on_hover_text("How long it stays before fading out");
                    });

                    ui.add_space(10.0);
                    ui.label("16colo.rs cache");
                    let (bytes, count) = crate::cache::stats();
                    ui.horizontal(|ui| {
                        ui.weak(format!(
                            "{} · {count} items",
                            human_size(bytes.max(0) as u64)
                        ));
                        if ui
                            .button("Clear cache")
                            .on_hover_text(
                                "Delete all cached 16colo.rs JSON, thumbnails, files and \
                                 pack zips (they'll re-download on demand)",
                            )
                            .clicked()
                        {
                            crate::cache::clear();
                            self.status = "Cache cleared".into();
                        }
                    });
                });
            self.show_prefs = open;
        }

        // Rename dialog (F2 / Edit menu / right-click / new-folder). The buffer is
        // owned by `self.renaming`; we lift it out for editing and decide its fate.
        if let Some((path, mut name)) = self.renaming.take() {
            let mut open = true;
            let mut commit = false;
            let mut cancel = false;
            egui::Window::new("Rename")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(&ctx, |ui| {
                    ui.label(format!("Rename “{}”", short_name(&path)));
                    let resp = ui.add(egui::TextEdit::singleline(&mut name).desired_width(280.0));
                    if self.focus_rename {
                        resp.request_focus();
                        self.focus_rename = false;
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        commit = true;
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Rename").clicked() {
                            commit = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            if commit {
                self.renaming = Some((path, name.clone())); // apply_rename take()s `old`
                self.apply_rename(&name);
            } else if cancel || !open {
                self.renaming = None;
            } else {
                self.renaming = Some((path, name)); // keep editing next frame
            }
        }

        // Auto-hide the mouse cursor in immersive mode after a still moment (set last so
        // it overrides any widget's cursor request for this frame).
        if hide_cursor {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        }

        if self.want_repaint {
            ctx.request_repaint();
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, Self::ZOOM_KEY, &self.ui_zoom);
        eframe::set_value(storage, Self::THUMB_KEY, &self.thumb_size);
        eframe::set_value(storage, Self::FAV_KEY, &self.favorites);
        let fav_colors: Vec<(PathBuf, [u8; 3])> =
            self.fav_colors.iter().map(|(p, &c)| (p.clone(), c)).collect();
        eframe::set_value(storage, Self::FAV_COLORS_KEY, &fav_colors);
        let filters: Vec<Vec<String>> = self
            .saved_filters
            .iter()
            .map(|(name, spec)| {
                let mut row = vec![name.clone()];
                row.extend(spec.record());
                row
            })
            .collect();
        eframe::set_value(storage, Self::SAVED_FILTERS_KEY, &filters);
        // Remember where we were. Save the *display* path, not `self.folder`: inside an
        // archive / downloaded 16colo pack, `self.folder` is a temp dir that's gone next
        // launch, whereas the display path (`pack.zip/…`, `<16colo.rs>/year/pack`) is
        // stable and re-openable.
        let last_folder = self.folder.clone().map(|f| self.to_display(&f));
        eframe::set_value(storage, Self::FOLDER_KEY, &last_folder);
        eframe::set_value(storage, Self::SORT_KEY, &self.sort_key.to_u8());
        eframe::set_value(storage, Self::SORT_DESC, &self.sort_desc);
        eframe::set_value(storage, Self::TABLE_VIEW_KEY, &self.table_view);
        eframe::set_value(storage, Self::TABLE_COLUMNS_KEY, &self.table_columns);
        eframe::set_value(storage, Self::COLO_COLUMNS_KEY, &self.colo_columns);
        let widths: Vec<(u8, f32)> = self.col_widths.iter().map(|(&k, &w)| (k, w)).collect();
        eframe::set_value(storage, Self::COL_WIDTHS_KEY, &widths);
        eframe::set_value(storage, Self::TABLE_ORDER_KEY, &self.table_order);
        eframe::set_value(storage, Self::COLO_ORDER_KEY, &self.colo_order);
        eframe::set_value(storage, Self::DIRS_FIRST, &self.dirs_first);
        eframe::set_value(storage, Self::MIN_RATING, &self.min_rating);
        eframe::set_value(storage, Self::EXPLORER_KEY, &self.show_explorer);
        eframe::set_value(storage, Self::DETAILS_KEY, &self.show_details);
        eframe::set_value(storage, Self::RECOLOR_KEY, &self.show_recolor);
        eframe::set_value(storage, Self::RECOLOR_GRID_KEY, &self.recolor_grid);
        eframe::set_value(storage, Self::FIT_MODE_KEY, &self.fit_mode);
        eframe::set_value(storage, Self::TEXTMODE_ZOOM_KEY, &self.textmode_zoom);
        eframe::set_value(storage, Self::CRT_ASPECT_KEY, &self.crt_aspect);
        eframe::set_value(storage, Self::FONT_9PX_KEY, &self.font_9px);
        eframe::set_value(storage, Self::BAUD_ANSI_KEY, &self.baud_ansi.to_u8());
        eframe::set_value(storage, Self::BAUD_RIP_KEY, &self.baud_rip.to_u8());
        eframe::set_value(
            storage,
            Self::CRT_SCANLINE_DARK_KEY,
            &self.crt_scanline_dark,
        );
        eframe::set_value(
            storage,
            Self::CRT_SCANLINE_SCALE_KEY,
            &self.crt_scanline_scale,
        );
        eframe::set_value(storage, Self::BLACK_BG_KEY, &self.black_bg);
        eframe::set_value(storage, Self::OSD_ENABLED_KEY, &self.osd_enabled);
        eframe::set_value(storage, Self::OSD_POSITION_KEY, &self.osd_position);
        eframe::set_value(storage, Self::OSD_SECS_KEY, &self.osd_secs);
        eframe::set_value(storage, Self::AUTO_NEXT_KEY, &self.auto_next);
        eframe::set_value(storage, Self::AUTO_NEXT_SECS_KEY, &self.auto_next_secs);
        eframe::set_value(storage, Self::GLOW_KEY, &self.glow);
        eframe::set_value(storage, Self::GLOW_AMT_KEY, &self.glow_amt);
        eframe::set_value(storage, Self::SHUFFLE_KEY, &self.shuffle);
        self.ratings.save(); // belt-and-suspenders; set_rating already flushes

        // (ratings persist to their own JSON sidecar, not eframe storage)
        eframe::set_value(storage, Self::ADJUST_KEY, &self.adjust.to_array());
        eframe::set_value(storage, Self::ADJUST_ORDER_KEY, &self.adjust.order_to_u8());
        eframe::set_value(storage, Self::IMG_ZOOM_KEY, &self.raster_zoom);
        eframe::set_value(storage, Self::ZOOM_LOCK_KEY, &self.zoom_lock);
        eframe::set_value(storage, Self::THEME_KEY, &self.theme);
        eframe::set_value(storage, Self::GAP_KEY, &self.grid_gap);
        eframe::set_value(storage, Self::GAP_Y_KEY, &self.grid_gap_y);
        eframe::set_value(storage, Self::CAPTION_KEY, &self.caption_fields);
        eframe::set_value(storage, Self::QUANT_ON_KEY, &self.quantize_on);
        eframe::set_value(storage, Self::QUANT_N_KEY, &self.quantize_n);
        eframe::set_value(storage, Self::DITHER_METHOD_KEY, &self.dither_method);
        eframe::set_value(storage, Self::DITHER_AMOUNT_KEY, &self.dither_amount);
        eframe::set_value(storage, Self::DITHER_CUSTOM_KEY, &self.dither_custom);
        eframe::set_value(storage, Self::DITHER_CUSTOM_N_KEY, &self.dither_custom_n);
        eframe::set_value(storage, Self::BALANCE_COLOR_KEY, &self.balance_color);
        eframe::set_value(storage, Self::BALANCE_STRENGTH_KEY, &self.balance_strength);
        eframe::set_value(storage, Self::PALETTE_DIR_KEY, &self.palette_dir);
        eframe::set_value(storage, Self::PALETTE_FAV_KEY, &self.palette_favorites);
        eframe::set_value(storage, Self::SELECTED_PAL_KEY, &self.selected_palette);
        let km: Vec<(u8, String)> = self
            .keymap
            .iter()
            .map(|(a, k)| (a.to_u8(), k.name().to_string()))
            .collect();
        eframe::set_value(storage, Self::KEYMAP_KEY, &km);
    }

    // Persist only our own keys (above), not all of egui's memory.
    fn persist_egui_memory(&self) -> bool {
        false
    }
}

/// File/dir name as a display string (lossy), or the whole path if it has none.
fn short_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Recursively copy a file or directory tree from `src` to the exact path `dst`.
/// Written by hand (rather than via a crate) so paste-into-the-same-folder can
/// choose a non-colliding `dst` like "sprite (copy).png".
fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)?.flatten() {
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst).map(|_| ())
    }
}

/// Move `src` to `dst`: a rename when they share a filesystem, else copy+delete.
fn move_path(src: &Path, dst: &Path) -> std::io::Result<()> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_recursive(src, dst)?;
    if src.is_dir() {
        std::fs::remove_dir_all(src)
    } else {
        std::fs::remove_file(src)
    }
}

/// Serialize a palette to GIMP `.gpl` text (importable by GIMP, Aseprite, etc.).
fn to_gpl(name: &str, palette: &[[u8; 4]]) -> String {
    let mut s = String::from("GIMP Palette\n");
    s.push_str(&format!("Name: {name}\n"));
    s.push_str("Columns: 16\n#\n");
    for c in palette {
        s.push_str(&format!(
            "{:3} {:3} {:3}\t#{:02X}{:02X}{:02X}\n",
            c[0], c[1], c[2], c[0], c[1], c[2]
        ));
    }
    s
}

/// The `.gpl` palette files in `dir`, sorted by name.
/// Virtual `PathBuf`s for the palettes baked into the binary (see
/// `palettes_builtin`). They live under a sentinel root and never exist on disk;
/// `load_gpl` resolves their contents from the embedded table.
fn builtin_palette_paths() -> Vec<PathBuf> {
    let root = Path::new(crate::palettes_builtin::BUILTIN_ROOT);
    crate::palettes_builtin::BUILTIN_PALETTES
        .iter()
        .map(|(name, _)| root.join(name))
        .collect()
}

/// The bundled ANSI32 palette as 32 RGB colors, parsed once — the swatch set offered
/// when color-tagging a favorite/pin (Places).
fn ansi32_palette() -> &'static [[u8; 3]] {
    static PAL: std::sync::OnceLock<Vec<[u8; 3]>> = std::sync::OnceLock::new();
    PAL.get_or_init(|| {
        crate::palettes_builtin::BUILTIN_PALETTES
            .iter()
            .find(|(n, _)| n.starts_with("ANSI32"))
            .map(|(_, c)| {
                crate::thumb::parse_gpl(c)
                    .iter()
                    .map(|p| [p[0], p[1], p[2]])
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Black or white text, whichever reads better on a favorite's color fill `c`.
fn contrast_text(c: [u8; 3]) -> egui::Color32 {
    let lum = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    if lum > 140.0 {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    }
}

/// The embedded contents of a built-in palette, matched by file name. None if
/// `path` isn't one of the baked-in palettes.
fn builtin_palette_contents(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    crate::palettes_builtin::BUILTIN_PALETTES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
}

/// Built-in palettes first, then any `*.gpl` in `dir` whose name isn't already a
/// built-in — so the bundled set always shows and a user dir can extend it.
fn all_palettes(dir: &Path) -> Vec<PathBuf> {
    let mut v = builtin_palette_paths();
    let builtin_names: std::collections::HashSet<_> = v
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_owned()))
        .collect();
    for p in scan_palettes(dir) {
        if p.file_name()
            .map(|n| !builtin_names.contains(n))
            .unwrap_or(true)
        {
            v.push(p);
        }
    }
    v
}

fn scan_palettes(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|x| x.to_str())
                        .is_some_and(|x| x.eq_ignore_ascii_case("gpl"))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The individual adjustment operations, in their fixed *declaration* order (which
/// is also the index used by `as u8` for persistence). The order they're *applied*
/// in is user-controlled — see `Adjust::order`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum OpKind {
    Brightness,
    Contrast,
    Gamma,
    Shadows,
    Highlights,
    Posterize,
    Hue,
    Saturation,
    Vibrance,
    Pixelate,
    Sharpen,
    /// The palette / color-rematch step. Has no slider value — it just marks *where*
    /// in the pipeline the recolor (selected palette / Reduce / custom) is applied.
    Palette,
    /// Invert (negate). A value op: 0 = off, 1 = full negative, in between blends.
    Invert,
    /// Per-channel R/G/B offset ("color balance"). A marker op — the color and
    /// strength live in the Color-balance section, not on a single slider.
    ColorBalance,
    /// The dither step. A marker op — method/amount/custom matrix live in the
    /// Dither section. Ordered methods bias here; the Palette op snaps later.
    Dither,
}

impl OpKind {
    /// Every op, in declaration order. `ALL[i] as u8 == i`. New ops are appended
    /// so persisted order indices (0..=11) stay valid across upgrades.
    const ALL: [OpKind; 15] = [
        OpKind::Brightness,
        OpKind::Contrast,
        OpKind::Gamma,
        OpKind::Shadows,
        OpKind::Highlights,
        OpKind::Posterize,
        OpKind::Hue,
        OpKind::Saturation,
        OpKind::Vibrance,
        OpKind::Pixelate,
        OpKind::Sharpen,
        OpKind::Palette,
        OpKind::Invert,
        OpKind::ColorBalance,
        OpKind::Dither,
    ];

    fn from_u8(b: u8) -> Option<OpKind> {
        Self::ALL.get(b as usize).copied()
    }

    /// Marker ops have no slider — their controls live in a dedicated section and
    /// their pipeline work is done by `apply_pipeline`, not `apply_op`.
    fn is_marker(self) -> bool {
        matches!(
            self,
            OpKind::Palette | OpKind::ColorBalance | OpKind::Dither
        )
    }

    /// `(label, min, max, default, step)` for this op's slider. `step > 0` makes the
    /// slider integer-stepped (Posterize / Pixelate). Palette has no slider; only its
    /// label is meaningful.
    fn spec(self) -> (&'static str, f32, f32, f32, f64) {
        match self {
            OpKind::Brightness => ("Brightness", -1.0, 1.0, 0.0, 0.0),
            OpKind::Contrast => ("Contrast", -1.0, 1.0, 0.0, 0.0),
            OpKind::Gamma => ("Gamma (levels)", 0.1, 3.0, 1.0, 0.0),
            OpKind::Shadows => ("Shadows", -1.0, 1.0, 0.0, 0.0),
            OpKind::Highlights => ("Highlights", -1.0, 1.0, 0.0, 0.0),
            OpKind::Posterize => ("Posterize", 0.0, 16.0, 0.0, 1.0),
            OpKind::Hue => ("Hue", -1.0, 1.0, 0.0, 0.0),
            OpKind::Saturation => ("Saturation", -1.0, 1.0, 0.0, 0.0),
            OpKind::Vibrance => ("Vibrance", -1.0, 1.0, 0.0, 0.0),
            OpKind::Pixelate => ("Pixelate", 0.0, 32.0, 0.0, 1.0),
            OpKind::Sharpen => ("Sharpen", 0.0, 1.0, 0.0, 0.0),
            OpKind::Invert => ("Invert", 0.0, 1.0, 0.0, 0.0),
            OpKind::Palette => ("Palette", 0.0, 0.0, 0.0, 0.0),
            OpKind::ColorBalance => ("Color balance", 0.0, 0.0, 0.0, 0.0),
            OpKind::Dither => ("Dither", 0.0, 0.0, 0.0, 0.0),
        }
    }
}

/// Per-image adjustments, applied BEFORE the palette map. Tone ops (a per-channel
/// LUT) and color/spatial ops in one struct. Each op is applied in `order` (a
/// user-rearrangeable permutation of `OpKind::ALL`), so e.g. pixelate-then-posterize
/// can be swapped to posterize-then-pixelate. Ranges:
/// brightness/contrast/shadows/highlights -1..1 (0 = none), gamma 0.1..3 (1 =
/// none), posterize = levels per channel (0/1 = off, 2..16 = banding),
/// hue -1..1 (→ ±180°, 0 = none), saturation -1..1 (-1 = grayscale, 0 = none,
/// +1 = ×2), vibrance -1..1 (0 = none; saturation push weighted toward muted
/// colors, protecting already-vivid ones and leaving neutrals alone), pixelate =
/// block size in px (0/1 = off, 2.. = mosaic), sharpen 0..1 (unsharp amount, 0 = off).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Adjust {
    brightness: f32,
    contrast: f32,
    gamma: f32,
    shadows: f32,
    highlights: f32,
    posterize: f32,
    hue: f32,
    saturation: f32,
    vibrance: f32,
    pixelate: f32,
    sharpen: f32,
    /// Invert amount: 0 = off, 1 = full negative, in between blends original↔negative.
    invert: f32,
    /// The order ops are applied in — a permutation of `OpKind::ALL`, and also the
    /// order the sliders are shown in. Not part of `is_identity` (it only matters
    /// when some op is active) but it *is* part of `key()` so the cache invalidates.
    order: [OpKind; 15],
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            gamma: 1.0,
            shadows: 0.0,
            highlights: 0.0,
            posterize: 0.0,
            hue: 0.0,
            saturation: 0.0,
            vibrance: 0.0,
            pixelate: 0.0,
            sharpen: 0.0,
            invert: 0.0,
            order: Self::DEFAULT_ORDER,
        }
    }
}

impl Adjust {
    /// The default pipeline order: pixelate → tone curve → invert → color →
    /// balance → sharpen → dither → palette rematch. Dither sits right before the
    /// palette snap so the default behaves like the historical dithered reduce.
    const DEFAULT_ORDER: [OpKind; 15] = [
        OpKind::Pixelate,
        OpKind::Brightness,
        OpKind::Contrast,
        OpKind::Gamma,
        OpKind::Shadows,
        OpKind::Highlights,
        OpKind::Posterize,
        OpKind::Invert,
        OpKind::Hue,
        OpKind::Saturation,
        OpKind::Vibrance,
        OpKind::ColorBalance,
        OpKind::Sharpen,
        OpKind::Dither,
        OpKind::Palette,
    ];

    fn is_identity(&self) -> bool {
        self.brightness == 0.0
            && self.contrast == 0.0
            && (self.gamma - 1.0).abs() < 1e-4
            && self.shadows == 0.0
            && self.highlights == 0.0
            && self.posterize < 2.0
            && self.hue == 0.0
            && self.saturation == 0.0
            && self.vibrance == 0.0
            && self.pixelate < 2.0
            && self.sharpen <= 0.0
            && self.invert <= 0.0
    }
    fn key(&self) -> String {
        let order: String = self
            .order
            .iter()
            .map(|o| (*o as u8 + b'a') as char)
            .collect();
        format!(
            "a{:.2},{:.2},{:.2},{:.2},{:.2},{:.0},{:.3},{:.3},{:.3},{:.0},{:.3},{:.3};o{order}",
            self.brightness,
            self.contrast,
            self.gamma,
            self.shadows,
            self.highlights,
            self.posterize,
            self.hue,
            self.saturation,
            self.vibrance,
            self.pixelate,
            self.sharpen,
            self.invert
        )
    }
    /// The apply order as op indices, for persistence.
    fn order_to_u8(&self) -> [u8; 15] {
        self.order.map(|o| o as u8)
    }
    /// Adopt a persisted order, ignoring unknown/duplicate entries and appending any
    /// ops the saved order was missing — so it survives corruption, a shorter legacy
    /// order, or a future op being added. Falls back to the default if it can't form
    /// a full set.
    fn with_order(mut self, arr: &[u8]) -> Self {
        let current = self.order;
        let mut ops: Vec<OpKind> = Vec::with_capacity(current.len());
        for &b in arr {
            if let Some(op) = OpKind::from_u8(b) {
                if !ops.contains(&op) {
                    ops.push(op);
                }
            }
        }
        // Append any ops the saved order didn't cover (corruption, a legacy order
        // from before an op existed), keeping the current order's relative placement
        // — so all-garbage input reduces to the current (default) order, never a
        // partial set.
        for op in current {
            if !ops.contains(&op) {
                ops.push(op);
            }
        }
        if let Ok(order) = <[OpKind; 15]>::try_from(ops) {
            self.order = order;
        }
        self
    }
    /// A mutable handle to the value field a given op drives (for sliders/reset).
    /// Only valid for value ops — never called for `OpKind::Palette` (no slider).
    fn field_mut(&mut self, op: OpKind) -> &mut f32 {
        match op {
            OpKind::Palette | OpKind::ColorBalance | OpKind::Dither => {
                unreachable!("marker ops have no slider value")
            }
            OpKind::Invert => &mut self.invert,
            OpKind::Brightness => &mut self.brightness,
            OpKind::Contrast => &mut self.contrast,
            OpKind::Gamma => &mut self.gamma,
            OpKind::Shadows => &mut self.shadows,
            OpKind::Highlights => &mut self.highlights,
            OpKind::Posterize => &mut self.posterize,
            OpKind::Hue => &mut self.hue,
            OpKind::Saturation => &mut self.saturation,
            OpKind::Vibrance => &mut self.vibrance,
            OpKind::Pixelate => &mut self.pixelate,
            OpKind::Sharpen => &mut self.sharpen,
        }
    }
    fn to_array(self) -> [f32; 12] {
        [
            self.brightness,
            self.contrast,
            self.gamma,
            self.shadows,
            self.highlights,
            self.posterize,
            self.hue,
            self.saturation,
            self.vibrance,
            self.pixelate,
            self.sharpen,
            self.invert,
        ]
    }
    fn from_array(a: [f32; 12]) -> Self {
        Self {
            brightness: a[0],
            contrast: a[1],
            gamma: a[2],
            shadows: a[3],
            highlights: a[4],
            posterize: a[5],
            hue: a[6],
            saturation: a[7],
            vibrance: a[8],
            pixelate: a[9],
            sharpen: a[10],
            invert: a[11],
            order: Self::DEFAULT_ORDER,
        }
    }
    /// Load a pre-invert (11-field) persisted array, defaulting `invert` to off.
    fn from_array11(a: [f32; 11]) -> Self {
        let mut full = [0.0f32; 12];
        full[..11].copy_from_slice(&a);
        Self::from_array(full)
    }
}

/// The non-`Adjust` inputs the marker ops need: dither settings (method/amount and
/// the custom matrix), the per-channel color-balance offset, and the palette to snap
/// to (the Palette op, and error-diffusion dither, both use it).
struct PipeAux<'a> {
    dither_method: u8,
    dither_amount: f32,
    dither_custom: &'a [u32],
    dither_n: usize,
    balance: [i16; 3],
    palette: Option<&'a [[u8; 4]]>,
}

/// Run the full pipeline in `a.order`. Each value op is its own pass (point ops via a
/// 256-entry LUT, color ops via per-pixel HSL, spatial ops over the `w`×`h` grid);
/// the *marker* ops do their work here — `Dither` lays down its pattern, `ColorBalance`
/// shifts channels, and `Palette` snaps to the active palette — each *wherever the
/// user dragged it*. Transparent pixels are untouched; inactive ops are skipped, so
/// order only matters among active ops.
fn apply_pipeline(rgba: &mut [u8], w: usize, h: usize, a: &Adjust, aux: &PipeAux) {
    for op in a.order {
        match op {
            OpKind::Palette => {
                if let Some(p) = aux.palette {
                    crate::thumb::remap_to_palette(rgba, p);
                }
            }
            OpKind::Dither => crate::thumb::dither_pass(
                rgba,
                w,
                h,
                aux.dither_method,
                aux.dither_amount,
                aux.dither_custom,
                aux.dither_n,
                aux.palette,
            ),
            OpKind::ColorBalance => color_balance(rgba, aux.balance),
            other => apply_op(rgba, w, h, other, a),
        }
    }
}

/// Per-channel additive offset ("color balance"): `out = in + off`, clamped. A zero
/// offset (neutral color or zero strength) is a no-op. Transparent pixels untouched.
fn color_balance(rgba: &mut [u8], off: [i16; 3]) {
    if off == [0, 0, 0] {
        return;
    }
    for px in rgba.chunks_exact_mut(4) {
        if px[3] == 0 {
            continue;
        }
        px[0] = (px[0] as i16 + off[0]).clamp(0, 255) as u8;
        px[1] = (px[1] as i16 + off[1]).clamp(0, 255) as u8;
        px[2] = (px[2] as i16 + off[2]).clamp(0, 255) as u8;
    }
}

/// Parse a hex color (`#1A2B3C`, `1a2b3c`, or the 3-digit shorthand `#abc`) into
/// `[r, g, b]`. A leading `#` and surrounding whitespace are tolerated; any other
/// length or a non-hex digit yields None — so a half-typed field is simply ignored.
fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    let nibble = |b: u8| (b as char).to_digit(16).map(|v| v as u8);
    match h.len() {
        6 => {
            let n = u32::from_str_radix(h, 16).ok()?;
            Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
        }
        3 => {
            // Each digit doubles up: 'a' -> 0xAA, '3' -> 0x33.
            let b = h.as_bytes();
            let dup = |v: u8| (v << 4) | v;
            Some([dup(nibble(b[0])?), dup(nibble(b[1])?), dup(nibble(b[2])?)])
        }
        _ => None,
    }
}

/// Apply just the adjustments (no palette/dither/balance) — every marker slot is a
/// no-op. Test-only helper; production paths use [`apply_pipeline`] with a real aux.
#[cfg(test)]
fn adjust_pixels(rgba: &mut [u8], w: usize, h: usize, a: &Adjust) {
    if a.is_identity() {
        return;
    }
    apply_pipeline(
        rgba,
        w,
        h,
        a,
        &PipeAux {
            dither_method: 0,
            dither_amount: 0.0,
            dither_custom: &[],
            dither_n: 0,
            balance: [0, 0, 0],
            palette: None,
        },
    );
}

/// Apply a single adjustment op (its value read from `a`). Inactive ops do nothing.
fn apply_op(rgba: &mut [u8], w: usize, h: usize, op: OpKind, a: &Adjust) {
    match op {
        OpKind::Pixelate => {
            // Mosaic: replace each bs×bs block with its mean (alpha untouched, so
            // the sprite's silhouette stays crisp).
            let bs = a.pixelate.round() as usize;
            if bs >= 2 && w > 0 && h > 0 {
                pixelate_blocks(rgba, w, h, bs);
            }
        }
        OpKind::Brightness => {
            if a.brightness != 0.0 {
                let b = a.brightness * 255.0;
                apply_lut(rgba, &std::array::from_fn(|i| clamp_u8(i as f32 + b)));
            }
        }
        OpKind::Contrast => {
            if a.contrast != 0.0 {
                let c = (a.contrast * 255.0).clamp(-255.0, 255.0);
                let cf = (259.0 * (c + 255.0)) / (255.0 * (259.0 - c));
                apply_lut(
                    rgba,
                    &std::array::from_fn(|i| clamp_u8((i as f32 - 128.0) * cf + 128.0)),
                );
            }
        }
        OpKind::Gamma => {
            if (a.gamma - 1.0).abs() >= 1e-4 {
                let inv = 1.0 / a.gamma.max(0.01);
                apply_lut(
                    rgba,
                    &std::array::from_fn(|i| clamp_u8((i as f32 / 255.0).powf(inv) * 255.0)),
                );
            }
        }
        OpKind::Shadows => {
            if a.shadows != 0.0 {
                let s = a.shadows;
                apply_lut(
                    rgba,
                    &std::array::from_fn(|i| {
                        let n = i as f32 / 255.0;
                        clamp_u8((n + s * (1.0 - n) * (1.0 - n)) * 255.0)
                    }),
                );
            }
        }
        OpKind::Highlights => {
            if a.highlights != 0.0 {
                let hl = a.highlights;
                apply_lut(
                    rgba,
                    &std::array::from_fn(|i| {
                        let n = i as f32 / 255.0;
                        clamp_u8((n + hl * n * n) * 255.0)
                    }),
                );
            }
        }
        OpKind::Posterize => {
            let levels = a.posterize.round();
            if levels >= 2.0 {
                apply_lut(
                    rgba,
                    &std::array::from_fn(|i| {
                        let n = i as f32 / 255.0;
                        clamp_u8(((n * (levels - 1.0)).round() / (levels - 1.0)) * 255.0)
                    }),
                );
            }
        }
        OpKind::Hue => {
            if a.hue != 0.0 {
                let deg = a.hue * 180.0;
                apply_hsl(rgba, |h, s, l| ((h + deg).rem_euclid(360.0), s, l));
            }
        }
        OpKind::Saturation => {
            if a.saturation != 0.0 {
                let m = (1.0 + a.saturation).max(0.0); // -1 → 0 (gray), +1 → 2
                apply_hsl(rgba, |h, s, l| (h, (s * m).clamp(0.0, 1.0), l));
            }
        }
        OpKind::Vibrance => {
            // Saturation push weighted by how *un*saturated a pixel already is — so
            // muted colors move most, vivid ones are protected, and neutrals (s = 0)
            // stay neutral instead of picking up a stray hue.
            if a.vibrance != 0.0 {
                let v = a.vibrance;
                apply_hsl(rgba, |h, s, l| {
                    (h, (s * (1.0 + v * (1.0 - s))).clamp(0.0, 1.0), l)
                });
            }
        }
        OpKind::Sharpen => {
            if a.sharpen > 0.0 && w > 0 && h > 0 {
                sharpen_image(rgba, w, h, a.sharpen);
            }
        }
        OpKind::Invert => {
            // Blend toward the photographic negative: 0 = original, 1 = full invert.
            if a.invert > 0.0 {
                let t = a.invert.clamp(0.0, 1.0);
                apply_lut(
                    rgba,
                    &std::array::from_fn(|i| {
                        let n = i as f32;
                        clamp_u8(n * (1.0 - t) + (255.0 - n) * t)
                    }),
                );
            }
        }
        // Marker ops — handled directly by apply_pipeline, never here.
        OpKind::Palette | OpKind::ColorBalance | OpKind::Dither => {}
    }
}

/// Round + clamp a float to a `u8` channel value.
fn clamp_u8(v: f32) -> u8 {
    v.clamp(0.0, 255.0).round() as u8
}

/// Apply a per-channel 256-entry LUT to every opaque pixel.
fn apply_lut(rgba: &mut [u8], lut: &[u8; 256]) {
    for px in rgba.chunks_exact_mut(4) {
        if px[3] == 0 {
            continue;
        }
        px[0] = lut[px[0] as usize];
        px[1] = lut[px[1] as usize];
        px[2] = lut[px[2] as usize];
    }
}

/// Round-trip every opaque pixel through HSL, transforming `(h, s, l)` with `f`.
fn apply_hsl(rgba: &mut [u8], f: impl Fn(f32, f32, f32) -> (f32, f32, f32)) {
    for px in rgba.chunks_exact_mut(4) {
        if px[3] == 0 {
            continue;
        }
        let (h, s, l) = rgb_to_hsl(px[0], px[1], px[2]);
        let (h, s, l) = f(h, s, l);
        let (r, g, b) = hsl_to_rgb(h, s, l);
        px[0] = r;
        px[1] = g;
        px[2] = b;
    }
}

/// Mosaic: average the RGB of each `bs`×`bs` block over its opaque pixels and
/// write that mean back to them. Alpha is preserved, so edges stay hard.
fn pixelate_blocks(rgba: &mut [u8], w: usize, h: usize, bs: usize) {
    let mut by = 0;
    while by < h {
        let mut bx = 0;
        while bx < w {
            let (y1, x1) = ((by + bs).min(h), (bx + bs).min(w));
            let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in by..y1 {
                for x in bx..x1 {
                    let i = (y * w + x) * 4;
                    if rgba[i + 3] == 0 {
                        continue;
                    }
                    sr += rgba[i] as u32;
                    sg += rgba[i + 1] as u32;
                    sb += rgba[i + 2] as u32;
                    n += 1;
                }
            }
            if let (Some(ar), Some(ag), Some(ab)) =
                (sr.checked_div(n), sg.checked_div(n), sb.checked_div(n))
            {
                let (ar, ag, ab) = (ar as u8, ag as u8, ab as u8);
                for y in by..y1 {
                    for x in bx..x1 {
                        let i = (y * w + x) * 4;
                        if rgba[i + 3] == 0 {
                            continue;
                        }
                        rgba[i] = ar;
                        rgba[i + 1] = ag;
                        rgba[i + 2] = ab;
                    }
                }
            }
            bx += bs;
        }
        by += bs;
    }
}

/// Unsharp mask with a 3×3 cross kernel (center 1+4·amount, 4-neighbours
/// −amount). Reads a snapshot so the pass isn't self-referential; transparent
/// neighbours (and image edges) clamp to the center value, so silhouettes don't
/// bleed dark halos.
fn sharpen_image(rgba: &mut [u8], w: usize, h: usize, amount: f32) {
    let src = rgba.to_vec();
    let center = 1.0 + 4.0 * amount;
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if src[i + 3] == 0 {
                continue;
            }
            for c in 0..3 {
                let cv = src[i + c] as f32;
                let nb = |nx: i32, ny: i32| -> f32 {
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        return cv;
                    }
                    let ni = (ny as usize * w + nx as usize) * 4;
                    if src[ni + 3] == 0 {
                        cv
                    } else {
                        src[ni + c] as f32
                    }
                };
                let (xi, yi) = (x as i32, y as i32);
                let v = cv * center
                    - amount * (nb(xi - 1, yi) + nb(xi + 1, yi) + nb(xi, yi - 1) + nb(xi, yi + 1));
                rgba[i + c] = v.clamp(0.0, 255.0).round() as u8;
            }
        }
    }
}

/// RGB (0..255) → HSL, h in 0..360, s/l in 0..1.
fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l); // gray: hue undefined, saturation 0
    }
    let s = (d / (1.0 - (2.0 * l - 1.0).abs())).clamp(0.0, 1.0);
    let h = if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h.rem_euclid(360.0), s, l)
}

/// HSL (h 0..360, s/l 0..1) → RGB (0..255).
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    (to(r1), to(g1), to(b1))
}

/// FNV-1a hash of a palette's bytes — a stable cache key for generated/edited
/// palettes (which have no file path).
fn palette_hash(pal: &[[u8; 4]]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for c in pal {
        for &b in c {
            h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// A wall-clock-seeded index in `0..n` (for the Random palette pick). Rust's
/// clock is fine here — this isn't the workflow JS sandbox.
fn random_index(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as usize)
        .unwrap_or(0)
        % n
}

/// A short display label for a palette file: its stem (e.g. "EGA (16)").
fn palette_label(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// While `resp` is hovered, let the mouse wheel nudge `*v` by `step` per notch
/// (×10 with Shift), clamped to `lo..=hi`. Vertical scroll is consumed while the
/// pointer is over the control, so an enclosing ScrollArea doesn't also move.
fn wheel_adjust<N: egui::emath::Numeric>(
    ui: &egui::Ui,
    resp: &egui::Response,
    v: &mut N,
    step: f64,
    lo: N,
    hi: N,
) {
    // contains_pointer() (not hovered()) — inside a ScrollArea, hovered() can read
    // false while the scroll gesture is being processed.
    if !resp.contains_pointer() {
        return;
    }
    let (notches, shift) = ui.input(|i| {
        // Discrete wheel events (not the smoothed delta) → one step per notch.
        let dy: f32 = i
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                _ => None,
            })
            .sum();
        (dy, i.modifiers.shift)
    });
    ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0); // consume the scroll
    if notches != 0.0 {
        let mult = if shift { 10.0 } else { 1.0 };
        let nv =
            (v.to_f64() + notches.signum() as f64 * step * mult).clamp(lo.to_f64(), hi.to_f64());
        *v = N::from_f64(nv);
    }
}

/// A middle-button click inside `resp`'s rect resets `*v` to `def`. Reads the
/// raw `PointerButton` event and hit-tests its own `pos` against the rect — NOT
/// `resp.middle_clicked()` (a `Slider` senses *drag*, never reporting a click)
/// and NOT `hovered()`/`contains_pointer()` (on the press frame, egui's hover
/// state lags — the slider grabs the pointer, so the hover test reads false the
/// very frame the event arrives). We match BOTH press and release of the middle
/// button: the slider drag-tracks the held button and re-commits the pointer
/// position on the *release* frame, so firing on release too (this runs after
/// the slider in the same frame) guarantees the reset has the last word.
fn middle_reset<N: egui::emath::Numeric>(ui: &egui::Ui, resp: &egui::Response, v: &mut N, def: N) {
    let rect = resp.rect;
    let hit = ui.input(|i| {
        i.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Middle,
                    ..
                } if rect.contains(*pos)
            )
        })
    });
    if hit {
        *v = def;
    }
}

/// Snap a viewer zoom to the next/previous step on the 100%-lock ladder: whole
/// multiples at or above 1× (100% steps — pixel-perfect nearest-neighbor), quarter
/// steps below 1×. `up` = zooming in.
fn snap_zoom(z: f32, up: bool) -> f32 {
    let n = if up {
        if z < 1.0 - 1e-4 {
            ((z * 4.0).floor() + 1.0) / 4.0
        } else {
            z.floor() + 1.0
        }
    } else if z > 1.0 + 1e-4 {
        z.ceil() - 1.0
    } else {
        ((z * 4.0).ceil() - 1.0) / 4.0
    };
    n.clamp(0.25, 64.0)
}

/// Snap a zoom to the nearest ladder value — used when the lock is switched on.
fn snap_zoom_nearest(z: f32) -> f32 {
    if z >= 1.0 {
        z.round().clamp(1.0, 64.0)
    } else {
        ((z * 4.0).round() / 4.0).clamp(0.25, 1.0)
    }
}

/// One labeled slider row body: `[label][== wide slider ==][right-justified value]`.
/// The value is a right-anchored `DragValue` (editable, and its right edge lines up
/// across rows). Handles middle-click-reset and wheel-step on the slider. Does NOT
/// draw any reset/move buttons — the caller adds those after it.
#[allow(clippy::too_many_arguments)]
fn value_slider<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    label_w: f32,
    slider_w: f32,
    value_w: f32,
    v: &mut N,
    lo: N,
    hi: N,
    def: N,
    step: f64,
    decimals: usize,
) {
    let h = ui.spacing().interact_size.y;
    ui.add_sized([label_w, h], egui::Label::new(label));
    ui.spacing_mut().slider_width = slider_w;
    let mut sl = egui::Slider::new(&mut *v, lo..=hi).show_value(false);
    if step > 0.0 {
        sl = sl.step_by(step);
    }
    let resp = ui.add(sl);
    middle_reset(ui, &resp, &mut *v, def);
    let wstep = if step > 0.0 {
        step
    } else {
        (hi.to_f64() - lo.to_f64()) / 100.0
    };
    wheel_adjust(ui, &resp, &mut *v, wstep, lo, hi);
    // Value box: right-anchored within a fixed slot so the column is right-justified.
    ui.allocate_ui_with_layout(
        egui::vec2(value_w, h),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.add(
                egui::DragValue::new(&mut *v)
                    .range(lo..=hi)
                    .fixed_decimals(decimals),
            );
        },
    );
}

/// A small painted grip (two columns of three dots) that senses dragging — the
/// reorder handle for the adjustment rows. Painted (not a glyph) so it can't tofu.
/// Returns its response so the caller can detect `drag_started`.
fn drag_handle(ui: &mut egui::Ui, w: f32, h: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::drag());
    let color = if resp.hovered() || resp.dragged() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let c = rect.center();
    for col in [-1.0_f32, 1.0] {
        for row in [-1.0_f32, 0.0, 1.0] {
            ui.painter()
                .circle_filled(egui::pos2(c.x + col * 3.0, c.y + row * 4.0), 1.3, color);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::Grab)
}

/// While `resp` (a ComboBox header) holds the pointer, the mouse wheel steps
/// `*index` through `0..len` (wheel up = previous, down = next), clamped at the
/// ends. Consumes the scroll. Returns true if `*index` changed.
fn wheel_cycle(ui: &egui::Ui, resp: &egui::Response, index: &mut usize, len: usize) -> bool {
    if len == 0 || !resp.contains_pointer() {
        return false;
    }
    let notches: f32 = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                _ => None,
            })
            .sum()
    });
    ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
    if notches == 0.0 {
        return false;
    }
    let new = if notches > 0.0 {
        index.saturating_sub(1)
    } else {
        (*index + 1).min(len - 1)
    };
    let changed = new != *index;
    *index = new;
    changed
}

/// The visible (non-hidden) subdirectories of `dir`, sorted by path.
fn subdirs_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && !is_hidden(p))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Cheap "does this folder contain at least one subfolder?" — uses the dirent's
/// `file_type()` (no per-entry `stat` on Linux) and short-circuits on the first
/// hit, so even a folder of hundreds of images is just one `read_dir`.
fn has_subdirs(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in rd.flatten() {
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) && !is_hidden(&e.path()) {
            return true;
        }
    }
    false
}

/// Draw a tree row's clickable folder label (frameless; navigates on click).
fn folder_tree_label(ui: &mut egui::Ui, path: &Path, nav: &mut Option<PathBuf>) {
    let btn = egui::Button::new(format!("📁 {}", short_name(path)))
        .frame(false)
        .wrap_mode(egui::TextWrapMode::Extend);
    if ui.add(btn).clicked() {
        *nav = Some(path.to_path_buf());
    }
}

/// One node of the explorer folder tree. Only folders that actually contain
/// subfolders get a disclosure triangle (which expands lazily — `body` only runs
/// when open, so collapsed nodes do no further I/O); leaf folders render flush,
/// indented to line their names up with the branches. Clicking a name navigates.
fn folder_tree_node(ui: &mut egui::Ui, path: &Path, nav: &mut Option<PathBuf>) {
    use egui::containers::collapsing_header::CollapsingState;
    if has_subdirs(path) {
        let id = ui.make_persistent_id(("ftree", path));
        CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| folder_tree_label(ui, path, nav))
            .body(|ui| {
                for child in subdirs_sorted(path) {
                    folder_tree_node(ui, &child, nav);
                }
            });
    } else {
        // No subfolders → no triangle; pad by the toggle's width so the name aligns.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(ui.spacing().indent);
            folder_tree_label(ui, path, nav);
        });
    }
}

/// Truncate `s` to at most `max_chars` glyphs, appending an ellipsis if cut.
/// Char-based (not pixel-perfect) but cheap and good enough for a tile caption.
fn elide(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let keep = max_chars.saturating_sub(1).max(1);
    format!("{}…", s.chars().take(keep).collect::<String>())
}

/// The caption lines to show under a grid tile, given the enabled `fields`
/// bitmask. Folders always show just their name; images show the chosen fields
/// (skipping any that don't apply, e.g. dimensions before the thumbnail decodes).
fn caption_lines(
    entry: &Entry,
    meta: Option<ImgMeta>,
    fields: u16,
    folder: Option<&str>,
) -> Vec<String> {
    if entry.is_dir {
        return vec![short_name(&entry.path)];
    }
    let mut out = Vec::new();
    for &(mask, _) in CAPTION_FIELDS {
        if fields & mask == 0 {
            continue;
        }
        let line = match mask {
            CAP_NAME => Some(short_name(&entry.path)),
            CAP_KIND => entry
                .path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_uppercase()),
            CAP_SIZE => Some(human_size(entry.size)),
            CAP_RATING => (entry.rating > 0).then(|| "★".repeat(entry.rating as usize)),
            CAP_COLORS => meta
                .and_then(|m| m.colors)
                .map(|c| format!("{c} color{}", if c == 1 { "" } else { "s" })),
            CAP_DIMENSIONS => meta.map(|m| format!("{} × {}", m.w, m.h)),
            _ => None,
        };
        if let Some(l) = line {
            out.push(l);
        }
    }
    // Search results add the (root-relative) folder the match lives in.
    if let Some(f) = folder {
        out.push(f.to_string());
    }
    out
}

/// Move the favorite at `from` so it lands at the `to` slot (drag-reorder). The
/// index shift from removing `from` first is accounted for, so dropping an item
/// onto a later slot places it just before that slot's original occupant.
fn reorder_favorites<T>(list: &mut Vec<T>, from: usize, to: usize) {
    if from >= list.len() || to >= list.len() || from == to {
        return;
    }
    let item = list.remove(from);
    let insert_at = if from < to { to - 1 } else { to };
    list.insert(insert_at.min(list.len()), item);
}

/// A non-colliding path for `name` in `dir`: the bare name if free, else
/// "stem (copy).ext", "stem (copy 2).ext", … (Dolphin/Finder-style).
fn dedup_path(dir: &Path, name: &OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let as_path = Path::new(name);
    let stem = as_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("item");
    let ext = as_path.extension().and_then(|s| s.to_str());
    for n in 1.. {
        let suffix = if n == 1 {
            " (copy)".to_string()
        } else {
            format!(" (copy {n})")
        };
        let cand = match ext {
            Some(e) => dir.join(format!("{stem}{suffix}.{e}")),
            None => dir.join(format!("{stem}{suffix}")),
        };
        if !cand.exists() {
            return cand;
        }
    }
    unreachable!()
}

/// Pure filter + sort behind `rebuild_view` — extracted so it can be unit-tested
/// without constructing a full `PixelView` (which spawns worker threads).
#[allow(clippy::too_many_arguments)]
fn sorted_filtered_view(
    all: &[Entry],
    key: SortKey,
    desc: bool,
    dirs_first: bool,
    min_rating: u8,
    name_filter: Option<&str>,
    meta: &HashMap<PathBuf, ImgMeta>,
    pieces: &HashMap<PathBuf, ColoPiece>,
) -> Vec<Entry> {
    use std::cmp::Ordering;
    let needle = name_filter
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let mut v: Vec<Entry> = all
        .iter()
        .filter(|e| e.is_dir || e.is_archive || e.rating >= min_rating)
        .filter(|e| match &needle {
            Some(q) => e
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_ascii_lowercase().contains(q)),
            None => true,
        })
        .cloned()
        .collect();

    let name = |e: &Entry| e.path.file_name().map(|s| s.to_ascii_lowercase());
    let ext = |e: &Entry| e.path.extension().map(|s| s.to_ascii_lowercase());
    // Distinct-color count, filled in lazily by the thumbnailer. Not-yet-decoded
    // images (and the "too many colors" cap) read as None and sort *after* every
    // known count, so the order settles toward the front as tiles decode.
    let colors = |e: &Entry| meta.get(&e.path).and_then(|m| m.colors);
    // Pixel dimensions (area, then width), also from the thumbnailer's lazy metadata.
    let dims = |e: &Entry| meta.get(&e.path).map(|m| (m.w as u64 * m.h as u64, m.w));
    // 16colo.rs scene metadata for the flat-listing sort keys (empty for local folders).
    let piece = |e: &Entry| pieces.get(&e.path);
    let artist = |e: &Entry| piece(e).map(|p| p.artist.to_ascii_lowercase());
    let group = |e: &Entry| piece(e).map(|p| p.group.to_ascii_lowercase());
    let year = |e: &Entry| piece(e).map(|p| p.year);
    let pack = |e: &Entry| piece(e).map(|p| p.pack.to_ascii_lowercase());

    // Archives sort alongside folders (they're navigated into like folders).
    let folder_like = |e: &Entry| e.is_dir || e.is_archive;
    v.sort_by(|a, b| {
        if dirs_first {
            match folder_like(b).cmp(&folder_like(a)) {
                Ordering::Equal => {}
                ord => return ord,
            }
        }
        let primary = match key {
            SortKey::Name => name(a).cmp(&name(b)),
            SortKey::Type => ext(a).cmp(&ext(b)).then_with(|| name(a).cmp(&name(b))),
            SortKey::Modified => a.mtime.cmp(&b.mtime),
            SortKey::Created => a.ctime.cmp(&b.ctime),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Rating => a.rating.cmp(&b.rating),
            // None (unknown) sorts last in *both* directions, then by count.
            SortKey::Colors => match (colors(a), colors(b)) {
                (Some(x), Some(y)) => {
                    let ord = x.cmp(&y);
                    if desc {
                        ord.reverse()
                    } else {
                        ord
                    }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            // Dimensions: same "unknown (not-yet-decoded) sorts last" rule as Colors.
            SortKey::Dimensions => match (dims(a), dims(b)) {
                (Some(x), Some(y)) => {
                    let ord = x.cmp(&y);
                    if desc {
                        ord.reverse()
                    } else {
                        ord
                    }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            // 16colo.rs scene keys (None only for non-piece entries → sorts first asc).
            SortKey::Artist => artist(a).cmp(&artist(b)),
            SortKey::Group => group(a).cmp(&group(b)),
            SortKey::Year => year(a).cmp(&year(b)),
            SortKey::Pack => pack(a).cmp(&pack(b)),
        };
        // Colors/Dimensions already applied their own direction (so unknowns stay
        // last); the other keys flip here.
        let primary = if desc && !matches!(key, SortKey::Colors | SortKey::Dimensions) {
            primary.reverse()
        } else {
            primary
        };
        primary.then_with(|| name(a).cmp(&name(b)))
    });
    v
}

/// True if the path has a `.gif` extension.
fn is_gif(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
}

/// Pick a pseudo-random element, seeded from the wall clock (no `rand` dependency —
/// good enough for a screensaver's pack shuffle). None if empty.
fn pick_random<T>(items: &[T]) -> Option<&T> {
    if items.is_empty() {
        return None;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mixed = nanos ^ (nanos >> 17) ^ (nanos << 11); // cheap scramble
    items.get((mixed % items.len() as u128) as usize)
}

/// Read + decode an animated GIF and upload its frames as textures. Returns None
/// for a non-animated GIF or one exceeding `max_pixels` (w*h*frames) — the cap
/// keeps grid hover-play from decoding a giant GIF on a passing mouse-over.
fn build_anim(ctx: &egui::Context, path: &std::path::Path, max_pixels: usize) -> Option<AnimState> {
    let bytes = std::fs::read(path).ok()?;
    let af = crate::anim::decode_gif(&bytes)?;
    let total = (af.width as usize)
        .saturating_mul(af.height as usize)
        .saturating_mul(af.frames.len());
    if total == 0 || total > max_pixels {
        return None;
    }
    let size = [af.width as usize, af.height as usize];
    let frames = af
        .frames
        .iter()
        .enumerate()
        .map(|(i, fr)| {
            let raw: Vec<u8> = fr.iter().flat_map(|&p| p).collect();
            load_texture_capped(
                ctx,
                format!("{}#{i}", path.to_string_lossy()),
                size,
                &raw,
                egui::TextureOptions::NEAREST_REPEAT,
            )
        })
        .collect();
    Some(AnimState {
        path: path.to_path_buf(),
        frames,
        delays_ms: af.delays_ms,
        size,
        current: 0,
        playing: true,
        acc_ms: 0.0,
    })
}

/// Max GPU texture dimension we'll upload. wgpu's default `max_texture_dimension_2d`
/// is 8192; a texture larger than this fails `Device::create_texture` with a
/// validation error that panics the whole app, so oversized images are downscaled
/// to fit before upload.
const MAX_TEX_DIM: usize = 8192;

/// Nearest-neighbor downscale `rgba` (`size` = `[w, h]`, RGBA8) so neither
/// dimension exceeds `cap`, preserving aspect ratio. Returns `None` when it
/// already fits (the common case — no copy).
fn downscale_to_cap(size: [usize; 2], rgba: &[u8], cap: usize) -> Option<([usize; 2], Vec<u8>)> {
    let [w, h] = size;
    if w <= cap && h <= cap {
        return None;
    }
    let scale = (cap as f64 / w as f64).min(cap as f64 / h as f64);
    let nw = ((w as f64 * scale).floor() as usize).clamp(1, cap);
    let nh = ((h as f64 * scale).floor() as usize).clamp(1, cap);
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let sy = (y * h) / nh;
        for x in 0..nw {
            let sx = (x * w) / nw;
            let si = (sy * w + sx) * 4;
            let di = (y * nw + x) * 4;
            out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
        }
    }
    Some(([nw, nh], out))
}

/// One tile of a [`TiledTexture`]: a GPU texture plus its pixel rect within the
/// full image.
#[derive(Clone)]
struct ImgTile {
    tex: egui::TextureHandle,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// A logical image uploaded as a grid of GPU textures, each ≤ [`MAX_TEX_DIM`] in
/// both dimensions. Images within the limit are a single tile (no extra cost);
/// larger ones are split so they still render at **full resolution** — the painter
/// blits each tile into its sub-rect, so zoom stays pixel-accurate at any size.
#[derive(Clone)]
struct TiledTexture {
    size: [usize; 2], // full logical image size in pixels
    tiles: Vec<ImgTile>,
}

impl TiledTexture {
    /// Wrap an already-built single texture (e.g. a GIF frame) as one tile.
    fn single(tex: egui::TextureHandle, size: [usize; 2]) -> Self {
        Self {
            tiles: vec![ImgTile {
                tex,
                x: 0,
                y: 0,
                w: size[0],
                h: size[1],
            }],
            size,
        }
    }

    /// Split full-resolution `rgba` (`size`, RGBA8) into ≤ [`MAX_TEX_DIM`] tiles
    /// and upload each. A within-limit image yields exactly one tile.
    fn from_rgba(
        ctx: &egui::Context,
        name: &str,
        size: [usize; 2],
        rgba: &[u8],
        options: egui::TextureOptions,
    ) -> Self {
        let [w, _] = size;
        let tiles = tile_grid(size, MAX_TEX_DIM)
            .into_iter()
            .map(|(x, y, tw, th)| {
                // Copy this tile's sub-rect out of the full buffer, row by row.
                let mut sub = vec![0u8; tw * th * 4];
                for row in 0..th {
                    let src = ((y + row) * w + x) * 4;
                    let dst = row * tw * 4;
                    sub[dst..dst + tw * 4].copy_from_slice(&rgba[src..src + tw * 4]);
                }
                let color = egui::ColorImage::from_rgba_unmultiplied([tw, th], &sub);
                let tex = ctx.load_texture(format!("{name}#{x}x{y}"), color, options);
                ImgTile {
                    tex,
                    x,
                    y,
                    w: tw,
                    h: th,
                }
            })
            .collect();
        Self { size, tiles }
    }
}

/// The grid of `(x, y, w, h)` pixel rects that tile a `size` image into pieces no
/// larger than `cap` in either dimension. One rect for a within-limit image.
fn tile_grid(size: [usize; 2], cap: usize) -> Vec<(usize, usize, usize, usize)> {
    let [w, h] = size;
    let mut rects = Vec::new();
    let mut y = 0;
    while y < h {
        let th = (h - y).min(cap);
        let mut x = 0;
        while x < w {
            let tw = (w - x).min(cap);
            rects.push((x, y, tw, th));
            x += tw;
        }
        y += th;
    }
    rects
}

/// Build a texture from full-resolution `rgba`, downscaling (nearest) if either
/// dimension exceeds [`MAX_TEX_DIM`]. Callers keep the original `size` for layout —
/// egui samples the (possibly smaller) texture into the full-size rect, so the
/// image still draws and zooms at its true dimensions. Used where a single texture
/// is required (GIF frames); still images use [`TiledTexture`] for full resolution.
fn load_texture_capped(
    ctx: &egui::Context,
    name: impl Into<String>,
    size: [usize; 2],
    rgba: &[u8],
    options: egui::TextureOptions,
) -> egui::TextureHandle {
    let color = match downscale_to_cap(size, rgba, MAX_TEX_DIM) {
        Some((tsize, tpx)) => egui::ColorImage::from_rgba_unmultiplied(tsize, &tpx),
        None => egui::ColorImage::from_rgba_unmultiplied(size, rgba),
    };
    ctx.load_texture(name, color, options)
}

/// Largest centered rect with `content`'s aspect ratio that fits inside `into`.
fn fit_centered(into: egui::Rect, content: egui::Vec2) -> egui::Rect {
    let scale = (into.width() / content.x).min(into.height() / content.y);
    let size = content * scale;
    egui::Rect::from_center_size(into.center(), size)
}

/// Build an `Entry` with its cheap metadata (size, mtime) read up front. The rating
/// is left 0 here and resolved by the app (`read_rating`), which has the sidecar and
/// the display-path context a free function lacks.
fn make_entry(path: PathBuf, is_dir: bool) -> Entry {
    let md = std::fs::metadata(&path).ok();
    let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = md.as_ref().and_then(|m| m.modified().ok());
    let ctime = md.as_ref().and_then(|m| m.created().ok());
    let is_archive = !is_dir && crate::archive::is_archive(&path);
    Entry {
        path,
        is_dir,
        is_archive,
        size,
        mtime,
        ctime,
        rating: 0,
    }
}

/// Hover-tooltip contents for a grid tile.
/// A deferred right-click-menu choice for a grid tile / table row — applied by the
/// caller (which holds `&mut self`), so the menu closure needn't borrow `self`.
enum TilePick {
    Pin,
    PinFolder, // pin the *current* folder (e.g. the 16colo artist/group/search) to Places
    Smart(SmartCriterion),
    File(FileAction),
    ToggleViewed(bool),     // mark this entry viewed (true) / not viewed (false)
    Download(bool),         // save a 16colo piece (false) or its whole pack .zip (true)
    Rate(u8),               // set this entry's star rating (0 = clear), via the context menu
}

/// The shared right-click context menu for a browse entry (used by both the grid tile
/// and the table row). Returns the chosen action, or `None` while the menu is still
/// open. `pinned` (already-a-favorite) and `can_paste` are passed in so this needn't
/// touch `self`. `colo_pin` is `Some(label)` inside a 16colo flat listing — the rows are
/// *pieces* (not dirs), so this offers pinning the current artist/group/search itself.
fn entry_context_menu(
    ui: &mut egui::Ui,
    entry: &Entry,
    can_paste: bool,
    pinned: bool,
    viewed: bool,
    colo_pin: Option<(&str, bool)>, // (label, already-pinned)
    colo_piece: bool,               // a 16colo.rs piece → offer Download
) -> Option<TilePick> {
    let mut pick = None;
    // 16colo.rs piece: save the single file or the whole pack .zip to disk.
    if colo_piece {
        if ui
            .button("⬇ Download file…")
            .on_hover_text("Save this art file from 16colo.rs to your disk")
            .clicked()
        {
            pick = Some(TilePick::Download(false));
            ui.close();
        }
        if ui
            .button("⬇ Download pack .zip…")
            .on_hover_text("Save the whole pack archive")
            .clicked()
        {
            pick = Some(TilePick::Download(true));
            ui.close();
        }
        ui.separator();
    }
    // In a flat artist/group/search listing, let the user bookmark the whole listing
    // (a pinned `…/search/artist/x` re-runs that search when clicked).
    if let Some((label, already)) = colo_pin {
        if ui
            .add_enabled(
                !already,
                egui::Button::new(format!("📌 Pin “{label}” to Places")),
            )
            .on_hover_text("Bookmark this artist / group / search")
            .clicked()
        {
            pick = Some(TilePick::PinFolder);
            ui.close();
        }
        ui.separator();
    }
    if entry.is_dir {
        if ui
            .add_enabled(!pinned, egui::Button::new("📌 Pin to Places"))
            .clicked()
        {
            pick = Some(TilePick::Pin);
            ui.close();
        }
        ui.separator();
    } else {
        ui.menu_button("🔍 Smart filter on…", |ui| {
            let mut on = |ui: &mut egui::Ui, label: &str, c| {
                if ui.button(label).clicked() {
                    pick = Some(TilePick::Smart(c));
                    ui.close();
                }
            };
            on(ui, "Type", SmartCriterion::Type);
            on(ui, "File name", SmartCriterion::Name);
            on(ui, "File size (±20%)", SmartCriterion::Size);
            on(ui, "Date modified", SmartCriterion::Date);
            if entry.rating > 0 {
                on(ui, "Rating (this ★ or more)", SmartCriterion::Rating);
            }
            if is_textmode_ext(&entry.path) {
                ui.separator();
                on(ui, "SAUCE group", SmartCriterion::Group);
                on(ui, "SAUCE artist", SmartCriterion::Artist);
            }
        });
        ui.separator();
    }
    // Star rating (files only) — same effect as the 0-5 hotkeys, shown alongside each
    // entry, so 16colo pieces (where the keys can be fiddly) are always ratable here.
    if !entry.is_dir {
        ui.menu_button("★ Rating", |ui| {
            let cur = entry.rating;
            let mut row = |ui: &mut egui::Ui, stars: u8, label: &str, key: &str| {
                if ui
                    .add(egui::Button::selectable(cur == stars, label).shortcut_text(key))
                    .clicked()
                {
                    pick = Some(TilePick::Rate(stars));
                    ui.close();
                }
            };
            row(ui, 0, "Unrated", "0");
            row(ui, 1, "★", "1");
            row(ui, 2, "★★", "2");
            row(ui, 3, "★★★", "3");
            row(ui, 4, "★★★★", "4");
            row(ui, 5, "★★★★★", "5");
        });
        ui.separator();
    }
    // Toggle visited state (drives the table link colour + grid/pack check badge).
    if ui
        .button(if viewed {
            "Mark as not viewed"
        } else {
            "Mark as viewed"
        })
        .clicked()
    {
        pick = Some(TilePick::ToggleViewed(!viewed));
        ui.close();
    }
    ui.separator();
    let mut file = |ui: &mut egui::Ui, label: &str, enabled: bool, a: FileAction| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            pick = Some(TilePick::File(a));
            ui.close();
        }
    };
    file(ui, "Copy", true, FileAction::Copy);
    file(ui, "Cut", true, FileAction::Cut);
    file(ui, "Paste", can_paste, FileAction::Paste);
    ui.separator();
    file(ui, "Rename…", true, FileAction::Rename);
    file(ui, "Move to trash", true, FileAction::Delete);
    ui.separator();
    file(ui, "New folder", true, FileAction::NewFolder);
    pick
}

/// The text shown in a table cell of kind `kind` for `entry` (the thumbnail, rating,
/// and download cells are painted/handled by the caller, so they produce nothing here).
fn table_cell_text(
    entry: &Entry,
    meta: Option<ImgMeta>,
    piece: Option<&ColoPiece>,
    kind: ColKind,
) -> String {
    match kind {
        ColKind::Name => entry
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string(),
        ColKind::Type => entry
            .path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_uppercase())
            .unwrap_or_default(),
        // Hide a 0-byte size (folders, virtual dirs) — "0 B" reads as noise.
        ColKind::Size => {
            if entry.size == 0 {
                String::new()
            } else {
                human_size(entry.size)
            }
        }
        ColKind::Dims => meta.map(|m| format!("{}×{}", m.w, m.h)).unwrap_or_default(),
        ColKind::Colors => meta
            .and_then(|m| m.colors)
            .map(|c| c.to_string())
            .unwrap_or_default(),
        ColKind::Modified => entry.mtime.map(date_ymd).unwrap_or_default(),
        ColKind::Created => entry.ctime.map(date_ymd).unwrap_or_default(),
        ColKind::Artist => piece.map(|p| p.artist.clone()).unwrap_or_default(),
        ColKind::Year => piece.map(|p| p.year.to_string()).unwrap_or_default(),
        ColKind::Group => piece.map(|p| p.group.clone()).unwrap_or_default(),
        ColKind::Pack => piece.map(|p| p.pack.clone()).unwrap_or_default(),
        // Painted/handled by the caller, not text:
        ColKind::Thumb | ColKind::Rating | ColKind::Download => String::new(),
    }
}

/// Build a piece's `Entry` + `ColoPiece` and send it as a hit. Returns false if the
/// receiver is gone (the user navigated away → stop the walk). The virtual display
/// path is `<16colo.rs>/<year>/<pack>/<FILE>` — the same scheme a downloaded pack uses.
fn emit_piece(
    tx: &std::sync::mpsc::Sender<ColoMsg>,
    p: crate::sixteen::Piece,
    count: &mut usize,
) -> bool {
    let path = Path::new(crate::sixteen::ROOT)
        .join(p.year.to_string())
        .join(&p.pack)
        .join(&p.filename);
    let entry = Entry {
        path,
        is_dir: false,
        is_archive: false,
        size: p.filesize, // from the API SAUCE record (0 = unknown → hidden in the table)
        mtime: None,
        ctime: None,
        rating: 0,
    };
    let piece = ColoPiece {
        artist: p.artist,
        group: p.group,
        year: p.year,
        pack: p.pack,
        raw_url: p.raw_url,
        tn_url: p.tn_url,
        sauce: p.sauce,
    };
    *count += 1;
    tx.send(ColoMsg::Hit(entry, Box::new(piece))).is_ok()
}

/// Stream one 16colo artist's pieces into `tx`. Tries the direct `/artist/{name}`
/// endpoint first (one call, works for single-word handles); when that comes back empty
/// — which it does for names with a *space* (`fetch_artist_pieces` quirk) — falls back to
/// the artist's pack list (search `details`) and fetches each pack, streaming the pieces
/// credited to this artist. Returns false to stop (cancelled / channel closed / hit the
/// piece cap, in which case a `Done` was already sent).
fn emit_artist(
    name: &str,
    tx: &std::sync::mpsc::Sender<ColoMsg>,
    cancel: &std::sync::atomic::AtomicBool,
    count: &mut usize,
    max_pieces: usize,
) -> bool {
    use std::sync::atomic::Ordering::Relaxed;
    let emit = |p: crate::sixteen::Piece, count: &mut usize| -> Option<bool> {
        if cancel.load(Relaxed) || !emit_piece(tx, p, count) {
            return Some(false); // stop
        }
        if *count >= max_pieces {
            let _ = tx.send(ColoMsg::Done(*count));
            return Some(false);
        }
        None // keep going
    };
    // Direct endpoint — fast path for single-word handles.
    if let Ok(pieces) = crate::sixteen::fetch_artist_pieces(name) {
        if !pieces.is_empty() {
            for p in pieces {
                if let Some(stop) = emit(p, count) {
                    return stop;
                }
            }
            return true;
        }
    }
    // Fallback: the multi-word artist's packs, each fetched + filtered to this artist.
    let want = name.to_lowercase();
    for pack in crate::sixteen::fetch_artist_packs(name).unwrap_or_default() {
        if cancel.load(Relaxed) {
            return false;
        }
        let Ok(pieces) = crate::sixteen::fetch_pack_pieces("", 0, &pack) else {
            continue; // a single pack failing shouldn't abort the whole artist
        };
        for p in pieces
            .into_iter()
            .filter(|p| p.artist.to_lowercase().split(", ").any(|a| a == want))
        {
            if let Some(stop) = emit(p, count) {
                return stop;
            }
        }
    }
    true
}

/// Background worker for a flat 16colo.rs piece listing (see
/// [`PixelView::start_colo_pieces`]). Streams a `ColoMsg::Hit` per piece, then
/// `Done(total)`. An artist is one API call; a group fetches each of its packs; a
/// search aggregates matched artists + groups (capped, so a broad query stays bounded).
/// Checks `cancel` between pieces / network calls so navigation stops it promptly.
fn colo_walk(
    source: ColoSource,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    tx: std::sync::mpsc::Sender<ColoMsg>,
) {
    use crate::sixteen;
    use std::sync::atomic::Ordering::Relaxed;
    let mut count = 0usize;

    // A search can match many artists/groups; cap the fan-out so a broad query can't
    // fetch thousands of packs. (The status reports the final count.)
    const MAX_ARTISTS: usize = 25;
    const MAX_GROUPS: usize = 15;
    const MAX_PIECES: usize = 4000;
    // Fetch + emit every piece by artists/groups whose name matches `query` (substring).
    // Returns false when the caller should stop (cancelled, channel dropped, or the
    // piece cap was hit — in which case a `Done` was already sent).
    let do_artists = |query: &str, count: &mut usize| -> bool {
        for a in sixteen::search_artists(query)
            .unwrap_or_default()
            .into_iter()
            .take(MAX_ARTISTS)
        {
            if cancel.load(Relaxed) {
                return false;
            }
            // Streams the matched artist (direct endpoint, or pack-fallback for spaces).
            if !emit_artist(&a, &tx, &cancel, count, MAX_PIECES) {
                return false;
            }
        }
        true
    };
    let do_groups = |query: &str, count: &mut usize| -> bool {
        for g in sixteen::search_groups(query)
            .unwrap_or_default()
            .into_iter()
            .take(MAX_GROUPS)
        {
            if cancel.load(Relaxed) {
                return false;
            }
            for (year, pack) in sixteen::fetch_group_pack_refs(&g).unwrap_or_default() {
                if cancel.load(Relaxed) {
                    return false;
                }
                if let Ok(pieces) = sixteen::fetch_pack_pieces(&g, year, &pack) {
                    for p in pieces {
                        if cancel.load(Relaxed) || !emit_piece(&tx, p, count) {
                            return false;
                        }
                        if *count >= MAX_PIECES {
                            let _ = tx.send(ColoMsg::Done(*count));
                            return false;
                        }
                    }
                }
            }
        }
        true
    };

    match source {
        ColoSource::Artist(name) => {
            // Direct endpoint, or pack-fallback for multi-word names (see emit_artist).
            emit_artist(&name, &tx, &cancel, &mut count, MAX_PIECES);
        }
        ColoSource::Group(name) => match sixteen::fetch_group_pack_refs(&name) {
            Ok(refs) => {
                for (year, pack) in refs {
                    if cancel.load(Relaxed) {
                        return;
                    }
                    // A single pack fetch may fail; skip it rather than abort the listing.
                    if let Ok(pieces) = sixteen::fetch_pack_pieces(&name, year, &pack) {
                        for p in pieces {
                            if cancel.load(Relaxed) || !emit_piece(&tx, p, &mut count) {
                                return;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(ColoMsg::Err(e));
                return;
            }
        },
        ColoSource::Search(query) => {
            if !do_artists(&query, &mut count) || !do_groups(&query, &mut count) {
                return;
            }
        }
        ColoSource::SearchArtists(query) => {
            if !do_artists(&query, &mut count) {
                return;
            }
        }
        ColoSource::SearchGroups(query) => {
            if !do_groups(&query, &mut count) {
                return;
            }
        }
    }
    let _ = tx.send(ColoMsg::Done(count));
}

fn hover_details(ui: &mut egui::Ui, entry: &Entry, meta: Option<ImgMeta>) {
    // Pin the tooltip to a consistent width. Without this, egui shrinks it to the
    // space available near a panel/screen edge (the leftmost grid column), which
    // wraps the long filename one word per line.
    ui.set_min_width(200.0);
    ui.set_max_width(280.0);
    ui.strong(short_name(&entry.path));
    if entry.is_dir {
        ui.label("Folder");
        return;
    }
    let fmt = entry
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    if let Some(m) = meta {
        ui.label(format!("{}×{}   {}", m.w, m.h, fmt));
        match m.colors {
            Some(c) => ui.label(format!("{c} colors")),
            None => ui.label("many colors"),
        };
    } else if !fmt.is_empty() {
        ui.label(fmt);
    }
    ui.label(format!("Size: {}", human_size(entry.size)));
    ui.label(format!("Rating: {}", stars_label(entry.rating)));
    if let Some(t) = entry.mtime {
        ui.label(format!("Modified: {}", fmt_time(t)));
    }
}

/// Human-readable byte size.
fn human_size(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut b = bytes as f64;
    let mut i = 0;
    while b >= 1024.0 && i < U.len() - 1 {
        b /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{b:.1} {}", U[i])
    }
}

/// Stars as a string, or an em-dash when unrated.
fn stars_label(rating: u8) -> String {
    if rating == 0 {
        "—".into()
    } else {
        "★".repeat(rating as usize)
    }
}

/// Relative "modified" time, dependency-free.
fn fmt_time(t: SystemTime) -> String {
    match t.elapsed() {
        Ok(d) => {
            let s = d.as_secs();
            if s < 60 {
                format!("{s}s ago")
            } else if s < 3600 {
                format!("{}m ago", s / 60)
            } else if s < 86_400 {
                format!("{}h ago", s / 3600)
            } else {
                format!("{}d ago", s / 86_400)
            }
        }
        Err(_) => "just now".into(),
    }
}

/// The user's home directory, if `$HOME` is set.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// True for dotfiles / hidden entries.
fn is_hidden(p: &std::path::Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

/// Per-folder info cached for the grid: a few preview images (possibly pulled from
/// immediate subdirectories) plus direct image/subdir counts.
#[derive(Clone, Default)]
struct FolderInfo {
    previews: Vec<PathBuf>,
    images: usize,
    subdirs: usize,
}

/// Read up to the last `n` bytes of a file (the whole file if shorter) — used to
/// fetch a SAUCE record without reading a large image in full.
fn read_file_tail(path: &Path, n: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len <= n {
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).ok()?;
        Some(buf)
    } else {
        f.seek(SeekFrom::End(-(n as i64))).ok()?;
        let mut buf = vec![0u8; n as usize];
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    }
}

/// Does this path look like an image we can (or soon will) decode?
fn is_image_ext(p: &std::path::Path) -> bool {
    const EXTS: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "bmp", "webp", "tga", "tif", "tiff", "ppm", "pgm", "pbm",
        "pnm", "qoi", "pcx", "psd", "aseprite", "ase", "xcf", "draw", "ico", "svg", "ans", "asc",
        "nfo", "diz", "txt", "xb", "xbin", "bin", "ice", "cia", "tnd", "idf", "adf", "seq", "pet",
        "petscii", "petmate", "rip",
    ];
    match p.extension().and_then(|x| x.to_str()) {
        Some(x) => EXTS.contains(&x.to_ascii_lowercase().as_str()),
        // Extensionless scene/BBS art (rendered as CP437 text). Dirs are filtered
        // out by an `is_dir()` check at every call site before reaching here.
        None => true,
    }
}

/// Text-mode / scene art: rendered at a tiny native 8×16 px per character cell, so
/// the viewer opens it at `textmode_zoom` (not 1:1) and offers the CRT aspect.
fn is_textmode_ext(p: &std::path::Path) -> bool {
    const EXTS: &[&str] = &[
        "ans", "asc", "nfo", "diz", "txt", "ice", "cia", "xb", "xbin", "bin", "tnd", "idf", "adf",
        "seq", "pet", "petscii", "petmate",
    ];
    match p.extension().and_then(|x| x.to_str()) {
        Some(x) => EXTS.contains(&x.to_ascii_lowercase().as_str()),
        None => true, // extensionless files render through the text-mode path
    }
}

/// Scan `dir` once for the grid: count direct images/subdirs and gather up to 4
/// preview images. The counts are for the *immediate* folder, but the previews fall
/// **recursively** through subdirectories (breadth-first, bounded) so a folder whose
/// images live several levels down — a `dir → subdir → subsubdir/*.png` chain — still
/// surfaces a montage instead of looking empty. Bounded by `MAX_DIRS` so a giant tree
/// can't stall the (cached, first-render) scan.
fn scan_folder_info(dir: &std::path::Path) -> FolderInfo {
    use std::collections::VecDeque;
    const MAX_DIRS: usize = 256; // descendant-directory visit cap for the preview walk
    let mut images = 0usize;
    let mut subdir_paths: Vec<PathBuf> = Vec::new();
    let mut previews: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if is_hidden(&p) {
                continue;
            }
            if p.is_dir() {
                subdir_paths.push(p);
            } else if is_image_ext(&p) {
                images += 1;
                previews.push(p);
            }
        }
    }
    previews.sort();
    previews.truncate(4);
    let subdirs = subdir_paths.len();
    // Deep preview fallback: walk descendants breadth-first until we have 4 previews.
    if previews.len() < 4 {
        subdir_paths.sort();
        let mut queue: VecDeque<PathBuf> = subdir_paths.into_iter().collect();
        let mut visited = 0usize;
        while previews.len() < 4 && visited < MAX_DIRS {
            let Some(d) = queue.pop_front() else { break };
            visited += 1;
            let mut child_dirs: Vec<PathBuf> = Vec::new();
            let mut found: Vec<PathBuf> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if is_hidden(&p) {
                        continue;
                    }
                    if p.is_dir() {
                        child_dirs.push(p);
                    } else if is_image_ext(&p) {
                        found.push(p);
                    }
                }
            }
            found.sort();
            for f in found {
                if previews.len() >= 4 {
                    break;
                }
                previews.push(f);
            }
            child_dirs.sort();
            queue.extend(child_dirs); // descend further if still short
        }
    }
    FolderInfo {
        previews,
        images,
        subdirs,
    }
}

fn parse_dim(s: &str) -> Option<u32> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.parse().ok()).flatten()
}

/// Days since the Unix epoch → `(year, month, day)` — Howard Hinnant's civil-date
/// algorithm, so we can format a file's mtime without pulling in a date crate.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A `SystemTime` as `YYYY-MM-DD` (UTC). YYYY-MM-DD sorts chronologically as text, so
/// search date-range checks are plain string comparisons.
/// Current wall-clock time as unix seconds (0 before the epoch). Used to stamp view
/// history; the app already uses `SystemTime::now()` for the random-pack seed.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Format unix seconds as `YYYY-MM-DD` (view-history "last viewed", etc.).
fn date_ymd_unix(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Paint a rotating "loading" spinner arc centred at `c`. `t` is `ctx.input().time`
/// (drives the rotation; the caller must keep requesting repaints — `want_repaint` is
/// set while thumbs are pending). Used on grid/table tiles whose thumbnail hasn't
/// arrived yet (those are hand-painted, so egui's `Spinner` widget doesn't fit).
fn paint_spinner(p: &egui::Painter, c: egui::Pos2, r: f32, t: f64, color: egui::Color32) {
    const SEG: usize = 20;
    let start = (t * 4.0) as f32; // radians/sec rotation
    let sweep = std::f32::consts::PI * 1.5; // ~270° arc
    let pts: Vec<egui::Pos2> = (0..=SEG)
        .map(|i| {
            let a = start + sweep * (i as f32 / SEG as f32);
            c + r * egui::vec2(a.cos(), a.sin())
        })
        .collect();
    p.add(egui::Shape::line(
        pts,
        egui::Stroke::new((r * 0.18).max(1.5), color),
    ));
}

/// Paint a small "viewed" check badge: a filled green disc with a white check stroke,
/// centred at `c` with radius `r`. Painted rather than drawn as a glyph because egui's
/// bundled font lacks a reliable check mark (see the font gotcha in CLAUDE.md).
fn paint_check_badge(p: &egui::Painter, c: egui::Pos2, r: f32) {
    p.circle_filled(c, r, egui::Color32::from_rgb(70, 175, 90));
    p.circle_stroke(c, r, egui::Stroke::new(1.0, egui::Color32::from_black_alpha(130)));
    let s = egui::Stroke::new((r * 0.30).max(1.5), egui::Color32::WHITE);
    let a = c + egui::vec2(-r * 0.42, r * 0.02);
    let b = c + egui::vec2(-r * 0.08, r * 0.38);
    let d = c + egui::vec2(r * 0.46, -r * 0.40);
    p.line_segment([a, b], s);
    p.line_segment([b, d], s);
}

fn date_ymd(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Cheap-ish pixel dimensions: a header-only read for raster formats the `image`
/// crate knows, falling back to a full decode for scene/odd formats. Only called
/// when a dimension filter is set, so the full-decode cost is opt-in.
fn quick_dims(path: &Path, registry: &crate::decode::Registry) -> Option<(u32, u32)> {
    if let Ok(d) = image::image_dimensions(path) {
        return Some(d);
    }
    registry.decode_path(path).ok().map(|i| (i.width, i.height))
}

/// Does the file's SAUCE record's title/author/group/font contain `q` (lowercased)?
fn sauce_matches(path: &Path, q: &str) -> bool {
    match read_file_tail(path, 128)
        .as_deref()
        .and_then(crate::sauce::parse)
    {
        Some(s) => format!("{} {} {} {}", s.title, s.author, s.group, s.font)
            .to_ascii_lowercase()
            .contains(q),
        None => false,
    }
}

/// Walk `root` recursively (breadth-first) for images matching `spec`, sending each
/// hit over `tx` as it's found. Runs on a background thread; `cancel` aborts it (a
/// new search or closing the panel). Cheap filters (name/type) run first; SAUCE and
/// dimensions (which read/parse the file) only when those fields are set.
fn search_walk(
    root: PathBuf,
    spec: SearchSpec,
    registry: Arc<crate::decode::Registry>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    tx: std::sync::mpsc::Sender<SearchMsg>,
    sidecar: std::collections::HashMap<String, u8>,
) {
    use std::collections::VecDeque;
    use std::sync::atomic::Ordering;
    let stop = || cancel.load(Ordering::Relaxed);
    let name_q = spec.name.trim().to_ascii_lowercase();
    let exts: Vec<String> = spec
        .ext
        .split([',', ' ', ';'])
        .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let sauce_q = spec.sauce.trim().to_ascii_lowercase();
    let (wmin, wmax) = (parse_dim(&spec.wmin), parse_dim(&spec.wmax));
    let (hmin, hmax) = (parse_dim(&spec.hmin), parse_dim(&spec.hmax));
    let need_dims = wmin.or(wmax).or(hmin).or(hmax).is_some();
    let (smin, smax) = (parse_dim(&spec.smin), parse_dim(&spec.smax)); // KB
    let (dfrom, dto) = (spec.dfrom.trim().to_string(), spec.dto.trim().to_string());
    let rmin = parse_dim(&spec.rmin).unwrap_or(0).min(5) as u8;
    let need_meta = smin.or(smax).is_some() || !dfrom.is_empty() || !dto.is_empty();

    let mut count = 0usize;
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(root);
    while let Some(dir) = queue.pop_front() {
        if stop() {
            return;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let (mut dirs, mut files): (Vec<PathBuf>, Vec<PathBuf>) = (Vec::new(), Vec::new());
        for e in rd.flatten() {
            let p = e.path();
            if is_hidden(&p) {
                continue;
            }
            if p.is_dir() {
                dirs.push(p);
            } else if is_image_ext(&p) {
                files.push(p);
            }
        }
        files.sort();
        for f in &files {
            if stop() {
                return;
            }
            if !name_q.is_empty() {
                let n = f
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !n.contains(&name_q) {
                    continue;
                }
            }
            if !exts.is_empty() {
                let e = f
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if !exts.contains(&e) {
                    continue;
                }
            }
            // Size + modified-date share one metadata read.
            if need_meta {
                let Ok(md) = std::fs::metadata(f) else {
                    continue;
                };
                let kb = md.len() / 1024;
                if smin.is_some_and(|v| kb < v as u64) || smax.is_some_and(|v| kb > v as u64) {
                    continue;
                }
                if !dfrom.is_empty() || !dto.is_empty() {
                    let date = md.modified().ok().map(date_ymd).unwrap_or_default();
                    if (!dfrom.is_empty() && date < dfrom) || (!dto.is_empty() && date > dto) {
                        continue;
                    }
                }
            }
            // Rating: the xattr is the on-disk source of truth, but files on a mount
            // (NTFS, etc.) can't carry one — their stars live only in the sidecar, so
            // fall back to it (mirrors `read_rating`), else 5-star art on a mount is
            // invisible to recursive search.
            if rmin > 0 {
                let mut stars = crate::rating::read(f);
                if stars == 0 {
                    stars = sidecar
                        .get(f.to_string_lossy().as_ref())
                        .copied()
                        .unwrap_or(0);
                }
                if stars < rmin {
                    continue;
                }
            }
            if !sauce_q.is_empty() && !sauce_matches(f, &sauce_q) {
                continue;
            }
            if need_dims {
                let Some((w, h)) = quick_dims(f, &registry) else {
                    continue;
                };
                if wmin.is_some_and(|v| w < v)
                    || wmax.is_some_and(|v| w > v)
                    || hmin.is_some_and(|v| h < v)
                    || hmax.is_some_and(|v| h > v)
                {
                    continue;
                }
            }
            count += 1;
            if tx
                .send(SearchMsg::Hit(make_entry(f.clone(), false)))
                .is_err()
            {
                return; // receiver dropped (search closed)
            }
        }
        dirs.sort();
        queue.extend(dirs);
    }
    let _ = tx.send(SearchMsg::Done(count));
}

/// Keyboard shortcuts, shown in Help → Keyboard shortcuts.
const HOTKEYS: &[(&str, &str)] = &[
    ("Ctrl +  /  Ctrl -", "Zoom the whole UI"),
    (
        "Ctrl + Wheel / Pinch",
        "Resize thumbnails (grid) · zoom image (viewer)",
    ),
    ("Wheel", "Viewer: previous / next image · Grid: scroll"),
    (
        "Mouse Back / Fwd",
        "Grid: folder history · Viewer: prev / next image",
    ),
    ("Home / End", "Grid: first / last · Viewer: scroll to top / bottom"),
    ("PageUp / PageDown", "Viewer: scroll 25 lines (a screen of scene art)"),
    ("Arrow Up / Down", "Viewer: scroll a long image"),
    ("/", "Grid: filter by filename"),
    ("Drag", "Pan the image"),
    ("F", "Fit to window + auto-fit new images (viewer)"),
    ("T", "Tile preview — fill window (viewer)"),
    ("1 – 5", "Set star rating"),
    ("0", "Clear rating"),
    ("Click", "Open image / enter folder"),
    ("Ctrl + Click", "Toggle selection"),
    ("Shift + Click", "Range-select"),
    ("Right-click", "Grid: file operations menu"),
    ("Ctrl + C / X / V", "Copy / Cut / Paste"),
    ("Ctrl + N", "New folder"),
    ("F2", "Rename"),
    ("Delete", "Move to trash"),
    ("Ctrl + Z", "Undo last file operation"),
];

/// Command-line options. These override persisted settings; add more flags here.
#[derive(Default)]
pub struct CliArgs {
    pub folder: Option<PathBuf>,
    pub thumb_size: Option<f32>,
}

const USAGE: &str = "\
pixelview — a pixel-art-first image viewer

USAGE:
    pixelview [OPTIONS]

OPTIONS:
    -f, --folder <PATH>           Open this folder on launch
    -t, --thumbnail-size <SIZE>   Thumbnail tile size: a number (e.g. 160) or
                                  WxH (e.g. 120x160 — tiles are square, so the
                                  larger dimension is used)
    -h, --help                    Print this help

Settings passed here override the persisted ones and are remembered afterward.
";

impl CliArgs {
    /// Parse `std::env::args`. Exits the process on `--help` or a bad argument.
    pub fn parse() -> Self {
        let mut out = CliArgs::default();
        let mut args = std::env::args().skip(1);
        while let Some(a) = args.next() {
            match a.as_str() {
                "-h" | "--help" => {
                    print!("{USAGE}");
                    std::process::exit(0);
                }
                "-f" | "--folder" => match args.next() {
                    Some(v) => out.folder = Some(PathBuf::from(v)),
                    None => cli_fail("--folder requires a path"),
                },
                "-t" | "--thumbnail-size" | "--thumb-size" => match args.next() {
                    Some(v) => match parse_thumb_size(&v) {
                        Ok(n) => out.thumb_size = Some(n),
                        Err(e) => cli_fail(&e),
                    },
                    None => cli_fail("--thumbnail-size requires a value like 160 or 120x160"),
                },
                other => cli_fail(&format!("unknown argument '{other}' (try --help)")),
            }
        }
        out
    }
}

fn cli_fail(msg: &str) -> ! {
    eprintln!("pixelview: {msg}");
    std::process::exit(2);
}

/// Parse a thumbnail size given as `N` or `WxH`. Tiles are square, so `WxH`
/// collapses to `max(W, H)`.
fn parse_thumb_size(s: &str) -> Result<f32, String> {
    let bad = || format!("invalid thumbnail size '{s}' (use e.g. 160 or 120x160)");
    if let Some((w, h)) = s.split_once(['x', 'X', '×']) {
        let w: f32 = w.trim().parse().map_err(|_| bad())?;
        let h: f32 = h.trim().parse().map_err(|_| bad())?;
        Ok(w.max(h))
    } else {
        s.trim().parse::<f32>().map_err(|_| bad())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi32_palette_loads_32_colors() {
        // The favorite color picker depends on the bundled ANSI32 palette; guard against
        // a rename/parse break (it'd silently leave the picker empty).
        assert_eq!(ansi32_palette().len(), 32);
        assert_eq!(contrast_text([255, 255, 255]), egui::Color32::BLACK);
        assert_eq!(contrast_text([0, 0, 0]), egui::Color32::WHITE);
    }

    #[test]
    fn cell_reveal_types_out_in_reading_order() {
        // 2×2 grid of 1×1 cells, four distinct colours in reading order.
        let px = vec![
            [1, 1, 1, 255],
            [2, 2, 2, 255],
            [3, 3, 3, 255],
            [4, 4, 4, 255],
        ];
        let cr = CellReveal::new(px, 2, 2, 1, 1);
        assert_eq!(cr.cells, 4);
        // 0 cells → all black.
        assert_eq!(cr.render(0).0.pixels, vec![[0, 0, 0, 255]; 4]);
        // 3 cells → first three revealed, the last still black.
        let img = cr.render(3).0;
        assert_eq!(img.pixels[0], [1, 1, 1, 255]);
        assert_eq!(img.pixels[1], [2, 2, 2, 255]);
        assert_eq!(img.pixels[2], [3, 3, 3, 255]);
        assert_eq!(img.pixels[3], [0, 0, 0, 255]);
        // All cells → the full image.
        assert_eq!(cr.render(4).0.pixels[3], [4, 4, 4, 255]);
    }

    #[test]
    fn baud_cps_and_roundtrip() {
        // ≤2400 is uncompressed (cps = baud/10); 14.4k+ gets ~4× V.42bis compression so
        // it "feels fast" like a real board (14400/10 × 4 = 5760).
        assert_eq!(Baud::B300.cps(), Some(30.0));
        assert_eq!(Baud::B2400.cps(), Some(240.0));
        assert_eq!(Baud::B14400.cps(), Some(5760.0));
        assert_eq!(Baud::None.cps(), None);
        // Persistence index round-trips for every rate.
        for b in Baud::ALL {
            assert_eq!(Baud::from_u8(b.to_u8()), b);
        }
    }

    #[test]
    fn baud_advance_paces_and_stops_at_end() {
        let img = crate::image_types::PixImage::from_rgba(1, 1, vec![[0, 0, 0, 255]]);
        let _ = img; // (Player needs a Stream, exercised below via a tiny text stream)
        let stream = Stream::Text(crate::decode::TextStream::new(b"hello world").unwrap());
        let len = stream.len();
        let mut p = Player::new(PathBuf::from("x.ans"), stream, true);
        assert_eq!(p.pos, 0);
        // 9600 baud = 960 cps; 0.5s → 480 bytes, clamped to len, playback ends.
        p.advance(Baud::B9600, 0.5);
        assert_eq!(p.pos, len);
        assert!(!p.playing, "stops when the stream is fully transmitted");
        // 300 baud = 30 cps; a fresh player advances ~3 bytes in 0.1s.
        let stream = Stream::Text(crate::decode::TextStream::new(b"hello world").unwrap());
        let mut q = Player::new(PathBuf::from("x.ans"), stream, true);
        q.advance(Baud::B300, 0.1);
        assert_eq!(q.pos, 3);
        assert!(q.playing);
    }

    fn img_entry(name: &str, size: u64, rating: u8) -> Entry {
        Entry {
            path: PathBuf::from(name),
            is_dir: false,
            is_archive: false,
            size,
            mtime: None,
            ctime: None,
            rating,
        }
    }
    fn dir_entry(name: &str) -> Entry {
        Entry {
            path: PathBuf::from(name),
            is_dir: true,
            is_archive: false,
            size: 0,
            mtime: None,
            ctime: None,
            rating: 0,
        }
    }

    #[test]
    fn thumb_size_parses_number_or_wxh() {
        assert_eq!(parse_thumb_size("160").unwrap(), 160.0);
        assert_eq!(parse_thumb_size("120x160").unwrap(), 160.0);
        assert_eq!(parse_thumb_size("200X100").unwrap(), 200.0);
        assert_eq!(parse_thumb_size("10×20").unwrap(), 20.0);
        assert!(parse_thumb_size("junk").is_err());
        assert!(parse_thumb_size("12xY").is_err());
    }

    #[test]
    fn downscale_caps_oversized_keeps_aspect_and_fits() {
        // 16000×800 with cap 8000 → halve to 8000×400, every dimension ≤ cap.
        let (w, h) = (16000usize, 800usize);
        let rgba = vec![7u8; w * h * 4];
        let (size, px) = downscale_to_cap([w, h], &rgba, 8000).expect("should downscale");
        assert_eq!(size, [8000, 400]);
        assert_eq!(px.len(), 8000 * 400 * 4);
        assert!(size[0] <= 8000 && size[1] <= 8000);
    }

    #[test]
    fn downscale_passes_through_when_within_cap() {
        let rgba = vec![0u8; 4 * 4 * 4];
        assert!(downscale_to_cap([4, 4], &rgba, 8192).is_none());
    }

    #[test]
    fn builtin_palettes_are_embedded_and_parse() {
        let paths = builtin_palette_paths();
        assert!(paths.len() >= 50, "expected the bundled palette set");
        // Every default favorite resolves to embedded contents that parse to colors.
        for fav in DEFAULT_PALETTE_FAVS {
            let p = Path::new(crate::palettes_builtin::BUILTIN_ROOT).join(fav);
            let text = builtin_palette_contents(&p).expect("favorite is embedded");
            assert!(
                !crate::thumb::parse_gpl(text).is_empty(),
                "{fav} parsed to no colors"
            );
        }
    }

    #[test]
    fn tile_grid_single_when_within_cap() {
        assert_eq!(tile_grid([100, 50], 8192), vec![(0, 0, 100, 50)]);
    }

    #[test]
    fn tile_grid_covers_oversized_image_exactly() {
        // 25×18 split at cap 10 → 3 cols (10,10,5) × 2 rows (10,8) = 6 tiles,
        // tiling the whole image with no gaps or overlaps.
        let rects = tile_grid([25, 18], 10);
        assert_eq!(rects.len(), 6);
        for &(x, y, w, h) in &rects {
            assert!(w <= 10 && h <= 10);
            assert!(x + w <= 25 && y + h <= 18);
        }
        let area: usize = rects.iter().map(|&(_, _, w, h)| w * h).sum();
        assert_eq!(area, 25 * 18); // exact cover, no double-counting
    }

    #[test]
    fn human_size_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(1024 * 1024), "1.0 MB");
    }

    #[test]
    fn stars_label_renders() {
        assert_eq!(stars_label(0), "—");
        assert_eq!(stars_label(3), "★★★");
    }

    #[test]
    fn zoom_lock_ladder_steps() {
        // Up: 100% steps at/above 1×; ¼ steps below, landing exactly on 1×.
        assert_eq!(snap_zoom(1.0, true), 2.0);
        assert_eq!(snap_zoom(2.0, true), 3.0);
        assert_eq!(snap_zoom(0.5, true), 0.75);
        assert_eq!(snap_zoom(0.75, true), 1.0);
        // Down: 100% steps above 1×; ¼ steps at/below, clamped at 25%.
        assert_eq!(snap_zoom(1.0, false), 0.75);
        assert_eq!(snap_zoom(2.0, false), 1.0);
        assert_eq!(snap_zoom(0.5, false), 0.25);
        assert_eq!(snap_zoom(0.25, false), 0.25);
        // Nearest snap when the lock is switched on.
        assert_eq!(snap_zoom_nearest(1.3), 1.0);
        assert_eq!(snap_zoom_nearest(2.6), 3.0);
        assert_eq!(snap_zoom_nearest(0.6), 0.5);
    }

    #[test]
    fn sort_key_u8_roundtrips() {
        for k in SortKey::ALL {
            assert_eq!(SortKey::from_u8(k.to_u8()), k);
        }
    }

    #[test]
    fn name_sort_pins_dirs_first() {
        let all = vec![
            img_entry("b.png", 10, 0),
            img_entry("a.png", 20, 0),
            dir_entry("z_dir"),
        ];
        let v = sorted_filtered_view(
            &all,
            SortKey::Name,
            false,
            true,
            0,
            None,
            &HashMap::new(),
            &HashMap::new(),
        );
        let names: Vec<_> = v.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(names, vec!["z_dir", "a.png", "b.png"]);
    }

    #[test]
    fn rating_filter_keeps_dirs_drops_low_images() {
        let all = vec![
            img_entry("low.png", 1, 2),
            img_entry("high.png", 1, 5),
            dir_entry("d"),
        ];
        let v = sorted_filtered_view(
            &all,
            SortKey::Name,
            false,
            true,
            3,
            None,
            &HashMap::new(),
            &HashMap::new(),
        );
        let names: Vec<_> = v.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(names, vec!["d", "high.png"]);
    }

    #[test]
    fn size_sort_descending() {
        let all = vec![
            img_entry("a", 10, 0),
            img_entry("b", 30, 0),
            img_entry("c", 20, 0),
        ];
        let v = sorted_filtered_view(
            &all,
            SortKey::Size,
            true,
            false,
            0,
            None,
            &HashMap::new(),
            &HashMap::new(),
        );
        let sizes: Vec<_> = v.iter().map(|e| e.size).collect();
        assert_eq!(sizes, vec![30, 20, 10]);
    }

    #[test]
    fn colors_sort_orders_by_count_unknowns_last() {
        let all = vec![
            img_entry("many.png", 1, 0),
            img_entry("few.png", 1, 0),
            img_entry("mid.png", 1, 0),
            img_entry("unknown.png", 1, 0), // no meta → unknown count
        ];
        let meta: HashMap<PathBuf, ImgMeta> =
            [("many.png", 256usize), ("few.png", 4), ("mid.png", 16)]
                .into_iter()
                .map(|(n, c)| {
                    (
                        PathBuf::from(n),
                        ImgMeta {
                            w: 1,
                            h: 1,
                            colors: Some(c),
                        },
                    )
                })
                .collect();
        // Ascending by count; the not-yet-decoded image sorts last.
        let v = sorted_filtered_view(
            &all,
            SortKey::Colors,
            false,
            true,
            0,
            None,
            &meta,
            &HashMap::new(),
        );
        let names: Vec<_> = v.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(names, vec!["few.png", "mid.png", "many.png", "unknown.png"]);
        // Descending flips the known counts but keeps unknowns last.
        let v = sorted_filtered_view(
            &all,
            SortKey::Colors,
            true,
            true,
            0,
            None,
            &meta,
            &HashMap::new(),
        );
        let names: Vec<_> = v.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(names, vec!["many.png", "mid.png", "few.png", "unknown.png"]);
    }

    #[test]
    fn name_filter_matches_substring_case_insensitively() {
        let all = vec![
            img_entry("Cat.png", 1, 0),
            img_entry("dog.png", 1, 0),
            dir_entry("catdir"),
        ];
        let v = sorted_filtered_view(
            &all,
            SortKey::Name,
            false,
            true,
            0,
            Some("cat"),
            &HashMap::new(),
            &HashMap::new(),
        );
        let names: Vec<_> = v.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(names, vec!["catdir", "Cat.png"]); // dir first; "dog" excluded
    }

    fn piece(artist: &str, group: &str, year: u32, pack: &str) -> ColoPiece {
        ColoPiece {
            artist: artist.into(),
            group: group.into(),
            year,
            pack: pack.into(),
            raw_url: String::new(),
            tn_url: String::new(),
            sauce: None,
        }
    }

    #[test]
    fn scene_sort_keys_use_the_piece_map() {
        // Three pieces with distinct artist/year, fed via the colo_pieces map.
        let all = vec![
            img_entry("c.ans", 0, 0),
            img_entry("a.ans", 0, 0),
            img_entry("b.ans", 0, 0),
        ];
        let mut pieces = HashMap::new();
        pieces.insert(PathBuf::from("c.ans"), piece("zinc", "acid", 1994, "p3"));
        pieces.insert(
            PathBuf::from("a.ans"),
            piece("aja", "blocktronics", 2002, "p1"),
        );
        pieces.insert(
            PathBuf::from("b.ans"),
            piece("mistress", "acid", 1991, "p2"),
        );

        // Artist ascending: aja, mistress, zinc.
        let v = sorted_filtered_view(
            &all,
            SortKey::Artist,
            false,
            false,
            0,
            None,
            &HashMap::new(),
            &pieces,
        );
        let names: Vec<_> = v.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(names, vec!["a.ans", "b.ans", "c.ans"]);

        // Year descending: 2002, 1994, 1991.
        let v = sorted_filtered_view(
            &all,
            SortKey::Year,
            true,
            false,
            0,
            None,
            &HashMap::new(),
            &pieces,
        );
        let years: Vec<_> = v.iter().map(|e| pieces[&e.path].year).collect::<Vec<_>>();
        assert_eq!(years, vec![2002, 1994, 1991]);
    }

    #[test]
    fn table_cell_text_scene_vs_file_columns() {
        let e = img_entry("MIDNACD3.ANS", 2048, 3);
        let p = piece("jed", "acid", 1992, "acdu0892");
        // Scene columns: filename / artist / type / year / group / pack.
        assert_eq!(
            table_cell_text(&e, None, Some(&p), ColKind::Name),
            "MIDNACD3.ANS"
        );
        assert_eq!(table_cell_text(&e, None, Some(&p), ColKind::Artist), "jed");
        assert_eq!(table_cell_text(&e, None, Some(&p), ColKind::Type), "ANS");
        assert_eq!(table_cell_text(&e, None, Some(&p), ColKind::Year), "1992");
        assert_eq!(table_cell_text(&e, None, Some(&p), ColKind::Group), "acid");
        assert_eq!(
            table_cell_text(&e, None, Some(&p), ColKind::Pack),
            "acdu0892"
        );
        // File columns: name / type / size / dims / colors.
        let meta = Some(ImgMeta {
            w: 320,
            h: 200,
            colors: Some(16),
        });
        assert_eq!(
            table_cell_text(&e, meta, None, ColKind::Name),
            "MIDNACD3.ANS"
        );
        assert_eq!(table_cell_text(&e, meta, None, ColKind::Type), "ANS");
        assert_eq!(table_cell_text(&e, meta, None, ColKind::Dims), "320×200");
        assert_eq!(table_cell_text(&e, meta, None, ColKind::Colors), "16");
        // A 0-byte entry (folder / virtual dir) hides its size instead of "0 B".
        let zero = img_entry("dir", 0, 0);
        assert_eq!(table_cell_text(&zero, None, None, ColKind::Size), "");
        assert!(!table_cell_text(&e, None, None, ColKind::Size).is_empty());
    }

    #[test]
    fn is_image_ext_recognizes_formats() {
        use std::path::Path;
        assert!(is_image_ext(Path::new("x.png")));
        assert!(is_image_ext(Path::new("X.PCX")));
        assert!(is_image_ext(Path::new("y.aseprite")));
        assert!(is_image_ext(Path::new("z.txt"))); // ASCII/ANSI art
        assert!(is_image_ext(Path::new("noext"))); // extensionless scene/BBS art
        assert!(!is_image_ext(Path::new("z.cpp"))); // genuinely non-art stays hidden
    }

    #[test]
    fn scan_folder_info_counts_and_pulls_previews_from_subdirs() {
        let base = std::env::temp_dir().join(format!("pv_scan_{}", std::process::id()));
        let sub = base.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("a.png"), b"").unwrap();
        std::fs::write(base.join("b.gif"), b"").unwrap();
        std::fs::write(base.join("source.cpp"), b"").unwrap(); // genuinely non-art
        std::fs::write(sub.join("c.png"), b"").unwrap(); // image one level down
        let info = scan_folder_info(&base);
        assert_eq!(info.images, 2, "non-art .cpp excluded");
        assert_eq!(info.subdirs, 1);
        // Fewer than 4 direct images → previews borrow from the subdir (request #4).
        assert!(info.previews.iter().any(|p| p.ends_with("c.png")));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn date_ymd_converts_known_timestamps() {
        use std::time::{Duration, UNIX_EPOCH};
        let at = |secs| date_ymd(UNIX_EPOCH + Duration::from_secs(secs));
        assert_eq!(at(0), "1970-01-01");
        assert_eq!(at(1609459200), "2021-01-01");
        assert_eq!(at(951782400), "2000-02-29"); // leap day
    }

    #[test]
    fn search_walk_filters_recursively_by_name_and_type() {
        let base = std::env::temp_dir().join(format!("pv_search_{}", std::process::id()));
        let sub = base.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(base.join("dragon.png"), b"").unwrap();
        std::fs::write(base.join("castle.gif"), b"").unwrap();
        std::fs::write(sub.join("dragon-deep.ans"), b"").unwrap(); // 2 levels down
        std::fs::write(sub.join("notes.txt"), b"").unwrap(); // not an image

        let run = |spec: SearchSpec| {
            let (tx, rx) = std::sync::mpsc::channel();
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let reg = Arc::new(Registry::with_builtins());
            search_walk(base.clone(), spec, reg, cancel, tx, Default::default());
            let mut names: Vec<String> = Vec::new();
            while let Ok(msg) = rx.recv() {
                match msg {
                    SearchMsg::Hit(e) => {
                        names.push(e.path.file_name().unwrap().to_string_lossy().into())
                    }
                    SearchMsg::Done(_) => break,
                }
            }
            names.sort();
            names
        };

        // Name "dragon" matches both pngs/ans across the tree (not castle, not txt).
        let n = run(SearchSpec {
            name: "dragon".into(),
            ..Default::default()
        });
        assert_eq!(n, vec!["dragon-deep.ans", "dragon.png"]);

        // Type filter: only .ans (recursively).
        let n = run(SearchSpec {
            ext: "ans".into(),
            ..Default::default()
        });
        assert_eq!(n, vec!["dragon-deep.ans"]);

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn scan_folder_info_surfaces_deeply_nested_previews() {
        // A top folder with NO direct images and only an empty intermediate dir, but
        // images three levels down, must still surface a montage (the deep-walk).
        let base = std::env::temp_dir().join(format!("pv_deep_{}", std::process::id()));
        let deep = base.join("empty").join("alsoempty").join("art");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("buried.png"), b"").unwrap();
        let info = scan_folder_info(&base);
        assert_eq!(info.images, 0, "no direct images");
        assert!(
            info.previews.iter().any(|p| p.ends_with("buried.png")),
            "preview pulled from 3 levels down"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn action_default_keys_round_trip_through_key_names() {
        // Persistence stores key.name() and reloads via Key::from_name.
        for a in Action::ALL {
            let k = a.default_key();
            assert_eq!(egui::Key::from_name(k.name()), Some(k), "{:?}", a.label());
        }
    }

    #[test]
    fn dedup_path_appends_copy_suffix() {
        let base = std::env::temp_dir().join(format!("pv_dedup_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        // A free name is used verbatim.
        assert_eq!(
            dedup_path(&base, OsStr::new("sprite.png")),
            base.join("sprite.png")
        );
        // Collisions step through " (copy)", " (copy 2)", … keeping the extension.
        std::fs::write(base.join("sprite.png"), b"x").unwrap();
        let p1 = dedup_path(&base, OsStr::new("sprite.png"));
        assert_eq!(p1, base.join("sprite (copy).png"));
        std::fs::write(&p1, b"x").unwrap();
        assert_eq!(
            dedup_path(&base, OsStr::new("sprite.png")),
            base.join("sprite (copy 2).png")
        );
        // Extensionless names too.
        std::fs::write(base.join("README"), b"x").unwrap();
        assert_eq!(
            dedup_path(&base, OsStr::new("README")),
            base.join("README (copy)")
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn copy_recursive_copies_dir_tree_and_keeps_source() {
        let base = std::env::temp_dir().join(format!("pv_copy_{}", std::process::id()));
        let src = base.join("src");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello").unwrap();
        std::fs::write(src.join("nested/b.txt"), b"world").unwrap();
        let dst = base.join("dst");
        copy_recursive(&src, &dst).unwrap();
        assert_eq!(std::fs::read(dst.join("a.txt")).unwrap(), b"hello");
        assert_eq!(std::fs::read(dst.join("nested/b.txt")).unwrap(), b"world");
        assert!(
            src.join("a.txt").exists(),
            "a copy leaves the source intact"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    #[ignore = "touches the real system trash; run explicitly with --ignored"]
    // `trash::os_limited` is configured out on macOS (see the guards in
    // `delete_selection`), so this freedesktop round-trip only builds elsewhere.
    #[cfg(not(target_os = "macos"))]
    fn trash_round_trips_on_this_system() {
        // Mirrors delete_selection -> undo: snapshot ids, trash, find the new item,
        // restore it. Verifies the user's actual freedesktop trash backend works.
        let base = std::env::temp_dir().join(format!("pv_trash_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let f = base.join("victim.png");
        std::fs::write(&f, b"bytes").unwrap();

        let before: std::collections::HashSet<OsString> = trash::os_limited::list()
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect();
        trash::delete(&f).unwrap();
        assert!(
            !f.exists(),
            "file left its original location after trashing"
        );

        let item = trash::os_limited::list()
            .unwrap()
            .into_iter()
            .find(|i| !before.contains(&i.id) && i.original_path() == f)
            .expect("trashed item is discoverable for undo");
        trash::os_limited::restore_all([item]).unwrap();
        assert!(f.exists(), "undo restored the file to its original path");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn to_gpl_emits_valid_header_and_rows() {
        let gpl = to_gpl("Hero", &[[255, 0, 0, 255], [0, 128, 64, 255]]);
        let lines: Vec<&str> = gpl.lines().collect();
        assert_eq!(lines[0], "GIMP Palette");
        assert_eq!(lines[1], "Name: Hero");
        assert_eq!(lines[2], "Columns: 16");
        assert_eq!(lines[3], "#");
        assert_eq!(lines[4], "255   0   0\t#FF0000");
        assert_eq!(lines[5], "  0 128  64\t#008040");
    }

    #[test]
    fn elide_truncates_with_ellipsis() {
        assert_eq!(elide("short.png", 20), "short.png");
        assert_eq!(elide("a_very_long_filename.png", 6), "a_ver…");
        assert_eq!(elide("xyz", 0), "");
    }

    #[test]
    fn caption_lines_respects_enabled_fields() {
        let e = img_entry("Hero.png", 2048, 3); // size 2048, rating 3
        let meta = Some(ImgMeta {
            w: 32,
            h: 48,
            colors: Some(12),
        });
        // Name + dimensions + colors enabled (rating off).
        let fields = CAP_NAME | CAP_DIMENSIONS | CAP_COLORS;
        let lines = caption_lines(&e, meta, fields, None);
        // Lines follow CAPTION_FIELDS order (colors before dimensions), not mask order.
        assert_eq!(
            lines,
            vec![
                "Hero.png".to_string(),
                "12 colors".to_string(),
                "32 × 48".to_string()
            ]
        );
        // Rating only, when rated, yields the stars; dimensions need meta.
        assert_eq!(
            caption_lines(&e, None, CAP_RATING, None),
            vec!["★★★".to_string()]
        );
        assert!(caption_lines(&e, None, CAP_DIMENSIONS, None).is_empty());
        // Search mode appends the folder line.
        assert_eq!(
            caption_lines(&e, None, CAP_NAME, Some("📁 sub")),
            vec!["Hero.png".to_string(), "📁 sub".to_string()]
        );
        // Folders always caption with just their name, ignoring the mask.
        let d = dir_entry("Sprites");
        assert_eq!(
            caption_lines(&d, None, 0, None),
            vec!["Sprites".to_string()]
        );
    }

    #[test]
    fn has_subdirs_only_true_with_nested_folders() {
        let base = std::env::temp_dir().join(format!("pv_hassub_{}", std::process::id()));
        let leaf = base.join("leaf");
        std::fs::create_dir_all(&leaf).unwrap();
        std::fs::write(base.join("a.png"), b"x").unwrap(); // a file, not a subdir
                                                           // `base` contains the subfolder "leaf" → true.
        assert!(has_subdirs(&base));
        // `leaf` has only files → false (no disclosure triangle in the tree).
        std::fs::write(leaf.join("x.png"), b"x").unwrap();
        std::fs::write(leaf.join("y.png"), b"x").unwrap();
        assert!(!has_subdirs(&leaf));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn adjust_pixels_behaves() {
        // Identity = no change.
        let mut px = vec![100, 120, 140, 255, 0, 0, 0, 0];
        let before = px.clone();
        adjust_pixels(&mut px, 2, 1, &Adjust::default());
        assert_eq!(px, before);
        // Brightness up brightens opaque pixels, leaves transparent alone.
        let a = Adjust {
            brightness: 0.5,
            ..Default::default()
        };
        adjust_pixels(&mut px, 2, 1, &a);
        assert!(px[0] > 100, "brightened");
        assert_eq!(&px[4..8], &[0, 0, 0, 0], "transparent untouched");
        // Posterize to 2 levels snaps each channel to 0 or 255.
        let mut px2 = vec![60, 60, 60, 255, 200, 200, 200, 255];
        let p = Adjust {
            posterize: 2.0,
            ..Default::default()
        };
        adjust_pixels(&mut px2, 2, 1, &p);
        assert_eq!(&px2[0..3], &[0, 0, 0], "low -> 0");
        assert_eq!(&px2[4..7], &[255, 255, 255], "high -> 255");
        // Array round-trip.
        assert_eq!(Adjust::from_array(a.to_array()), a);
    }

    #[test]
    fn invert_and_color_balance_behave() {
        // Full invert negates opaque channels; transparent pixels are untouched.
        let mut px = vec![10u8, 200, 50, 255, 1, 2, 3, 0];
        let inv = Adjust {
            invert: 1.0,
            ..Default::default()
        };
        adjust_pixels(&mut px, 2, 1, &inv);
        assert_eq!(&px[0..3], &[245, 55, 205]);
        assert_eq!(&px[4..8], &[1, 2, 3, 0], "transparent untouched");
        // Half invert lands halfway between the original and its negative.
        let mut g = vec![100u8, 100, 100, 255];
        let half = Adjust {
            invert: 0.5,
            ..Default::default()
        };
        adjust_pixels(&mut g, 1, 1, &half);
        assert_eq!(&g[0..3], &[128, 128, 128]); // 100*.5 + 155*.5 = 127.5 -> 128

        // Color balance adds a per-channel offset (clamped); neutral is a no-op.
        let mut c = vec![10u8, 250, 128, 255, 9, 9, 9, 0];
        color_balance(&mut c, [20, 20, -20]);
        assert_eq!(&c[0..3], &[30, 255, 108]);
        assert_eq!(&c[4..8], &[9, 9, 9, 0], "transparent untouched");
        let before = c.clone();
        color_balance(&mut c, [0, 0, 0]);
        assert_eq!(c, before, "neutral offset is a no-op");
    }

    #[test]
    fn parse_hex_handles_forms_and_rejects_junk() {
        assert_eq!(parse_hex("#1A2B3C"), Some([0x1A, 0x2B, 0x3C]));
        assert_eq!(parse_hex("1a2b3c"), Some([0x1a, 0x2b, 0x3c]));
        assert_eq!(parse_hex("  #abc "), Some([0xAA, 0xBB, 0xCC]));
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("12"), None, "wrong length");
        assert_eq!(parse_hex("#nothex"), None, "non-hex digits");
    }

    #[test]
    fn op_order_changes_result_and_persists() {
        // Posterize-then-brightness vs brightness-then-posterize give different
        // pixels — proof the pipeline honors `order`.
        let base = Adjust {
            brightness: 0.25,
            posterize: 3.0,
            ..Default::default()
        };
        let mut post_first = base;
        post_first.order = [
            OpKind::Posterize,
            OpKind::Brightness,
            OpKind::Contrast,
            OpKind::Gamma,
            OpKind::Shadows,
            OpKind::Highlights,
            OpKind::Hue,
            OpKind::Saturation,
            OpKind::Vibrance,
            OpKind::Pixelate,
            OpKind::Sharpen,
            OpKind::Invert,
            OpKind::ColorBalance,
            OpKind::Dither,
            OpKind::Palette,
        ];
        let mut bright_first = post_first;
        bright_first.order.swap(0, 1); // brightness now before posterize
        let src = vec![90u8, 90, 90, 255];
        let (mut a, mut b) = (src.clone(), src.clone());
        adjust_pixels(&mut a, 1, 1, &post_first);
        adjust_pixels(&mut b, 1, 1, &bright_first);
        assert_ne!(
            a, b,
            "reordering posterize vs brightness changes the result"
        );
        // Persisted order round-trips, and a garbage order falls back to a full set.
        assert_eq!(
            Adjust::default()
                .with_order(&post_first.order_to_u8())
                .order,
            post_first.order
        );
        assert_eq!(
            Adjust::default().with_order(&[99u8; 15]).order,
            Adjust::DEFAULT_ORDER,
            "unknown indices → default order, never a partial set"
        );
    }

    #[test]
    fn palette_op_position_changes_result() {
        // A remap that snaps every pixel to red=100; combined with +brightness.
        // Palette LAST: brightness first, then remap → exactly (100,0,0).
        let a = Adjust {
            brightness: 0.5,
            order: [
                OpKind::Brightness,
                OpKind::Contrast,
                OpKind::Gamma,
                OpKind::Shadows,
                OpKind::Highlights,
                OpKind::Posterize,
                OpKind::Hue,
                OpKind::Saturation,
                OpKind::Vibrance,
                OpKind::Pixelate,
                OpKind::Sharpen,
                OpKind::Invert,
                OpKind::ColorBalance,
                OpKind::Dither,
                OpKind::Palette,
            ],
            ..Default::default()
        };
        // A single-color palette so the Palette op snaps every pixel to (100,0,0);
        // dither off and balance neutral so only brightness + the snap matter.
        let pal = [[100u8, 0, 0, 255]];
        let aux = PipeAux {
            dither_method: 0,
            dither_amount: 0.0,
            dither_custom: &[],
            dither_n: 0,
            balance: [0, 0, 0],
            palette: Some(&pal),
        };
        let mut last = vec![10u8, 20, 30, 255];
        apply_pipeline(&mut last, 1, 1, &a, &aux);
        assert_eq!(&last[0..3], &[100, 0, 0], "remap last wins outright");
        // Palette FIRST: remap to red=100, THEN brightness lifts it above 100.
        let mut b = a;
        b.order.rotate_right(1); // Palette moves to the front
        let mut first = vec![10u8, 20, 30, 255];
        apply_pipeline(&mut first, 1, 1, &b, &aux);
        assert!(
            first[0] > 100,
            "brightness after remap lifts the red channel"
        );
        assert_ne!(last, first, "moving the palette op changes the result");
    }

    #[test]
    fn vibrance_pushes_muted_colors_and_spares_neutrals() {
        let v = Adjust {
            vibrance: 1.0,
            ..Default::default()
        };
        // A near-neutral gray (saturation ~0) must stay gray — no stray hue.
        let mut gray = vec![128, 128, 128, 255];
        adjust_pixels(&mut gray, 1, 1, &v);
        assert_eq!(
            &gray[0..3],
            &[128, 128, 128],
            "neutral untouched by vibrance"
        );
        // A muted color gains saturation: its channel spread (max-min) widens.
        let muted = [150u8, 120, 110];
        let spread0 = muted.iter().max().unwrap() - muted.iter().min().unwrap();
        let mut px = vec![muted[0], muted[1], muted[2], 255];
        adjust_pixels(&mut px, 1, 1, &v);
        let spread1 = px[0..3].iter().max().unwrap() - px[0..3].iter().min().unwrap();
        assert!(spread1 > spread0, "muted color got more saturated");
    }

    /// Visual smoke check (run with `--ignored`): apply each new adjustment to a
    /// real sprite and dump 8× nearest-scaled PNGs to /tmp/adj_*.png for eyeballing.
    #[test]
    #[ignore]
    fn dump_adjustment_previews() {
        let dir = std::path::Path::new(concat!(
            env!("HOME"),
            "/git/qb64pe-lab/greywood/sprites/ash_wolf"
        ));
        let Ok(rd) = std::fs::read_dir(dir) else {
            eprintln!("sprite dir missing, skipping");
            return;
        };
        let Some(src) = rd
            .flatten()
            .map(|e| e.path())
            .find(|p| p.to_string_lossy().contains("_sprite_"))
        else {
            return;
        };
        let img = image::open(&src).unwrap().to_rgba8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let base = img.into_raw();

        let cases: [(&str, Adjust); 6] = [
            (
                "hue",
                Adjust {
                    hue: 120.0 / 180.0,
                    ..Default::default()
                },
            ),
            (
                "saturation_up",
                Adjust {
                    saturation: 0.8,
                    ..Default::default()
                },
            ),
            (
                "saturation_gray",
                Adjust {
                    saturation: -1.0,
                    ..Default::default()
                },
            ),
            (
                "pixelate",
                Adjust {
                    pixelate: 4.0,
                    ..Default::default()
                },
            ),
            (
                "sharpen",
                Adjust {
                    sharpen: 0.8,
                    ..Default::default()
                },
            ),
            (
                "combo",
                Adjust {
                    hue: 0.3,
                    saturation: 0.4,
                    pixelate: 3.0,
                    sharpen: 0.5,
                    ..Default::default()
                },
            ),
        ];
        for (name, a) in cases {
            let mut px = base.clone();
            adjust_pixels(&mut px, w, h, &a);
            // 8× nearest upscale so the 32×48 sprite is legible in a viewer.
            const S: usize = 8;
            let mut up = vec![0u8; w * S * h * S * 4];
            for y in 0..h * S {
                for x in 0..w * S {
                    let si = ((y / S) * w + (x / S)) * 4;
                    let di = (y * w * S + x) * 4;
                    up[di..di + 4].copy_from_slice(&px[si..si + 4]);
                }
            }
            let out = format!("/tmp/adj_{name}.png");
            image::save_buffer(
                &out,
                &up,
                (w * S) as u32,
                (h * S) as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            eprintln!("wrote {out}");
        }
    }

    #[test]
    fn hsl_round_trips() {
        // Pure primaries and a gray survive RGB→HSL→RGB within rounding.
        for c in [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [128, 128, 128],
            [10, 200, 90],
        ] {
            let (h, s, l) = rgb_to_hsl(c[0], c[1], c[2]);
            let (r, g, b) = hsl_to_rgb(h, s, l);
            assert!(
                (r as i32 - c[0] as i32).abs() <= 1
                    && (g as i32 - c[1] as i32).abs() <= 1
                    && (b as i32 - c[2] as i32).abs() <= 1,
                "round-trip {c:?} -> {:?}",
                [r, g, b]
            );
        }
    }

    #[test]
    fn hue_and_saturation_transform() {
        // Hue +120° (slider 2/3) rotates red → green.
        let mut red = vec![255, 0, 0, 255];
        let a = Adjust {
            hue: 120.0 / 180.0,
            ..Default::default()
        };
        adjust_pixels(&mut red, 1, 1, &a);
        assert!(
            red[1] > red[0] && red[1] > red[2],
            "red rotated toward green: {red:?}"
        );
        // Saturation -1 → grayscale (all channels equal).
        let mut col = vec![200, 50, 90, 255];
        let g = Adjust {
            saturation: -1.0,
            ..Default::default()
        };
        adjust_pixels(&mut col, 1, 1, &g);
        assert!(
            col[0] == col[1] && col[1] == col[2],
            "desaturated to gray: {col:?}"
        );
    }

    #[test]
    fn pixelate_averages_blocks_and_keeps_alpha() {
        // A 2×2 image: one block of size 2 → all opaque pixels share the mean.
        // Pixels: (0,0,0) (60,60,60) / (120,120,120) (180,180,180); mean = 90.
        let mut img = vec![
            0, 0, 0, 255, 60, 60, 60, 255, // row 0
            120, 120, 120, 255, 180, 180, 180, 0, // row 1; last is transparent
        ];
        let a = Adjust {
            pixelate: 2.0,
            ..Default::default()
        };
        adjust_pixels(&mut img, 2, 2, &a);
        // Mean over the 3 opaque pixels = (0+60+120)/3 = 60.
        assert_eq!(&img[0..3], &[60, 60, 60], "opaque pixel got block mean");
        assert_eq!(img[15], 0, "transparent alpha preserved");
        assert_eq!(
            &img[12..15],
            &[180, 180, 180],
            "transparent pixel color untouched"
        );
    }

    #[test]
    fn sharpen_increases_local_contrast() {
        // 3×1 ramp; sharpening the middle pushes it away from its neighbours' mean.
        let mut img = vec![100, 100, 100, 255, 130, 130, 130, 255, 100, 100, 100, 255];
        let a = Adjust {
            sharpen: 1.0,
            ..Default::default()
        };
        adjust_pixels(&mut img, 3, 1, &a);
        // center = 130*(1+4) - 1*(100+130+130+100) = 650 - 460 = 190 (4-neighbour
        // cross with edge/clamp; top+bottom are out-of-bounds → clamp to center).
        assert!(img[4] > 130, "center brightened by sharpen: {}", img[4]);
    }

    #[test]
    fn random_index_and_palette_hash() {
        assert!(random_index(55) < 55);
        assert_eq!(random_index(0), 0);
        let a = vec![[1, 2, 3, 255], [4, 5, 6, 255]];
        let b = vec![[1, 2, 3, 255], [7, 5, 6, 255]];
        assert_eq!(palette_hash(&a), palette_hash(&a), "stable");
        assert_ne!(palette_hash(&a), palette_hash(&b), "content-sensitive");
    }

    #[test]
    fn reorder_favorites_moves_items() {
        // Drag the first onto the third slot: it lands just before that occupant.
        let mut v = vec!["A", "B", "C", "D"];
        reorder_favorites(&mut v, 0, 2);
        assert_eq!(v, vec!["B", "A", "C", "D"]);
        // Drag the last onto the second slot.
        let mut v = vec!["A", "B", "C", "D"];
        reorder_favorites(&mut v, 3, 1);
        assert_eq!(v, vec!["A", "D", "B", "C"]);
        // No-ops: same index, or out of range.
        let mut v = vec!["A", "B"];
        reorder_favorites(&mut v, 1, 1);
        reorder_favorites(&mut v, 5, 0);
        assert_eq!(v, vec!["A", "B"]);
    }

    #[test]
    fn move_path_relocates_and_removes_source() {
        let base = std::env::temp_dir().join(format!("pv_move_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let src = base.join("orig.txt");
        std::fs::write(&src, b"data").unwrap();
        let dst = base.join("moved.txt");
        move_path(&src, &dst).unwrap();
        assert!(!src.exists(), "the source is gone after a move");
        assert_eq!(std::fs::read(&dst).unwrap(), b"data");
        std::fs::remove_dir_all(&base).ok();
    }
}

/// Headless GUI tests via `egui_kittest` — drive the real `eframe::App` widget
/// tree (no window/compositor), the Rust-idiomatic alternative to screenshot QA.
#[cfg(test)]
mod gui_tests {
    use super::*;
    use egui_kittest::kittest::Queryable; // brings get_by_label onto Harness
    use egui_kittest::Harness;

    #[test]
    fn empty_state_renders_without_panic() {
        // No folder open → menu bar, toolbar, favorites, status all render the
        // empty state. Several frames must complete without panicking.
        let mut harness =
            Harness::builder().build_eframe(|cc| PixelView::new(cc, CliArgs::default()));
        harness.run_steps(3);
        // The menu bar's top-level items exist in the accessibility tree.
        harness.get_by_label("File");
        harness.get_by_label("Help");
    }

    #[test]
    fn file_menu_opens_and_shows_quit() {
        let mut harness =
            Harness::builder().build_eframe(|cc| PixelView::new(cc, CliArgs::default()));
        harness.run();
        harness.get_by_label("File").click();
        harness.run();
        // Opening File reveals its items.
        harness.get_by_label("Quit");
    }

    #[test]
    fn help_opens_keyboard_shortcuts_window() {
        let mut harness =
            Harness::builder().build_eframe(|cc| PixelView::new(cc, CliArgs::default()));
        harness.run();
        harness.get_by_label("Help").click();
        harness.run();
        harness.get_by_label("Keyboard shortcuts").click();
        harness.run();
        // A row from the shortcuts table is now visible inside the window.
        harness.get_by_label("Zoom the whole UI");
    }

    #[test]
    fn explorer_and_details_toggles_present() {
        let mut harness =
            Harness::builder().build_eframe(|cc| PixelView::new(cc, CliArgs::default()));
        harness.run();
        // The dock toggles moved from the toolbar to the View menu (and the rail).
        harness.get_by_label("View").click();
        harness.run();
        harness.get_by_label("Explorer pane");
        harness.get_by_label("Details pane");
    }

    #[test]
    fn table_view_toggle_present_and_switches_without_panic() {
        let mut harness =
            Harness::builder().build_eframe(|cc| PixelView::new(cc, CliArgs::default()));
        harness.run();
        harness.get_by_label("View").click();
        harness.run();
        // The new Table-view toggle is in the View menu; clicking it swaps the central
        // panel renderer (Mode stays Grid) — must render without panicking.
        harness.get_by_label("Table view").click();
        harness.run();
        harness.run();
    }

    #[test]
    fn view_menu_opens_preferences() {
        let mut harness =
            Harness::builder().build_eframe(|cc| PixelView::new(cc, CliArgs::default()));
        harness.run();
        harness.get_by_label("View").click();
        harness.run();
        harness.get_by_label("Preferences…").click();
        harness.run();
        // A control inside the Preferences window is visible.
        harness.get_by_label("Grid spacing (horizontal)");
        // The rebindable-hotkeys section lists the nav actions.
        harness.get_by_label("Previous image");
    }

    #[test]
    fn clicking_a_favorite_navigates_to_it() {
        // Regression: making favorites drag-reorderable once swallowed the click.
        // A favorite button must still navigate on a plain click.
        let dir = std::env::temp_dir().join(format!("pv_fav_click_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut harness =
            Harness::builder().build_eframe(|cc| PixelView::new(cc, CliArgs::default()));
        harness.state_mut().favorites.push(dir.clone());
        harness.run();
        // The toolbar favorite renders as "📁 <name>" (Places dock is hidden by
        // default, so there's exactly one such button).
        let label = format!("📁 {}", dir.file_name().unwrap().to_str().unwrap());
        harness.get_by_label(&label).click();
        harness.run();
        assert_eq!(harness.state().folder.as_deref(), Some(dir.as_path()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wheel_over_a_slider_adjusts_it() {
        // Drives a real pointer-move + mouse-wheel over a slider and asserts the
        // value changed (the feature the user reported as not working).
        struct S {
            val: usize,
            rect: egui::Rect,
        }
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, s: &mut S| {
                // Inside a (scrollable) ScrollArea, like the real details pane —
                // this is exactly where plain hovered() was reading false.
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let resp = ui.add(egui::Slider::new(&mut s.val, 2..=256));
                    s.rect = resp.rect;
                    wheel_adjust(ui, &resp, &mut s.val, 1.0, 2usize, 256usize);
                    ui.add_space(2000.0); // make the area actually scrollable
                });
            },
            S {
                val: 16,
                rect: egui::Rect::ZERO,
            },
        );
        harness.run();
        let center = harness.state().rect.center();
        harness.event(egui::Event::PointerMoved(center));
        harness.run();
        let before = harness.state().val;
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::default(),
        });
        harness.run();
        assert_ne!(
            harness.state().val,
            before,
            "wheel over the slider must change its value"
        );
    }

    #[test]
    fn middle_click_resets_a_slider() {
        // Middle-clicking a slider restores its default. DEF is off-center (-1.0)
        // while the click lands at the track center (value 0.0), so a passing
        // assert proves the *reset* fired, not an accidental slider drag.
        struct S {
            val: f32,
            rect: egui::Rect,
        }
        const DEF: f32 = -1.0;
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, s: &mut S| {
                // In a ScrollArea, where the press-frame hover state lags — the
                // position hit-test in middle_reset must still fire.
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let resp = ui.add(egui::Slider::new(&mut s.val, -1.0..=1.0));
                    s.rect = resp.rect;
                    middle_reset(ui, &resp, &mut s.val, DEF);
                    ui.add_space(2000.0);
                });
            },
            S {
                val: 0.75,
                rect: egui::Rect::ZERO,
            },
        );
        harness.run();
        let center = harness.state().rect.center();
        harness.event(egui::Event::PointerMoved(center));
        harness.run();
        let before = harness.state().val;
        // A real middle-click is press + release; without the release the button
        // stays "down" and the slider keeps drag-tracking the pointer.
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: center,
                button: egui::PointerButton::Middle,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        harness.run();
        assert_eq!(before, 0.75, "slider untouched before the middle-click");
        assert_eq!(
            harness.state().val,
            DEF,
            "middle-click resets the slider to its default"
        );
    }

    #[test]
    fn wheel_over_a_combo_changes_selection() {
        struct S {
            idx: usize,
            rect: egui::Rect,
        }
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, s: &mut S| {
                let names = ["a", "b", "c", "d"];
                let cr = egui::ComboBox::from_id_salt("t")
                    .selected_text(names[s.idx])
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            ui.selectable_value(&mut s.idx, i, *n);
                        }
                    });
                s.rect = cr.response.rect;
                wheel_cycle(ui, &cr.response, &mut s.idx, names.len());
            },
            S {
                idx: 1,
                rect: egui::Rect::ZERO,
            },
        );
        harness.run();
        let center = harness.state().rect.center();
        harness.event(egui::Event::PointerMoved(center));
        harness.run();
        let before = harness.state().idx;
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::default(),
        });
        harness.run();
        assert_ne!(
            harness.state().idx,
            before,
            "wheel over the combo must change the selection"
        );
    }
}

#[cfg(test)]
mod hold_test {
    use super::*;
    #[test]
    fn flash_holds_while_button_down() {
        struct S {
            flash: Option<u8>,
            rect: egui::Rect,
        }
        let mut h = egui_kittest::Harness::new_ui_state(
            |ui, s: &mut S| {
                let (r, resp) = ui.allocate_exact_size(egui::vec2(16., 16.), egui::Sense::click());
                s.rect = r;
                s.flash = if resp.is_pointer_button_down_on() {
                    Some(7)
                } else {
                    None
                };
                ui.painter().rect_filled(r, 0.0, egui::Color32::RED);
            },
            S {
                flash: None,
                rect: egui::Rect::ZERO,
            },
        );
        h.run();
        let c = h.state().rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run();
        let down = |p, pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        h.event(down(c, true));
        h.run();
        assert_eq!(h.state().flash, Some(7), "pressed -> flashing");
        h.run();
        assert_eq!(h.state().flash, Some(7), "still held across a frame");
        h.event(down(c, false));
        h.run();
        assert_eq!(h.state().flash, None, "released -> normal");
    }
}
