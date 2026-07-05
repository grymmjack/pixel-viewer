# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`pixelview` — a pixel-art-first image **browser** in Rust + eframe/egui, grown
from a scaffold into a Gwenview-like tool. Single binary crate named `pixelview`.
Features: folder navigation (breadcrumbs, drag-reorderable favorites, a left
activity rail, an explorer dock with an expandable folder tree + filter, and a
live details dock with a fit thumbnail + palette swatches + `.GPL` export), a
virtualized thumbnail grid with independent Ctrl+wheel sizing and configurable
per-tile captions **or a sortable table view** (Grid/Table toggle in the sortbar +
View menu, persisted; click a header to sort), a nearest-neighbor zoom view (persistent zoom), per-image
metadata + hover details, multi-select, star ratings stored as KDE Baloo xattrs,
recursive folder montages + count badges, sort/filter, filename search (`/`),
file operations (copy/cut/paste, new folder, rename, trash + Ctrl+Z undo via the
`trash` crate; context menu / Edit menu / shortcuts), a menu bar with hotkeys +
Preferences (Dark/Light theme, grid spacing, caption fields) dialogs, a custom app
icon, plus persisted settings and CLI flags.
Decodes png/gif/bmp/jpeg/webp/tga/tiff/pnm/qoi/**ico** (image crate), **PCX**,
**Aseprite** (`.aseprite`/`.ase`), **PSD**, GIMP **XCF**, **.draw** (PNG preview),
**SVG** (resvg), **ANSI/ASCII art** (`.ans`/`.asc`/`.nfo`/`.diz`/`.ice`/`.cia`,
embedded CP437 VGA font), and the binary scene formats **XBin** (`.xb`/`.xbin`),
**raw BIN** (`.bin`), **TundraDraw** (`.tnd`, 24-bit), **iCE Draw** (`.idf`),
**Artworx** (`.adf`), **PETSCII** (`.seq`/`.pet` — Commodore C64), **petmate** (`.petmate`
— nurpax/petmate JSON PETSCII) and **RIPscript**
(`.rip` — EGA vector BBS graphics), both via Mike Krüger's `icy_parser_core` — all with
SAUCE-driven hints, shown in the Details pane. Also **source code + text** (~90 exts: rs,
c/cpp/h, py, js/ts, css, html, php, lua, asm, gd, json, yaml, md, log, … — rasterized with
the CP437 font + a lean hand-rolled syntax highlighter, `decode/code.rs`), **PDF**
(`decode/pdf.rs`: the tile is the **real first page** rendered via poppler's `pdftoppm` —
PDF on stdin, PNG on stdout — falling back to a labeled placeholder if poppler is absent;
page-count/size/title/author metadata via pure-Rust `lopdf`), and **audio**
(`decode/audio.rs`: a real waveform tile for mp3/wav/ogg/flac via `symphonia`, else a
music-note icon for trackers/voc/au/midi; duration/rate/channels/codec in Details, plus an
**in-app play/pause/seek preview** via `rodio`). These three are **toggleable plugins**
(Preferences → "Format plugins" checkboxes; a runtime atomic flag on the `Registry` — off
drops the type from the listing + skips decoding). **Any** file also gets "Open in default
app" (xdg-open/open/explorer) in the right-click "Open in…" menu, the Details pane, and via
**Enter** in the viewer — so a source file drops into its associated editor.
**Animated GIFs** play (autoplay + seek in the viewer,
hover-to-play in the grid). Archives (`.zip`/`.lha`/`.arj`/…) and **16colo.rs** (the
online ANSI archive: a Places entry with a nav bar — Years / Latest / Groups / Artists
+ a server-side `?filter=` Search) are browsable as virtual folders. A **Year** lists
Packs (→ auto-downloaded pack art); **Artist / Group / Search** instead flatten to a
**sortable table of individual pieces** (thumb · filename · artist · type · year · group ·
pack + a per-row "download file / pack" menu), fetched from the JSON API with no pack
download — opening a piece grabs just its single `raw` file. Keys: 1–5/0 rate, Esc→grid, Backspace→up — the nav keys
are rebindable (Preferences → Hotkeys); in the viewer, Ctrl+wheel or hold-Z + 1-9/0
zooms (Snap locks to 100% steps).

## Project layout

Standard Cargo binary-crate layout (`Cargo.toml` has no `path` override):

```
src/
  main.rs            eframe entry / window setup
  app.rs             PixelView: the whole UI — panels, grid/single views, model,
                     settings persistence, sort/filter, ratings, CLI parsing
  image_types.rs     PixImage (RGBA + optional indexed/palette)
  cache.rs           persistent HTTP cache for 16colo (JSON/thumbnails/files/zips):
                     blob files under <data>/cache/ + a SQLite index (cache.db) for
                     TTL freshness, LRU eviction (2 GiB cap) + size stats; thread-safe
                     via a global Mutex<Connection>. `init`/`get_bytes`/`get_file`/
                     `stats`/`clear`. Used by sixteen.rs (get_json/download) + colo_thumb.
  thumb.rs           worker pool: thumbnails + image metadata (dims, color count)
  colo_thumb.rs      RemoteThumbs: HTTP worker pool fetching 16colo.rs `tn` PNGs
                     (mirrors ThumbBuilder; results uploaded to thumb_tex by path)
                     Busy feedback: `net_busy()` (any *_rx in flight + colo_sauce_pending)
                     drives a status-bar `egui::Spinner`; grid/table tiles paint a
                     `paint_spinner` arc while a thumbnail loads; the empty grid/table
                     and the SAUCE panel ("fetching…") show one too.
  rating.rs          read/write star ratings via the user.baloo.rating xattr
  ratings.rs         cross-platform ratings sidecar (ratings.json) for virtual art
  anim.rs            decode animated GIF frames + per-frame delays
  soundfont.rs       .sf2 as a virtual folder: rustysynth parse → each sample extracted
                     to a WAV in a temp dir (mounted like an archive) + preset/instr counts
  sfz.rs             .sfz as a virtual folder: parse opcodes → symlink referenced samples
                     into a temp dir; region/sample/key-range info
  dls.rs             .dls as a virtual folder: walk the RIFF wave pool → rewrap each
                     embedded wave into a standalone WAV; instrument/sample counts
  xi.rs              .xi (FastTracker II instrument) as a virtual folder: xmrs parse →
                     each sample extracted to a WAV; name/sample count
                     (.xrns/.xrni are ZIPs → handled by archive.rs's force-zip path)
  libxmp.rs          FFI to bundled libxmp (vendor/libxmp, built by build.rs) — plays the
                     tracker formats xmrs doesn't (669/far/okt/med/amf/ult/mtm/stm)
  format_color.rs    per-format tile/waveform/badge accent colors — a process-global ext→RGB
                     map (Preferences-editable, persisted), read by the grid AND thumbnailer
build.rs             compiles vendor/libxmp (C) into a static lib via the cc crate
vendor/libxmp/       vendored libxmp 4.6.3 source (MIT) — src/ + include/ + libxmp-sources.cmake
  decode/
    opl3.rs          OPL3 FM-synth chip emulator (public-domain "Opal" port) — for .rad
    rad.rs           Reality Adlib Tracker replayer (public-domain RADPlayer port) — .rad
    mod.rs           Decoder trait + Registry (sniff-then-extension dispatch)
    builtin.rs       image-crate decoder: png/gif/bmp/jpeg/webp/tga/tiff/pnm/qoi/ico/draw
    pcx.rs           hand-written, palette-preserving PCX decoder
    aseprite.rs      .aseprite/.ase via the asefile crate (composited frame 0)
    psd.rs           .psd via the psd crate (flattened composite)
    xcf.rs           GIMP .xcf — composites layers (xcf crate; offsets not applied)
    svg.rs           .svg rasterized via resvg/usvg/tiny-skia
    ansi.rs          .ans/.asc/.nfo/.diz/.ice/.cia — CP437 + ANSI SGR/cursor + iCE
                     + SAUCE-driven 8×8 (VGA50/EGA43) vs 8×16 cell selection;
                     optional 9-dot VGA cell (font_9px); pads to a ≥25-row screen;
                     TextStream renders byte prefixes for baud-rate ANSImation playback
    xbin.rs          .xb/.xbin — binary ANSI: palette/font + RLE; shared render_textmode;
                     default palette is ansi::VGA_PALETTE (raw VGA attr order, not SGR)
    bin.rs           .bin — raw char/attr pairs (SAUCE width); idf/adf reuse render_textmode
    code.rs          source code / text (~90 exts) → CP437 8x16 raster + a lean hand-rolled
                     highlighter (per-language comment/string rules, shared keyword union,
                     line-number gutter, tab expand, UTF-8→CP437, line+cell budget). `CODE_EXTS`
                     re-exported; registry routes code exts to `decode_ext(bytes, ext)`. ipynb
                     flattens to highlighted Python. Zero heavy deps (no syntect).
    pdf.rs           .pdf — the tile is the REAL first page via `render_first_page` (poppler
                     `pdftoppm`: PDF→stdin, PNG→stdout, decoded by the image crate; stdin fed
                     from a thread to avoid pipe deadlock), else a labeled placeholder. Metadata
                     (page count / MediaBox size / /Info title+author) via `lopdf`; `pdf_meta`/
                     `PdfMeta` feed the Details pane.
    audio.rs         audio → waveform tile (mp3/wav/ogg/flac via `symphonia`: decode → peak
                     envelope → resample → mirror) else a music-note icon (trackers/voc/au/midi).
                     `audio_info`/`AudioInfo` (duration/rate/channels/codec) feed Details; the
                     DECODE path is device-free so `cargo test` stays headless. `AUDIO_EXTS`
                     re-exported; registry routes audio exts to `decode_ext` (needs the ext hint).
                     In-app PLAYBACK is separate (`AudioPlayer` in app.rs, rodio) — see below.
    tundra.rs        .tnd — TundraDraw 24-bit truecolor command stream
    idf.rs           .idf — iCE Draw: bounds + RLE + end-of-file font/palette
    adf.rs           .adf — Artworx: version + 64-color palette + font + pairs
    petscii.rs       .seq/.pet — Commodore PETSCII: icy_parser_core parses → our render
    petmate.rs       .petmate — nurpax/petmate JSON PETSCII (screens of {code,color}
                     cells) → rendered with the same C64 font + VIC-II palette, stacked
    c64_font.rs      embedded C64 8x8 char ROM (MEGA65 open-roms, LGPL; for PETSCII)
    rip.rs           .rip — RIPscript: icy_parser_core parses → our 640×350 EGA rasterizer;
                     RipStream renders byte prefixes for "watch it draw" baud playback
    rip_chr.rs       RIP scalable text: the 10 BGI stroke fonts (rip_chr/*.CHR) as lines
    cp437_font.rs    embedded CP437 8x16 VGA font (generated from a system PSF)
    cp437_font_8x8.rs  embedded CP437 8x8 VGA50 font (IBM ROM, for 80×50 ANSI)
  palettes_builtin.rs  the bundled .GPL palette library, embedded via include_str!
assets/pixelview.png   generated app icon (4×4 thumbnail grid)
assets/palettes/       55 bundled .GPL palettes (embedded into the binary)
assets/DejaVuSans.ttf  embedded UI fallback font (fills egui's tofu gaps; see Font gotcha)
assets/SymbolsNerdFont-Regular.ttf  embedded icon font (the `icons::*` designed glyphs)
pixelview.desktop      desktop entry (StartupWMClass=pixelview) for the task icon
install-icon.sh        installs the .desktop + icon into ~/.local/share
```

Palettes are **embedded** (`src/palettes_builtin.rs` `include_str!`s every file in
`assets/palettes/`), so the palette library works on any platform with no external
directory. The optional `palette_dir` (persisted; defaults to a Linux path that
simply scans empty elsewhere) only *adds* user `.gpl` files on top of the bundled
set — see `all_palettes` / `builtin_palette_contents` in `app.rs`. To add a bundled
palette, drop a `.GPL` in `assets/palettes/` and add one `include_str!` line.

Note: `Cargo.lock` **is committed** (the monorepo `.gitignore`'s `**/Cargo.lock`
rule was removed for these binary crates). It pins `eframe`/`egui` to 0.34.3 — the
range `eframe = "0.34"` would otherwise let a fresh checkout drift to a newer patch
with a moved API (see the egui gotcha below — that's what bit the initial scaffold).

## Commands

```sh
cargo run --release      # build + launch (release; nearest-neighbor needs the GPU path)
cargo build --release
cargo check              # fast type-check during edits
cargo clippy             # lint
cargo fmt                # format
cargo test               # 198 tests (188 unit + 10 headless egui_kittest GUI tests; +11 ignored network/real-trash)
cargo test gui_tests     # just the egui_kittest UI tests; cargo test <name> for one
```

First-time eframe/winit system deps on Debian/KDE:
```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev libasound2-dev
```

## Architecture: the big picture

Three subsystems wired together in `PixelView::new` (`app.rs`):

**1. Decoder registry (`decode/mod.rs`)** — the entire extension story. `Registry`
holds `Vec<Box<dyn Decoder>>`. Dispatch in `decode_bytes` is **two-tier**: every
decoder's `sniff()` (magic bytes of the first ≤32 bytes) is tried first, then file
extension is the fallback. **Order matters**: `PcxDecoder` is registered before
`ImageCrateDecoder` in `with_builtins`, so PCX's magic byte (`0x0A`) wins before
the broad `image`-crate sniff. `known_extension()` is what filters a folder
listing down to viewable files.

**2. Threaded thumbnailer (`thumb.rs`)** — `ThumbBuilder` spawns N worker threads
(N = `available_parallelism`) sharing an `Arc<(Mutex<Vec<Job>>, Condvar)>` used as
a **LIFO stack**, so the most-recently-requested (i.e. just-scrolled-into-view)
thumbnail is decoded first. Results return over an `mpsc::channel`. `request()`
dedupes via a `requested: HashSet`, so the grid can cheaply call it every frame
for visible tiles. Workers do decode + nearest-neighbor scale (`make_thumb`)
entirely off the UI thread; **only CPU RGBA buffers cross back** — never GPU
handles.

**3. UI / eframe::App (`app.rs`)** — `ui()` (the eframe 0.34.3 entry; see gotcha)
`drain()`s finished thumbs and uploads them to GPU textures (`ctx.load_texture`,
`TextureOptions::NEAREST`) — **texture upload must happen on the UI thread**, which
is why workers stop at raw pixels. The chrome is a stack of `show_inside` panels:
`menubar` (top) → `rail` (left, a VSCode-style activity bar of icon toggles:
`ui_rail`) → `favorites` (top) → `crumbs`/path (top, under favorites) → optional
`searchbar` (top, vim `/` filename filter) → optional `advsearch` (top, the advanced
recursive search — see below) → `status` (bottom) → `sortbar` (bottom, mounted *after* status
so it sits above it) → optional left dock (`leftdock`: `details` on top, `explorer`
below) / `recolor` (right) → `CentralPanel` (grid or single view, by the `Mode` enum). The status bar reserves
its flush-right zoom readout first, then fills the rest with a truncating left
label, so they can't overlap. The grid is virtualized via `ScrollArea::show_rows`;
each tile is a `tile`-square thumbnail plus a configurable caption strip below
(`caption_lines`, `caption_fields` bitmask; independent `grid_gap`/`grid_gap_y`).
`want_repaint` drives `ctx.request_repaint()` while thumbnails are pending.

File ops (`copy_selection`/`cut_selection`/`paste`/`new_folder`/`start_rename`/
`delete_selection`/`undo`, dispatched via `FileAction`/`do_file_action`) go to the
system trash (`trash` crate) and push reversible `UndoOp`s for Ctrl+Z; they set
`self.status` **after** `refresh()` (which re-scans via `open_folder`, clearing
stale status). Favorites are drag-reorderable (`favorites_buttons` + a unioned
drag sensor, *not* `dnd_drag_source` — its scope breaks `horizontal_wrapped`) and
right-click-removable in both the toolbar and the Places dock. Each favorite/pin can
be **color-tagged** from its right-click menu (a popup grid of the bundled **ANSI32**
swatches via `ansi32_palette`, + ✕ to clear); the chosen color fills the button (text
flipped to black/white by `contrast_text`). Stored in `fav_colors` (path → RGB,
persisted as `FAV_COLORS_KEY`); cleared when the favorite is removed. In the Places
dock the favorites split into **Local** vs **16colo.rs** sub-tabs (`places_tab`):
`favorites_buttons` takes a `filter: Fn(&Path) -> bool` (`!is_remote` / `is_remote`) and
filters with `continue` so the *global* favorite index — and thus reorder/remove —
stays correct. The **top toolbar** passes a filter that, when `fav_bar_colored_only`
(persisted, default on; View → "Favorites bar: colored only") is set, surfaces **only
color-tagged favorites** to declutter the bar — with a fallback to all when *none* are
colored (never an empty bar), and a faint `+N` marking how many uncolored ones are hidden
in the dock. Local also holds Home + smart
filters; 16colo.rs holds the 🌐 browse entry + the remote pins. The explorer's
folder tree uses `CollapsingState` (lazy: collapsed nodes do no I/O). The details
dock shows a fit thumbnail + palette swatches + `.GPL` export (`to_gpl`), fed by
the thumbnailer's `extract_palette` (authoritative palette for indexed art).

**4. The app model (`app.rs`)** — `open_folder` scans into `all_entries` (raw
`Entry { path, is_dir, size, mtime, ctime, rating }`); `rebuild_view()` filters (by
`min_rating`) and sorts (by `SortKey`, dirs-first optional) into `entries`, the list
the grid renders. **`selection` is keyed by `PathBuf`, not index**, so it survives
re-sorting/filtering. The caches `thumb_tex` / `img_meta` / `folder_previews` are
**persistent path-keyed maps, never cleared on navigation** — clearing `thumb_tex`
while the worker's `requested` set persisted once caused black tiles (and persisting
makes back-nav instant). Because an egui closure can't borrow `self` twice, menu and
combo handlers stash a deferred action (`MenuAction`, or locals like `clicked`/`nav`)
and apply it *after* the closure returns.

**Advanced recursive search (`ui_search`, Ctrl+F / Edit menu)** — a `SearchSpec`
(filename / extension list / W·H min-max / **size KB min-max** / **modified-date
range** / **min ★** / SAUCE text; all optional) drives `search_walk`, which BFS-walks
the current folder's subtree on a **background thread** (mirrors the 16colo `mpsc`
pattern: `search_rx` + an `AtomicBool` cancel, `Send`ing a `SearchMsg::Hit(Entry)` per
match then `Done(n)`). Filters run cheapest-first: name/type (path strings) → size/date
(one `metadata` read) → rating (xattr; sidecar-only ratings on archive/16colo art are
invisible to the thread) → SAUCE (`read_file_tail` + parse) → dimensions (`quick_dims`:
a header-only `image::image_dimensions`, full-decode fallback) — each only when its
field is set, and a cheap reject skips the costly ones. Dates use `date_ymd` (mtime →
`YYYY-MM-DD` via `civil_from_days`, no date crate; the format sorts as text so range
checks are string compares). `poll_search` drains
hits each frame into `search_results`, resolving each star via `read_rating`. When
`search_results.is_some()` **`rebuild_view` renders it instead of `all_entries`** — so
the grid, thumbnailer, sort/filter and open-in-viewer all work on results for free. Any
navigation (`show_folder`) cancels the search and drops the results. Colors filtering is
deferred (needs a full decode of every candidate). Result tiles get an extra caption
line — the match's folder relative to `search_root` (`result_folder_label`; `📁 ·` =
directly in the root), so you can see *where* a hit lives.

**Smart filters (saved searches)** — `save_filter` stores the current `SearchSpec`
under its `summary()` label (e.g. `*.ans · sauce:acid`) in `saved_filters`, persisted
as `Vec<Vec<String>>` (`[name, ...record]`; no serde-derive on `SearchSpec` — it
flattens via `record`/`from_record`). They render in the Places dock under "Smart
filters" with a 🔍 prefix; click → `recall_filter` (load spec + `start_search` from the
*current* folder), right-click → remove. Both deferred out of the `ui_explorer` closure
(`recall` / `remove_filter` locals applied after) since it can't borrow `self` twice.

**Grid context menu extras** — folder tiles get **📌 Pin to Places** (`pin_dir`
deferred → `favorites.push`). File tiles get a **🔍 Smart filter on…** submenu
(`SmartCriterion`) that seeds a fresh `SearchSpec` from that one file and runs it
(`smart_filter_from`): Type=its ext, File name=first ≥3-char word of the stem, File
size=±20% KB, Date=its mtime day, Rating=its stars+ (shown only when rated), SAUCE
group/artist=its SAUCE field (shown only for `is_textmode_ext`). Both are deferred
`pin_dir`/`smart_on` locals applied after the tile closure. **In a 16colo flat listing
(`colo_flat`) the rows are *pieces*, not dirs**, so `entry_context_menu` also takes a
`colo_pin: Option<(&str, bool)>` and offers **📌 Pin “<artist/group/search>” to Places**
(`TilePick::PinFolder` → `pin_current` → `pin_current_folder` pins `self.folder`). Since
the listing's virtual path encodes the search (`…/search/artist/x`), the pinned favorite
re-runs that exact artist/group/search when clicked — the way to bookmark an artist.

**External "Open in…" associations (`Opener`, `openers`, editor `ui_associations`)** — a
file tile's context menu has an **Open in…** submenu listing the user's programs registered
for that extension (+ "Other program…" → an rfd one-off). An `Opener` is `{name, exec, args,
env, icon, exts}` (`exts` comma/space-separated; `ext_list()` normalizes), persisted as a
flat `Vec<Vec<String>>` (`record`/`from_record`, no serde-derive — like `SearchSpec`).
`launch_external` spawns `Command::new(exec)` with `args` (a `{}` token → the **`resolve_local`d
real** file path, else appended — so virtual 16colo/archive art opens from its on-disk copy)
and `env` (`KEY=VALUE` lines). `open_external_for` routes both paths: a 16colo flat-listing
piece **not yet downloaded** kicks off `start_piece_open` and stashes `(exec,args,env)` in
`pending_external`, which `poll_colo_open` consumes to launch the program once the real file
lands (instead of opening the viewer); everything else launches immediately. `entry_context_menu` is a free fn so it can't borrow `self`; it
takes an owned `&[OpenerItem]` snapshot (`opener_items`, built inside the menu closure) and
filters by the entry's extension; the pick (`TilePick::OpenWith(idx)` / `OpenWithOther`) is a
deferred `open_with`/`open_other` local applied after the grid/table loop. Icons decode via
`ensure_opener_icons` (registry → 32px texture, cached by path in `opener_icons`; `None` on
failure so it doesn't re-decode). The editor (View → Associations…, `show_associations`) is a
two-pane window — a left list + right field editor on the `assoc_selected` opener, with
add/`opener_presets()`/remove deferred; it precomputes an owned `list` of (name, icon) so the
list pane doesn't borrow `openers` while the editor `get_mut`s the selected one.

**Table view (`ui_table`, `table_view` bool, persisted)** — an *alternate renderer*
for the browse mode, **not a third `Mode`**: `Mode` stays `Grid` and the central panel
picks `ui_table` vs `ui_grid` by `self.table_view`, so selection (`PathBuf`-keyed),
ratings, keyboard nav, and context menus (`entry_context_menu`, shared with the grid)
all keep working unchanged. Hand-rolled (no `egui_extras`) on the same virtualized
`ScrollArea::show_rows` the grid uses: a fixed-width header row above the body (click a
header → set `SortKey`, click again → reverse), then one row per `Entry` laid out cell
by cell with `item_spacing = 0` so header/body columns align. Two column sets: **file**
columns (name/type/size/dims/colors/rating/modified) for any folder, and **scene**
columns (filename/artist/type/year/group/pack + a ⬇ download menu) when `colo_flat`.
Sorting routes through the same `sorted_filtered_view`, which now also takes the
`colo_pieces` map for the scene `SortKey`s (`Artist/Group/Year/Pack` — in `SortKey::ALL`
for persistence but excluded from `SortKey::COMMON`, the sortbar combo). `Dimensions`
sorts like `Colors` (unknown-last in both directions). The toggle is also the rebindable
`Action::ToggleView` (default **`T`**, browse-mode only so it never clashes with `T` =
tile in the single view), shown in Preferences → Hotkeys + the Help window. Each cell
has `cell_pad` breathing room and clips its text to the cell, so columns never touch.
Rows always paint a **zebra stripe** (odd rows); the optional **`table_grid`** toggle
(Preferences → "Table dividing lines", persisted) additionally draws a subtle translucent
**bottom row separator + interior column dividers** per row (painted over the cells).
Columns key off a `ColKind` (not a position), so the **file** layout's optional columns
are user-toggled via a `TC_*` bitmask (`table_columns`, persisted; Preferences → "Table
columns" *and* the header right-click menu) and the **scene** layout's via a parallel
`CS_*` bitmask (`colo_columns`); Name + thumbnail are always shown. **Header UX:**
left-click a header sorts (re-click reverses); **right-click** → Sort ascending/descending
+ a "Show columns" checklist (toggles the layout's bitmask, no Preferences trip); a thin
border at each fixed column's right edge **drag-resizes** it (`col_widths`: ColKind →
points, persisted; flex columns absorb the slack); and **dragging a header body reorders**
the columns (a vertical drop indicator follows the pointer; the new order persists as
`table_order`/`colo_order` — a `Vec<u8>` of ColKind, applied by sorting the built `cols`,
unknown/new kinds appended). Thumbnail stays first, the scene Download menu last; click vs
drag vs border-drag disambiguate by sense (the cell is `click_and_drag`, the border its own
`drag` widget). Archive rows (.zip/…) render the folder glyph + a format badge like the grid. In the scene layout the **Pack / Year /
Group** cells are clickable links into the 16colo browser (`colo_link` deferred →
`open_folder` of the pack / year / `groups/<group>` path; the link click takes priority
over the row's open-the-art click).

**16colo.rs flat-piece listings (`ColoSource`, `colo_walk`, `start_colo_pieces`)** —
Artist / Group / Search no longer list pack *folders*; they stream individual **pieces**
into the table (the requested flat view), keyed by virtual path `<16colo.rs>/<year>/
<pack>/<FILE>`. `colo_walk` (a background thread mirroring `search_walk`: `colo_rx` +
`AtomicBool` cancel) emits `ColoMsg::Hit(Entry, ColoPiece)` then `Done(n)`; `poll_colo_pieces`
drains them, resolves each rating, appends to `all_entries` + the `colo_pieces` map, and
re-sorts. Pieces come from the JSON API with **no pack download**: an artist = one call
(`fetch_artist_pieces`), a group = per-pack (`fetch_group_pack_refs` → `fetch_pack_pieces`),
a search = matched artists + groups (capped). The nav-bar search is **facet-scoped**:
on the Artists tab it builds `SEARCH/artist/<q>` → `ColoSource::SearchArtists` (artist
names only), on Groups `SEARCH/group/<q>` → `SearchGroups`, otherwise `SEARCH/<q>` →
`Search` (both) — so "tainted" under Artists no longer drags in the *group* "tainted".
`do_artists`/`do_groups` closures in `colo_walk` are shared by all three. Thumbnails are
16colo's pre-rendered render PNGs fetched by the `RemoteThumbs` HTTP pool (`colo_thumb.rs`)
and uploaded to `thumb_tex` by virtual path (LINEAR — they're rendered previews, not
pixel-art); we fetch the **larger `/x1/`** render (≈768px, derived from the `tn` path via
`sixteen::x1_url`), not the tiny `/tn/`, so big grid tiles aren't a blurry upscale. **The
advanced search (Ctrl+F) filters a remote/flat listing in memory** (`colo_filter_in_memory`)
instead of walking disk: name matches the visible row text (filename + artist/group/pack),
SAUCE matches artist/group + cached SAUCE, plus ext/size/rating; dims/colours/date are
unknown for virtual pieces so those filters are ignored. Opening a piece
downloads just its single `raw` file (`start_piece_open`/`poll_colo_open` → `colo_files`
map; `load_full` decodes via `resolve_local`, keeping the virtual path as identity).
Per-row ⬇ menu saves the file or its pack `.zip` to disk (`download_piece` → rfd +
`sixteen::download_to`, reported via `colo_save_rx`). Entering a flat listing auto-switches
to table view; navigating away (`show_folder`) cancels the stream + clears `colo_pieces`.
**URL encoding:** the API returns *literal* paths, so a filename with a `#` (e.g.
`#44_FIRE.ANS`) would truncate every URL at the fragment — leaving the piece
un-downloadable and its thumbnail spinning forever. `sixteen::enc_path` percent-encodes a
site path **preserving `/`** (and `abs_url` runs every relative path through it; the
pack-view `raw_url` `enc()`s the built filename) so `#`→`%23` etc. survive
(`hash_in_filename_is_percent_encoded_in_urls`).

## Core invariants (read these before touching the data model)

- **`PixImage` (`image_types.rs`) always has `pixels` (RGBA, for display) and
  optionally `indexed` (the original `indices` + `palette`).** Palette-based
  formats must call `PixImage::from_indexed` (populates both); RGBA-only formats
  call `from_rgba` (leaves `indexed = None`). Preserving the palette is the whole
  point of the project — it's what makes future palette-swap / cycling / accurate
  re-export possible. `pcx.rs` is the reference for the palette-preserving path;
  `builtin.rs` (the `image` crate) deliberately leaves `indexed = None`.
- **Nearest-neighbor for *upscaling*; area-average for *downscaling*.** The viewer
  and source-res thumbnails upload `TextureOptions::NEAREST` so pixel art upscales
  crisply (and the viewer's pixel-perfect blit keeps it exact — see below). But a big
  image shrunk to a thumbnail must NOT be nearest-sampled: single-sampling a 50%
  dither (▒) aliases it into fake noise. So `make_thumb` **area-averages** (box
  filter) on downscale, and the upload picks `LINEAR` for any thumb it downscaled
  (`src_w/h > thumb w/h`), `NEAREST` otherwise — same rule in `grid_recolored_tex`.
  Net: crisp upscales, faithful-tone downscales. Don't nearest-sample a downscale.

## Colorizer / Recolor pane pipeline (`Adjust` + `apply_pipeline`)

The Recolor pane (`ui_recolor`) applies per-image **adjustments** then a **palette
rematch**, and the *order* of all of it is user-controlled.

- **`Adjust`** holds 12 value ops (brightness, contrast, gamma, shadows, highlights,
  posterize, hue, saturation, **vibrance**, pixelate, sharpen, **invert**) plus `order:
  [OpKind; 15]` — a permutation of `OpKind::ALL`. Three of those 15 are **marker ops**
  (no slider value), each marking *where* a step happens and configured in its own
  section: `OpKind::Palette` (the recolor/remap), `OpKind::Dither` (the dither
  pattern), and `OpKind::ColorBalance` (per-channel R/G/B offset). New ops are
  **appended** to `OpKind::ALL` so persisted order indices stay valid. `OpKind::is_marker`
  is the slider-vs-marker test the row UI keys off.
- **`apply_pipeline(rgba, w, h, &adjust, &PipeAux)`** is the one true pipeline: it
  walks `adjust.order` and runs each value op (point ops via 256-LUT, color ops via
  HSL, pixelate/sharpen spatial); at each *marker* op it does that step inline —
  `Palette` snaps via `thumb::remap_to_palette(aux.palette)`, `Dither` runs
  `thumb::dither_pass`, `ColorBalance` runs `color_balance(aux.balance)`. `PipeAux`
  carries the marker inputs (dither method/amount/custom matrix, the balance offset,
  the snap palette). All 5 recolor paths build it via `self.pipe_aux(palette)`: viewer
  preview (`make_preview`), full-res (`make_full_reduced`), grid tiles
  (`grid_recolored_tex`), swatch-flash (`make_flash_tex`, dither forced off), and
  `save_recolored`. `adjust_pixels` is a **test-only** wrapper (neutral aux).
- **Dither is decoupled from the snap.** Ordered/Bayer + the editable **Custom** matrix
  (`dither_custom`/`dither_custom_n`, seeded by `thumb::bayer_values`) are a *pure bias*
  applied at the `Dither` slot via `thumb::ordered_bias` — the later `Palette` op does
  the quantize, so dither can sit anywhere (e.g. before Posterize = dithered banding,
  no palette). Error-diffusion (Floyd–Steinberg/Atkinson) *can't* be moved off the
  snap, so it quantizes toward `aux.palette` at the Dither slot and no-ops if none.
  `thumb::dither_pass` dispatches both; `DITHER_NAMES`/`DITHER_CUSTOM` index the methods.
- **Color balance** = `out = in + (color − 128)·2·strength` per channel (the picked
  R/G/B/hex color read as a ±offset around neutral grey), clamped. `balance_offset()`
  resolves it to `[i16; 3]` and feeds `pipe_aux`; `parse_hex` powers the hex paste field.
- **Vibrance** = saturation push weighted by `(1 - s)` so muted colors move most,
  vivid ones are protected, neutrals (s=0) stay neutral. **Invert** blends toward the
  negative (0 = original, 1 = full, in between = partial/solarize).
- **`pipeline_active()` / `pipeline_key()`** are the canonical "is anything on?" and
  cache-key helpers — they fold in dither (method/amount + custom matrix when selected)
  and the balance offset on top of `adjust.key()`, so changing *any* stage invalidates
  the preview/full/grid caches. Use them, not bare `adjust.is_identity()`/`adjust.key()`.
- **Persistence:** values in `ADJUST_KEY` (`[f32; 12]`; legacy `[f32; 11]` still loads
  via `from_array11`), order in `ADJUST_ORDER_KEY` (`[u8; 15]`; legacy `[u8; 12]` still
  loads). `Adjust::key()` includes the order so reordering invalidates the caches.
  `with_order(&[u8])` tolerates corrupt/short/legacy orders (appends missing ops).
  Dither/balance persist separately (`DITHER_CUSTOM*`, `BALANCE_*` keys).
- **UI rows** are reorderable two ways: a painted **drag-handle grip** (`drag_handle`,
  drag → `adjust_drag`, drop reorders) and **⬆/⬇** buttons. Layout per row:
  `grip [label] [==slider==] [value]  ⟲ ⬆ ⬇`, with the `⟲ ⬆ ⬇` cluster
  **right-anchored** (a `right_to_left` sub-layout) so arrows align on every row
  regardless of value-box width; marker rows (Palette/Dither/ColorBalance) show only a
  label (spanning `marker_w`) and keep an empty `⟲` slot to match. The slider+value
  body is the shared `value_slider` helper: the slider has `show_value(false)` and the
  value is a **right-anchored `DragValue`** in a fixed `VALUE_W` slot (so the value
  column is right-justified). Each row paints a **zebra stripe** (odd rows,
  `faint_bg_color`) + **hover highlight** behind its content via a pre-reserved
  `Shape::Noop` slot (`painter().add` then `.set` once the rect is known, so it draws
  *under* the widgets). The Color-balance section reuses `value_slider` for wide
  R/G/B/Strength rows (each with its own `⟲`); Reduce + the Dither dropdown sit under
  the palette quick-chooser (above the long "All palettes" list) so they stay visible.
- **Reduce** (`quantize_on`/`quantize_n`) lives **only** in the Recolor pane (the old
  duplicate in the viewer status bar was removed). Its checkbox enables only when the
  image has an extractable palette (`pal_state` is `Some(Some(_))`).
- **Swatches + Export/Save** sit directly under the preview image (prioritized for
  small screens); the recolor preview + Details thumbnail scale to pane width.

## Viewer / large images

- **`TiledTexture`** uploads a still image as a grid of ≤ `MAX_TEX_DIM` (8192) tiles,
  so images over the GPU texture limit render at full resolution (`load_full`,
  `make_full_reduced`). GIF frames use `load_texture_capped` (downscale-to-fit).
- **Fit = `F`** is a sticky, persisted mode (`fit_mode`, `FIT_MODE_KEY`): toggling it
  on fits now and **auto-fits every newly loaded image** (`load_full`). `.` is unbound.
- **Per-kind viewer zoom, in device pixels (`N×`).** Text-mode/scene art
  (`is_textmode_ext`: ans/asc/nfo/diz/ice/cia/xb/xbin/bin/tnd/idf/adf) renders at a
  tiny **native 8×16 px per cell**, so a true 1:1 is unreadably small. Its zoom is
  measured in **device pixels per source pixel** — `textmode_zoom` (default **3×**, a
  Prefs combo), and the viewer reads out `N×` (not a %), because that's the unit that
  is actually crisp on a fractionally-scaled display (300% logical = a warped 3.9 dev-
  px; `3×` = a clean 3). `load_full` opens at `textmode_zoom / pixels_per_point`
  (logical), Z+N sets `N/ppp`, and Z+/Ctrl-wheel step the device ladder via
  `step_device_zoom`. **The ladder is integer in *both* directions** — `pp_device_scale`
  snaps the device scale to `N×` when upscaling (1,2,3,…) and **`1/N×` when zooming out
  below 1:1** (½,⅓,…,⅟₁₆), so a big or very tall scene can shrink to fit (the old
  `.max(1.0)` floor locked it at 1×; reads out `1/N×`). The viewer textures use
  `view_tex_opts` — **NEAREST magnification, LINEAR minification** — so a zoomed-out
  dither area-averages to grey instead of nearest-aliasing into noise. Raster art instead
  keeps a logical `%` zoom remembered across
  images (`raster_zoom`, persisted as the old `IMG_ZOOM_KEY`; `draw_image_view` writes
  it back each frame), whereas text-mode always reopens at its preference (manual zoom
  is transient). Both persist (`TEXTMODE_ZOOM_KEY`). `viewing_textmode` tracks the kind.
- **CRT aspect** (`crt_aspect`, `CRT_ASPECT_KEY`, **off by default**) is a viewer-only
  toggle shown in the status bar **only for text-mode art**: it stretches the blit
  ≈1.2× vertically (an 80×25 8×16 grid → 4:3) to match non-square VGA pixels. **X stays
  pixel-perfect** (integer device-px, for dither crispness); **Y is the CRT stretch** —
  `ny = round(nx·aspect_y)` *when that's still visibly taller than nx* (high zoom →
  uniform & crisp), else the **exact fractional** `nx·aspect_y` (low zoom). The exact
  fallback matters: a fit-to-screen tall ANSI sits at nx≈1–2, where `round(2·1.2)=2`
  **erased** the stretch — the main view looked unchanged on toggle while the
  linear-sampled previews (continuous 1.2×) did change. It's a *display* scale, not a zoom
  change, and never touches the texture. The
  **navigator minimap, Recolor preview and Details thumbnail all apply the same
  `aspect_y`** so every rendering of the open image agrees (the minimap used to stay
  native → looked squished next to the stretched main view + previews). The previews also
  take their **base aspect from the open image's `full_tex`** (via `preview_aspect`), not
  the downscaled thumbnail's own dims — otherwise a thumbnail decoded at a different width
  (e.g. cached at 8px before a 9px-cell toggle re-decoded the full view) renders squished.
  **But a 16colo flat-listing piece (`colo_pieces`) keeps its thumbnail's own dims** — its
  thumb is 16colo's pre-rendered PNG, a different renderer than our (often very tall) full
  decode, so forcing it to `full_tex`'s aspect squashed the preview into a thin sliver
  (looked "gone"). `make_preview` likewise decodes `resolve_local(path)` so the colorizer
  can recolor an opened 16colo piece (its bytes live at the cache file, not the virtual path).
- **9px VGA cell** (`font_9px`, `FONT_9PX_KEY`, **off by default**) is a separate
  status-bar toggle (next to CRT, text-mode art only) that renders the 8-pixel CP437
  glyph in a **9-dot-wide cell**, the way real VGA text mode did: the 9th column is
  background except for the line-draw range `0xC0..=0xDF`, where it repeats column 8 so
  box rules join (see `ansi::dot_on`). Off = exact 8px cells. Unlike CRT this is a
  *decode-time* change (the texture width grows ~12.5%, 80 cols → 720px), so it's a
  process-global flag (`decode::set_font_9px`, read by `AnsiDecoder`) primed from
  storage on startup; toggling it calls `redecode_full` to rebuild the viewer texture
  in place (keeping zoom/pan). `redecode_full` decodes `resolve_local(path)` (the **real**
  cache file) while keeping the virtual `path` as the stored identity — same split as
  `load_full`; without it, re-decoding a 16colo piece read the *virtual* path off disk,
  failed silently, and the toggle was a no-op. This is why ansilove/16colo (9-dot) render
  wider than a naive 8px blit. Thumbnails aren't re-rendered on toggle (sub-pixel at thumb
  scale).
- **Baud-rate playback** (ANSImation / "watch RIP draw"). The whole engine is "render
  the first N bytes into a fixed-size canvas": `ansi::TextStream` and `rip::RipStream`
  parse a byte *prefix* (canvas sized from the whole file so frames don't resize). The
  viewer's `Player` (parallel to the GIF `AnimState`) holds the stream + a byte cursor
  advanced at `Baud::cps()` × dt, caching the frame texture by cursor position. cps =
  baud/10 (8N1) **× a V.42bis compression factor** (`(baud/3600).clamp(1,4)`): real
  modems compressed ANSI ~4:1, so 14.4k+ "feels fast" like a real board while ≤2400
  stays an authentic crawl. Rates are the modem ladder (300/1200/2400/4800/9600/14.4k/
  28.8k/33.6k/56k/115.2k); RIP and ANSI keep **independent** remembered speeds
  (`baud_ansi`/`baud_rip`, picked by `Stream::is_rip()`). `Stream::for_file` makes a
  byte-prefix player for RIP + ANSI/CP437 streams; the **binary** scene formats
  (XBin/BIN/Tundra/IDF/ADF/PETSCII) aren't byte streams (RLE/headers/embedded
  font+palette), so `load_full` instead wraps their *decoded* image in a
  **`Stream::Cells(CellReveal)`** — a progressive **cell** reveal (first N cells in
  reading order over black, `textmode_cell` gives the 8×16 / 8×8 box) — so they "type
  out" at the baud rate too. The `Player` is unit-agnostic (it advances `pos` toward
  `len()` at `cps`; for `Cells`, `pos`/`len` are *cells* not bytes). A controls row (▶/⏸ · ⏮ Replay · byte
  seek) shows above the art; the **baud picker** is in the status bar. `Baud::None`
  (default) → instant, so the player sits at-rest and the view falls through to the
  static `full_tex` path (recolor/minimap keep working). Picking a baud restarts the open
  file. During playback the view auto-scrolls BBS-style to follow the typing **cursor**:
  `parse` returns the cursor's row extent (`max_y+1`, counting trailing blank lines the
  cursor moved onto — *not* just `grid.len()`), the canvas is sized to it, and
  `render_frame`/`Player.cursor_px` drive `play_autoscroll`. One extra frame renders on
  finish so the scroll lands exactly at the bottom (incl. a blank final line); the pan
  clamp keeps short (fits-the-screen) art put. Perf caveat: a frame re-parses+re-uploads
  the whole prefix, so very tall files at high baud are heavy (most ANSImations are 80×25).
  **Interrupt:** any user input while playing (`interrupt` in `ui_single` — a scroll,
  zoom gesture, or any key press) finishes the playback instantly (`pos=len`, `playing=
  false`) and clears `play_autoscroll` so the user can scroll/pan the full art; that
  input's own nav action is suppressed for the frame. The viewer also scrolls a long file
  with **Up/Down arrows** (Left/Right stay prev/next image), alongside the wheel, plus
  **Home/End** (top/bottom) and **PageUp/PageDown** — a page is **25 character rows** for
  text-mode art (an old 80×25 DOS screen: `25 · textmode_cell.h · scale.y`), else ~90% of
  the viewport for raster. All gated on `overflow_y` and re-clamped to the pan bounds.
- **Slideshow** (`auto_next` + `auto_next_secs` 1/3/5/10s, status bar, persisted). In the
  single view, once the file has *settled* (any baud transmission finished — `Player.playing`
  is the "busy" gate) and the delay elapses, `ui_single` steps to the next file. Dwell
  resets on load and while busy. While slideshow is on, RIP + raster (non-text-mode) art
  opens **fit-to-screen** (`load_full` sets `fit_requested`) so it's fully visible; text-mode
  keeps its readable zoom + fit-to-width. Great for flipping through a whole pack hands-free.
  **Auto-pause** (`auto_paused`, not persisted): while running, any deliberate user
  interaction in the viewer — scroll, zoom gesture, key press, or a drag-to-pan
  (`pointer.is_decidedly_dragging()`; passive mouse *movement* is excluded so a hands-off
  screensaver isn't paused) — sets `auto_paused`, which gates the advance. The status-bar
  "auto ▶" then renders **yellow** (a `RichText` color) with a "you took control — click to
  resume" hover; clicking it while paused **resumes** (the checkbox would set `auto_next`
  false, so the handler forces it back true and clears `auto_paused`) instead of toggling
  the slideshow off. Pause persists across manual navigation until you click to resume.
- **Metadata OSD** (`osd_enabled`/`osd_position`/`osd_secs`, Preferences →
  "Viewer info OSD", persisted). `load_full` resets `osd_t=0` + `osd_dismissed=false` (and
  primes SAUCE via `cached_sauce`) so a fading rounded panel appears on each opened image:
  fade-in (0.35s) → hold (`osd_secs`) → fade-out (0.7s), painted at the end of
  `draw_image_view` (covers static / player / GIF paths). `osd_content` returns a **headline
  title** + a list of `(gap_before, fields)` **rows** so sections get vertical breathing room:
  the title (larger + faux-bold via a 0.7px double-draw), then the **artist(s)** on their own
  line — a collab piece's `", "`-joined `piece.artist` is split so **each artist is its own
  link** (flows `a · b · c`) — then the **SAUCE comment / description** (the COMNT block on
  local scene files — `sauce::parse`,
  which `cached_sauce` reads a ~16 KB tail for since COMNT precedes the 128-byte record; the API
  `Comments` string on 16colo pieces), then an **attributes** row flowing `label value · …`
  (type / columns / lines / font / group / pack / year, or local type / size / dimensions /
  colors / created) — ending in a ★ rating. A one-field row reads as its own line; a
  multi-field row wraps only on overflow. **Placement** (`osd_position` 0..=7, a spatial 3×3
  Preferences picker with the center unused): each index decodes to a horizontal third
  (left/center/right) and vertical third (top/middle/bottom) resolved independently, so any
  corner or edge-center — top-left, top-right, bottom-left, bottom-right, etc. — is a single
  choice. **Interactive:** `paint_osd` returns `(rect, link rects, close rect)` —
  links (`osd_links`) each carry an `open_folder` target (a local directory or a 16colo
  artist/group/pack/year virtual path), and the top-right **`×`** (`osd_close`) **dismisses the
  OSD for this view** (`osd_dismissed`, reset on the next `load_full` so it returns on the next
  image). `draw_image_view` hit-tests last frame's rects (using `hover_pos().or(interact_pos())`
  so *passive* hovering — no button held — counts, which is what re-pins it reliably **even
  mid-fade-out**): hovering **pins** it open (`osd_t` reset to the hold, full opacity) **and
  pauses the slideshow** (`auto_paused`), underlines + pointing-hands the link under the cursor
  (the `×` takes click priority); a link click navigates there. Once pinned, the only ways out
  are the `×` or letting it fade un-hovered. The panel caps to the viewport width and clips
  overflow.
- **Random-pack screensaver** (`shuffle` + "🔀 Random pack" button, status bar). A worker
  (`start_random_pack` → `random_rx`, polled by `poll_random`) picks a random 16colo.rs year
  + pack (`pick_random`, wall-clock seeded — no `rand` dep), inserts its download URL into
  `remote_urls`, and `open_folder`s the virtual pack path (which downloads + mounts it).
  `pending_autoplay` then opens the pack's first art file once both `random_rx`/`remote_rx`
  idle. With **Shuffle** on, reaching a pack's end (a `step_image` no-op — it doesn't wrap)
  triggers the next random pack → endless. Shuffle auto-enables `auto_next`; pair with F11
  for a screensaver of real scene art. **R** jumps to a new random pack (skip a dud).
- **Look defaults** lean to the late-night-BBS aesthetic (`crt_aspect`, scanlines 0.5 +
  `scale`, `glow`, `black_bg`, `zoom_lock`/Snap, `auto_next`, ANSI 4800 / RIP 9600 baud —
  all `unwrap_or(...)` in `PixelView::new`). `font_9px` is the one left default-off: its
  setter flips a process-global the decoder reads, which would leak into parallel decode
  tests. Persisted values always override the defaults.
- **Fit W** button (status bar, single view): re-applies the fit-to-viewport-width zoom
  (sets the one-shot `fit_width_on_open`) after you've zoomed away. Text-mode art already
  fits width on open.
- **Immersive mode** (`immersive`, F11). Hides every panel + the playback controls row,
  drops the window decorations, and goes OS-fullscreen (`ViewportCommand::Fullscreen` +
  `Decorations(false)`), showing only the art (navigator suppressed too). `update` computes
  `show_top/bottom/left/right` from the pointer's distance to each window edge
  (`content_rect`, 48px) and gates each panel group, so a bar reveals when the mouse reaches
  its edge and hides when it leaves. The mouse cursor auto-hides after ~1.5s of stillness
  (`idle_t`; `set_cursor_icon(None)` set last so it wins) and reappears on any movement. Not
  persisted (always starts windowed).
- **Phosphor glow** (`glow` toggle + `glow_amt` slider, status bar, persisted). `paint_crt`
  redraws the art a few times offset in a ring with **additive** blending — the egui trick
  is a premultiplied tint with **alpha 0**, which turns the "over" blend (`src + (1-a)·dst`)
  into `src + dst`. Dark areas add ~nothing, bright glyphs bloom into a halo. Radius tracks
  the zoom (`src_px_dpx`). Reuses the displayed texture (incl. the live player frame), so no
  rebuild. Composes under the scanlines.
- **Retro-monitor scanlines** (`crt_scanline_dark` slider, status bar, persisted, any
  single-tile image). `paint_crt` blits the (panned) art, then tiles a 1×3 opaque-black-row
  scanline texture down the **viewport** (the monitor, not the art — so a scrolling
  ANSImation isn't distorted); darkness is the **draw-time tint alpha** (slider → 0..255,
  no texture rebuild). The `scale` toggle (`crt_scanline_scale`) sets the line period to
  one-per-source-pixel-row (`scale.y·ppp`) so lines grow with zoom; off = fixed 3px. The
  `black bg` toggle (`black_bg`) fills the viewer background black instead of grey-28.
  (Barrel-curvature and vignette modes existed briefly but were dropped as not-useful.)
  Effects skipped for multi-tile (huge) images → plain blit.
- **Pixel-perfect blit (accuracy — read before touching `draw_image_view`).**
  Nearest-neighbor only stays undistorted when one source pixel maps to a **whole
  number of *device* pixels** — and integer `zoom` is not enough, because what counts
  is `zoom × pixels_per_point` (fractional desktop scaling, e.g. ppp≈1.3, otherwise
  duplicates some source pixels more than others → warped dither). So when
  `pixel_perfect` (`viewing_textmode || zoom_lock`) we round to an integer device-px
  scale per axis (`nx/ny`), set `scale = (nx,ny)/ppp` points-per-source-pixel, **and
  snap the image origin to the device grid** (a sub-pixel origin re-warps even at an
  integer scale). `self.zoom` is re-aligned to `nx/ppp` (idempotent), and the status
  bar shows that device scale as `N×` (= `round(zoom·ppp)`) rather than a fractional %.
  The font (`cp437_font.rs`) is the authentic IBM VGA ROM (identical to
  ansilove's BlockZone) — the shade glyphs 0xB0/B1/B2 ARE the canonical 25/50/75%
  dithers; if dither ever looks warped it's the *scale*, not the glyphs.
- **Margin wrap (`decode/ansi.rs`).** Writing the last column parks the cursor at
  `wrap`; the wrap fires at the **top of the parse loop, before the next byte** — like
  ansilove — so it happens whether that byte is a printable char *or an ESC sequence*
  (`ESC[s`, SGR). Checking it only on printable chars (an earlier "deferred wrap")
  mis-saved an `ESC[s` parked at the margin and **sheared cursor-addressed art**
  (ACID-RN.ANS, gj-os.ans) — see `save_at_right_margin_wraps_before_saving`. The top
  check excludes CR/LF, so an exactly-`wrap`-wide line + CRLF still occupies one row
  (no blank), keeping `full_width_line_plus_newline_has_no_blank_row`.

## Font glyph gotcha (solved — two embedded fallbacks + an icon set)

egui's *bundled* fonts (Ubuntu-Light + a small NotoEmoji subset) omit whole Unicode blocks,
so glyphs used to render as tofu (`□`). **`install_fallback_font` (`app.rs`, called first
thing in `PixelView::new`) appends TWO embedded fallbacks** to the Proportional + Monospace
families: **Symbols Nerd Font** (`assets/SymbolsNerdFont-Regular.ttf`) for designed UI icons,
then **DejaVu Sans** (`assets/DejaVuSans.ttf`) to back every remaining standard-Unicode symbol
(Arrows, Geometric Shapes, Misc Symbols, Box Drawing, …). Both are appended *last*, so they
only fill gaps — existing text + color-emoji (`📁`/`🎹`/`🔊`/`★`) come from the earlier fonts
unchanged. Between them, **nothing we draw tofus.**

- **For a real icon, use `icons::*`** (the `mod icons` block near `install_fallback_font`) —
  `PLAY`/`PAUSE`/`STOP`/`VOLUME`/`MUTE`/`LOOP`/`SHUFFLE`/`REFRESH`/`DOWNLOAD`/`SEARCH`/`BOLT`/
  `SORT_ASC`/`SORT_DESC`/`PIN`/`GLOBE`/`MUSIC`/`PIANO`. **Codepoint rule (critical):** use Nerd
  Font's **Material-Design range (U+F0xxx, plane 15)**, *not* the low FontAwesome range (U+F0xx) —
  the text font stores the fi/fl ligatures at U+F001/F002 and egui's bundled emoji-icon-font is
  FontAwesome-based, so a low codepoint gets **shadowed** (music note → "fi"). Plane-15 is
  untouchable by any other stacked font. Verify a new codepoint is in the font's cmap first
  (fonttools: `TTFont(path)['cmap']`; a venv one-liner — see git log).
- **For a plain symbol**, a normal Unicode char (`·`/`…`/`×`/`★`) still just works via DejaVu.
- Only ASCII (`*`) or a *painted* shape (see `drag_handle`'s dots) if something tofus even with
  both fallbacks (very new emoji / CJK / niche pictographs). When in doubt, test in the real app.

## Button-jiggle gotcha (read before a button whose label swaps by state)

egui sizes each widget to its content every frame, so a button whose label changes width
between states — a glyph swap (`▶`/`⏸`, `🔊`/`🔇`), a text swap (`↑ Asc`/`↓ Desc`,
`▶ Play`/`⏸ Pause`), or a live counter — **resizes on toggle and shoves the rest of its
horizontal row sideways** ("jiggle"). Fix: pin such a button to a fixed width with
`.min_size(fixed_btn_size(ui, &["state A", "state B", …]))` — the helper measures the widest
label's galley + button padding (font-robust, no magic px). For a *label* that shoves buttons
(e.g. the single-view zoom readout `N×`/`1/N×`/`%`), reserve a fixed width via
`ui.add_sized([w, h], Label::new(..))` with `w` measured off the widest reading. All the audio
transport / GIF / RIP-baud play-pause, the sortbar Asc/Desc, and the menu-bar mute are pinned
this way — match it for any new state-swapping control.

## Palette-order gotcha (ANSI SGR vs VGA attribute)

There are **two index orderings** for the same 16 colours, and mixing them swaps
red↔blue + cyan↔brown (and their bright variants). `ansi::PALETTE` is in **ANSI SGR**
order (SGR 31=red→index 1, 34=blue→4) — correct for `.ans`, whose parser maps SGR codes
to those indices. The **binary** text-mode formats (`.bin`/`.xbin`/iCE) store *raw VGA
attribute bytes*, where index 1=blue and 4=red, so they must use `ansi::VGA_PALETTE`
(the same colours in hardware order). `render_textmode` indexes whatever palette it's
handed by the raw `attr & 0x0f`, so the **caller** picks the right order: `bin.rs` and
`xbin.rs` (no embedded palette) pass `VGA_PALETTE`; XBIN/IDF/ADF *embedded* palettes are
already VGA-ordered (raw RGB by attribute index) and are indexed directly. Bug symptom:
a piece whose 16colo/ansilove thumbnail is red renders blue in the viewer
(`MULTI-13.BIN`); guarded by `bin::tests::vga_attribute_indices_are_not_ansi_order`.

## Settings & ratings

- **Persistence** uses eframe's `persistence` feature (storage at
  `~/.local/share/pixelview/`). Each setting is its own key (consts on `PixelView`:
  `ZOOM_KEY`, `THUMB_KEY`, `FAV_KEY`, `FOLDER_KEY`, sort/filter keys, `EXPLORER_KEY`,
  `DETAILS_KEY`, `GAP_KEY`/`GAP_Y_KEY` (h/v grid spacing), `CAPTION_KEY` (caption
  bitmask), `KEYMAP_KEY`).
  `persist_egui_memory()` returns `false` — we persist only our keys, in `save()`.
- **Last folder** (`FOLDER_KEY`) is reopened on launch (CLI `--folder` wins). `save()`
  stores the **display** path, not `self.folder` — inside an archive / downloaded 16colo
  pack the latter is a temp dir that's gone next run, whereas the display path
  (`pack.zip/…`, `<16colo.rs>/year/pack`) is stable. `new()` restores it when it's a real
  dir, a 16colo path (re-fetched), or an archive file (re-extracted) — `open_folder`
  routes each. (A subpath *inside* a local archive isn't restored — `is_archive` only
  matches the archive file itself.)
- **Two independent zoom axes:** Ctrl +/- = whole-GUI scale (egui `zoom_factor`);
  Ctrl+wheel = thumbnail tile size only. Ctrl+wheel arrives as `zoom_delta()`, NOT
  `smooth_scroll_delta` (see gotcha).
- **Ratings** (`rating.rs`) read/write `user.baloo.rating` (ASCII `0..10`, 2 per
  star) — the KDE Baloo / Gwenview scheme, so they interoperate with Gwenview and
  the user's `~/git/qb64pe-lab/greywood/sort-by-rating.sh`. Keys 1–5 set, 0 clears
  (removes the attr); single view rates the current image. In the grid/table the **tile
  under the cursor wins** when it isn't part of the selection (`apply_rating`) — so "point
  at a piece and press 3" rates *that* one even with an earlier-opened piece still selected
  (the 16colo flat-listing gotcha); hovering one of the selected tiles still rates the whole
  selection. The shared `entry_context_menu` also has a **★ Rating** submenu (Unrated/1–5
  with the 0–5 hotkeys shown via `Button::shortcut_text`, current marked selected →
  `TilePick::Rate` → `rate_entry`), a reliable rating path for 16colo pieces.
- **Cross-platform sidecar** (`ratings.rs`, `RatingStore`): xattrs only exist for a
  real on-disk file on Linux, but **virtual art has no such file** — archive contents
  are extracted to a *disposable* temp dir and 16colo.rs pieces are downloaded on
  demand. So ratings are also kept in `ratings.json` (in eframe's data dir), keyed by
  the **stable display path** (`/u/pack.zip/SUB/ART.ANS`, `<16colo.rs>/2023/p/ART.ANS`
  — what `to_display` yields). The rule, in `read_rating`/`set_rating`: a path whose
  display ≠ disk (i.e. inside a mount) is **sidecar-only**; an on-disk file uses xattr
  as source of truth (so external Gwenview edits win) **and** mirrors to the sidecar
  (a portable record + the fallback where xattrs don't exist). `make_entry` no longer
  reads the rating itself — `open_folder` overlays every file via `read_rating`, the
  single resolution path. This is what makes a piece **inside a zip or on 16colo.rs
  ratable at all**, and it survives re-extraction because the key is the archive path,
  not the temp file. `set_rating` flushes immediately; `save()` re-flushes on exit.

- **View history** (`viewdb.rs`, `ViewDb`): a **SQLite** store (`views.db` in eframe's
  data dir, via `rusqlite` `bundled`) of visited-state + view count + first/last-viewed,
  keyed by the same **stable display path** as ratings (`view_key` = `to_display` as a
  string). All rows mirror into an in-memory `HashMap` on open so the grid/table can ask
  `is_viewed` for every visible tile each frame without touching SQLite; writes go through
  immediately (user-paced). `ViewDb::open` returns `None` on any SQLite error → the app
  runs with tracking silently disabled (never crashes). **Recording:** `load_full` marks a
  file viewed when opened in the viewer; the top of `open_folder` marks a folder/pack
  viewed when entered (recorded *before* the remote/archive redirect so the key is the
  tile's own path). **Surfaces:** the table filename is a browser-style visited link
  (unvisited = `hyperlink_color` + underline, visited = `weak_text_color`, no underline);
  grid + pack tiles get a *painted* check badge (top-right — `paint_check_badge`, painted
  not glyphed because the bundled font lacks a reliable ✓); the Details pane shows
  `Views`/`Last viewed`; and the shared `entry_context_menu` has **Mark as (not) viewed**
  (`TilePick::ToggleViewed(bool)` → `set_viewed`). Timestamps are unix seconds
  (`unix_now`), formatted via `date_ymd_unix`. **rusqlite is pinned to 0.37** — 0.40 pulls
  `libsqlite3-sys` 0.38 whose build script uses the still-unstable `cfg_select!` and fails
  on stable rustc 1.92 (see Cargo.toml comment).

## Adding a format (the common task)

Copy `decode/pcx.rs`, implement the `Decoder` trait (`name`, `extensions`,
`sniff`, `decode`), and add one `Box::new(...)` line to `Registry::with_builtins`
in `decode/mod.rs`. Use `from_indexed` if the format is palette-based (IFF/ILBM,
LBM), `from_rgba` otherwise. Place the new sniff-able decoder ahead of
`ImageCrateDecoder` if its magic bytes could be ambiguous. Adding extensions to a
decoder's `extensions()` makes them viewable via `known_extension` **and** the two hard-coded
parallel lists in `app.rs` — `is_image_ext` (montages/counts/prev-next; `code.rs`/`audio.rs`
re-export `CODE_EXTS`/`AUDIO_EXTS` so it shares one list) and `is_textmode_ext` (crisp-integer
textmode zoom + CellReveal typeout + SAUCE panel — usually *not* wanted for non-scene files).

**Extension-routed decoders (`code`/`audio`).** A `Decoder::decode` only gets `bytes`, not
the path — but source code needs the extension to pick a language spec and audio needs it as a
format hint. So `decode_bytes` special-cases `CODE_EXTS`/`AUDIO_EXTS` *before* the generic
extension loop, calling `CodeDecoder::decode_ext(bytes, ext)` / `SoundDecoder::decode_ext`
inside `caught(||…)` (the same panic guard as `decode_caught`). Both always return *some* image
(highlighted text / plain / waveform / icon), never an error, so a weird file still shows a tile.

**Audio player + PDF viewer + trackers — all shipped (not feature-gated).**
- **PDF**: the tile is a real `pdftoppm`-rendered first page (placeholder fallback if poppler
  is absent). Opening a PDF enters an in-app **multi-page viewer** (`PdfView` in app.rs):
  Prev/Next + "Page N/M" + a 1-page vs 2-page spread toggle; Left/Right turn pages (not step
  images); each page renders on demand via `render_pdf_page` into `full_tex` (reusing zoom/pan/
  fit); two-page mode composes facing pages side by side.
- **Audio** (`AudioPlayer` in app.rs, `rodio`): decodes the file to a sample buffer ONCE, then
  each play appends a fresh `SamplesBuffer` of the selected region to a new `Player` — so it
  restarts cleanly (the old "play once" bug), loops (`repeat_infinite`), and plays a drag-
  selected region. Opening an audio file puts the **full player right in the main viewer**
  (not just the side pane): `load_full` calls `ensure_audio_loaded` (decode + open device
  without necessarily starting — Autoplay starts it) so the transport + waveform + keyboard
  are visible **immediately on open**, no Play press needed. The whole player UI lives in one
  shared method, **`draw_audio_controls(ui, path, big, meta_dur)`**, called two ways: `ui_single`
  draws it `big` (220px waveform + 130px keyboard) filling the viewer when the open file
  `is_audio_ext`; `ui_details` draws it compact (`big=false`, 96px/66px) under the metadata grid.
  It draws an **interactive waveform** (hi-res peaks + a moving loop-aware playhead + shaded
  region): drag = set loop region, click = seek; plus a loop toggle, an **Autoplay** checkbox
  (persisted: play on select + loop until Stop), a Stop button, and **Spacebar** play/pause. The
  method collects `want_*` locals while drawing and applies them at its own end (it's a
  `&mut self` method, not an egui closure, so no caller-side deferral). The device opens
  **lazily + fallibly on first Play** — a headless box reports "no audio output" and `cargo test`
  never touches a device. Leaving the file (`load_full` on a non-audio path) tears the player
  down so it can't keep looping.
- **Trackers** (MOD/XM/S3M/IT): `AudioPlayer::open` routes tracker extensions to **`xmrs`**
  (pure Rust — parses + synthesizes to interleaved-stereo i16 → f32) into the same sample
  buffer, so loop/region/playhead/waveform all work for modules too. voc/au (no in-app
  decoder) keep the icon tile + external open.
- **More trackers** (669/FAR/OKT/MED/AMF/ULT/MTM/STM): no pure-Rust player exists, so **libxmp**
  (MIT, vendored under `vendor/libxmp`) is **compiled from source** by `build.rs` (a `cc` build —
  no cmake/autotools/system dep; it's self-contained + only links libm). `build.rs` compiles
  exactly the files in libxmp's own `cmake/libxmp-sources.cmake` full list (a plain `src/**.c` glob
  pulls in two disabled/leftover ProWizard files that don't compile). `libxmp.rs` is a tiny FFI:
  `xmp_load_module_from_memory` → render to interleaved `f32`, run off the decode worker thread
  (own context per render). It renders **exactly the song length** from `xmp_get_frame_info`'s
  `total_time` with `xmp_play_buffer(loop=1)` — many modules loop forever, and 10 min of stereo
  f32 is 211 MB, so a blind cap would over-render. `is_tracker_ext` is now just MOD/XM/S3M/IT
  (which also get the sample explorer); `is_libxmp_ext` handles the rest.
- **MIDI** (`.mid`/`.midi`/`.kar`/`.rmi`): a MIDI file is only note events, so `render_midi`
  synthesizes it to PCM through a **General MIDI SoundFont** via `rustysynth`
  (`MidiFileSequencer::render`) — then it feeds the same player path as any audio. The SoundFont is
  a persisted preference (`midi_soundfont` / Preferences → "MIDI SoundFont"), else auto-detected
  from common system paths (**preferring a small ~6 MB TimGM6mb over a 100+ MB FluidR3** — the load
  is synchronous). The loaded font is cached in `midi_sf_cache` (an `Arc`, parsed once per session,
  not per MIDI); changing it clears that + the decode cache. `None` available → a "set one" note.
  `.rmi` is RIFF-wrapped MIDI; `rmid_inner` strips the wrapper before synthesis.
- **RAD** (`.rad`, Reality Adlib Tracker): **OPL2/OPL3 FM synthesis**, not PCM — so neither `xmrs`
  nor rodio can play it. `render_rad` drives two **public-domain C→Rust ports**: `decode/rad.rs`
  (the RADPlayer replayer — `new()` parses v1/v2, `update()` emits OPL register writes per tick,
  `hz()` the tick rate, end-of-song via the `RAD_DETECT_REPEATS` order map) feeds `(reg,val)` writes
  into `decode/opl3.rs` (the "Opal" OPL3 chip emulator — `write_reg` + `sample()` with an internal
  49716→44100 resampler), pulling `sr/hz` samples per tick. Both are faithful ports (tables verbatim,
  wrapping arithmetic, bounds-guarded so a malformed file returns `None`/never panics). Ported (not
  the GPL `opl-emu` crate) so pixelview stays MIT.

Both player surfaces also have an **onscreen piano keyboard** (`piano_keyboard(ui, octave, h)`
— `h` sizes it big vs compact) + Oct −/+ to audition the sample as a one-shot instrument — a
key plays the selected region pitch-shifted via rodio `speed()` (2^(semitone/12));
`AudioPlayer::play_note` / `play_speed` keep the playhead correct at a pitched tempo.

**Master audio controls** live flush-right in the **menu bar**, shown only while the audio
plugin is on (`self.plugin_audio`): a **🔊/🔇 mute** speaker toggle, a **global ⏹ stop**, and a
**volume slider** (reading left→right; added right-to-left in a `Layout::right_to_left` block).
Volume/mute are `audio_volume` (0..1) + `audio_muted`, both persisted (`AUDIO_VOLUME_KEY`/
`AUDIO_MUTED_KEY`) and pushed live to the player via `set_audio_volume`/`toggle_audio_mute`.
Because `play_source` rebuilds a fresh rodio `Player` on every play (to cleanly replace a
finished/looping source), the gain is re-applied there from `AudioPlayer::effective_volume()`
(= 0 when muted, else `volume`) — not just once; `apply_volume()` handles a live slider drag.
`AudioPlayer` mirrors the two values (seeded from `self.audio_volume`/`audio_muted` wherever a
player is created — `ensure_audio_loaded` + `toggle_audio`). The menu-bar handlers defer through
`audio_stop`/`audio_mute`/`audio_vol` locals applied after the `MenuBar` closure (it can't
borrow `self` twice).

**Sample explorer (a swappable-buffer model).** A tracker module carries a bank of individual
samples; `AudioPlayer.tracker_samples: Vec<NamedSample>` holds them (`extract_tracker_samples`
walks the xmrs module's `InstrDefault.sample` lists → mono `f32` at a C-4-derived rate:
`8363 · 2^(relative_pitch/12)`, with precomputed peaks). Below the keyboard, `draw_audio_controls`
lists them: click one to **load** it (the transport/waveform/keyboard all follow it) or ⬇ **export**
it as WAV. The mechanism is a **swappable buffer**: `SampleBuf` bundles samples/channels/rate/
duration/peaks; `select_sample(Some(i))` stashes the song via `take_buffer` → `song_backup` and
`put_buffer`s the sample; `select_sample(None)` restores the stash — so **only one song copy is ever
held** (the "Full song" row reverts). `write_wav_16` is a tiny dependency-free 16-bit PCM writer.

**Hardware MIDI input (`midir`).** Pick a controller in the big player's "MIDI in:" combo
(`midi_input_port_names` enumerates with a throwaway `MidiInput`; `open_midi_port` connects — the
callback runs on **midir's own thread** and sends `(note, vel, on)` over an mpsc `Sender` passed as
the callback's data). `poll_midi(now)` (called each frame in `ui()`) drains the receiver and routes
each note-on through **`route_note_on(note, vel, now)`** (see the sample-pad grid below): a note
matching a loaded pad triggers that pad, otherwise it auditions the editor sample via
`AudioPlayer::play_note_vel(note − 60, vel)` (MIDI middle C = native pitch, velocity scales gain,
Note Off ignored — monophonic one-shot "preview"). `midi_conn` **must be kept alive** (drop =
close); `connect_midi(None)` closes it. The chosen device persists (`MIDI_PORT_KEY`), auto-reconnects
on launch when still present, and shows a **✓** in the big player's "MIDI in:" row when connected.
The callback also calls **`ctx.request_repaint()`** (via a stored `egui_ctx` clone passed to
`open_midi_port`) — otherwise a note while the UI is idle isn't drained by `poll_midi` until the next
repaint (a mouse move), which reads as "MIDI doesn't play until I move the mouse". midir's ALSA-seq
backend links the **same `libasound2`** rodio already needs — no new system dep.

**Sample-pad grid (a mini Battery) + kits.** The big audio player (`draw_audio_controls(.., big=true)`)
splits below the waveform into a **fully resizable** layout — draggable dividers (`drag_h_divider`)
set the **waveform height** (`AUDIO_WAVE_H_KEY`) and **keyboard height** (`AUDIO_KB_H_KEY`), and a
vertical divider sets the **sample-list pane width** (`AUDIO_LEFT_W_KEY`). The right column has a
**multi-octave keyboard** (`piano_keyboard` fits as many whole octaves as the pane width allows at
~34 px/white-key, C keys labeled by octave) above a **4×4 pad grid** (`draw_pad_grid`) that **fills
the remaining vertical space** (`cell_w`/`cell_h` divide the pane's width/leftover height, so the 16
pads fit the bottom exactly — the dividers reshape them). The big audio view is a
`ScrollArea(auto_shrink=[false;2])` so `available_height()` is real. Side docks set `min_size` so a
drag can't shrink them into a black sliver. A `Pad` (`pads: Vec<Pad>`, always
16) holds `buf: Option<Arc<SampleBuf>>`, an assigned MIDI `note`, `volume`/`muted`/`soloed`, and
per-pad **pitch / loop region / loop_on / loop_type (Forward/Reverse/Ping-pong)**. Pads **auto-map
chromatically from a base note** (`pad_base_note`, default 48 = C3, a header dropdown); `pad_note(i)`
= the individual override (MIDI-learn) or `base + i`. Per pad: ⟲ load (captures the current editor
selection → `load_pad`, WAV write-through to `<data>/pads/pad_NN.wav`), **e** drill-in editor
(`focus_pad`/`focus_back`: re-points the main waveform at the pad's sample so its loop/pitch/type are
set on the big editor, restored on Back — via `take_buffer`/`put_buffer` + an `EditorStash`, gated by
`EditFocus::{Song,Sample,Pad}`), 🎹 MIDI-learn (`pad_assign` → next note assigns), ⬇ WAV download,
× clear, **M/S** (`pad_is_audible` folds mute + kit-wide solo), a **V** velocity toggle (on = track
the played note's velocity, off = fixed 127), a volume slider, and a painted vertical
**VU** (`pad_levels`). A pad is triggered by clicking it, or a matching note (onscreen/MIDI) via
`route_note_on`; **`AudioPlayer::trigger_pad_voice`** fires each hit as its own `rodio::Player` on the
**shared `_stream.mixer()`** (`pad_voices: Vec<PadVoice>`, reaped each frame) — the mixer sums them,
so pads are **polyphonic** (the base player is monophonic). Feedback (the user's ask): every played
note lights its keyboard key (`note_flash`) — **red** if it routes to a pad, an accent otherwise — and
the triggered **pad flashes green**. **Octave lock** (`octave_lock`) keeps the keyboard octave across
drill-ins **and, while on, auditions a browser-clicked sample at that octave** (`select_sample` takes
a `semitone` = `audio_octave * 12`; the "Full song" always plays native so it isn't sped up).
**Global velocity tracking** (`kit_global_vel` + `kit_global_vel_amt`, a kit-wide checkbox + 0–127
slider in the Pads header) overrides every pad's per-pad **V** with one fixed velocity for the whole
kit. **PANIC** (**Shift+Esc**, `audio_panic` → `AudioPlayer::panic`; a bright-red button in the menu
bar *and* both transport rows) stops all sound + pad voices + MIDI notes, incl. looping pads — the
truly-global escape hatch (the plain back-to-grid Escape excludes Shift). The whole kit is a
**persistent cross-file working set** (metadata in `PADS_KEY`, audio in
`<data>/pads/*.wav`, reloaded in `new()` via `decode_audio`), plus **named kits**: Save/Load + a name
field save the kit as a `.pvkit` (a zip — `manifest.txt` storing the kit name + **MIDI controller +
base/octave + global velocity + every pad's record** + `pad_NN.wav`s; `save_kit`/`load_kit` via the
`zip` crate). Saved
kits live in `<data>/kits/`. **Places dock sub-tabs are `Local · 16colo · Kits · Samples`** (last two
audio-plugin-only): the **Kits** tab lists saved `.pvkit`s (click → `load_kit` into the current pads,
no navigation) and **opens the standalone pad editor** (below); **Samples** holds user-added sample
folders (`sample_places`: name/dir/color, `SAMPLE_PLACES_KEY`) — `＋ Add location…` (rfd), click to
browse, right-click to rename (inline) / colorize (ANSI32 swatches) / remove. All gated on
`self.plugin_audio`.

**Standalone Sample-Pads editor (`kit_editor`).** Clicking the Kits tab or loading a kit shows the
pad grid + keyboard + a **silent waveform** with no audio file open (`enter_kit_editor` → `mode =
Single` + `ensure_kit_editor`, which installs a 1 s **silent `AudioPlayer`** at the synthetic
`kit_editor_path()` so the pad mixer has a device + a flat waveform). `ui_single` renders the "🎹
Sample Pads — <kit>" view (heading + ‹ Grid) via the shared `draw_audio_controls(big)`; `kit_editor`
clears on navigation (`open_folder`) or opening any file (`load_full`).

**Waveform editor (`draw_audio_controls`'s interactive waveform).** A pro sample-editor interaction,
resolved BEFORE drawing so the shade/edges track the live drag. `WaveDrag`/`Edge` model the drag:
drag empty = new selection, drag an edge = adjust it (the opposite edge anchors → crossover swaps
L/R), drag inside = move; a green hover line shows where a new selection begins, edges are bright-
green handles (hot on hover/drag), the cursor is `↔`/Grab; a **full-file selection counts as no
selection** so a drag can always start a sub-selection. `wave_view` is a zoom window: **mousewheel
zooms** around the cursor (consuming `smooth_scroll_delta` so the audio ScrollArea doesn't scroll;
per-sample bars when zoomed in), **Shift+wheel pans**, double-click resets. Over an edge, **wheel
nudges** it by 1 / Shift 10 / Shift+Alt zero-crossing sample(s) (grow vs shrink by direction), and a
magnified **edge inset** (the `zoom_edit_pct` "Zoom Edit %" pref) overlays per-sample detail with an
`@ smp` readout. Move-drag snapping: **Alt+Shift** → nearest zero crossing (`next_zero_crossing`),
**Alt** → nearest transient. **Transients** (`detect_transients`: RMS energy-flux + refractory gap,
sensitivity slider; drawn as culled amber guideline markers) + **BPM** (`estimate_bpm` Detect) + a
**Musical** beat-division grid. `next_zero_crossing`/`detect_transients` are unit-tested. A `.pvkit`
file (shown with a KIT badge) **loads on click** (`is_kit_ext` → `load_kit`, not the viewer).

**Window geometry** persists (`WINDOW_GEOM_KEY` = `[x,y,w,h]`, captured each frame, restored on the
first frame the monitor size is known, clamped on-screen) — except in `DEBUG_MODE`, where the
bottom-right dev dock wins.

**Sample banks as folders (`soundfont.rs` / `sfz.rs` / `dls.rs`).** A `.sf2`, `.sfz`, or `.dls`
is a **virtual folder** of its samples. `is_sample_bank(path)` folds all three into the
`Entry.is_archive` path (📁 glyph + a per-format badge, click-to-enter, prev/next skipping), and
each gets an `enter_*` that mirrors `enter_archive` — extract/mount to a per-file temp dir (cached
by path+size+mtime) via `ArchiveMount`, so `to_display`/`real_path` give the `<file>/NNN_name.wav`
breadcrumb and every sample opens/plays/rates/exports like a real file. A cached `*_info` (`sf_cache`
/ `sfz_cache` / `dls_cache`) feeds the Details pane:
- **SF2** (`soundfont::extract_to_cache`, `rustysynth`): writes each `SampleHeader`'s slice of
  `get_wave_data()` (one shared `&[i16]`) to a 16-bit WAV. Details: Presets / Instruments / Samples.
- **SFZ** (`sfz::mount_to_cache`): an `.sfz` is *text* referencing **external** samples (`sample=`
  resolved against `default_path` + the file's dir), so it **symlinks** each unique sample into the
  temp dir (copy fallback) — no data copy. Hand-rolled opcode parser handles spaces-in-paths, note
  names (`c4`/`f#3`/`60`), and `//` + `/* */` comments. Details: Regions / Samples / Key range.
- **DLS** (`dls::extract_to_cache`): each `LIST:wave` in the RIFF `wvpl` is an embedded WAV, so it
  rewraps each `fmt `+`data` (+ INFO/INAM name) into a standalone WAV bit-for-bit (no re-encode).
  Details: Instruments / Samples.
- **XI** (`xi::extract_to_cache`): a FastTracker II instrument — `xmrs` parses it
  (`XiInstrument::load` → `to_instrument()` → a core `Instrument`), then each sample's PCM is
  written to a WAV at its C-4-derived native rate (`8363·2^(relative_pitch/12)`). Details: Name /
  Samples. (Reachable because xmrs's `tracker::import::xm::xi_instrument` chain is all `pub`.)
- **XRNS / XRNI** (Renoise song / instrument): **ZIP containers**, so they go through the *archive*
  path, not `is_sample_bank` — `is_archive` includes them and `archive::extract_all` opens `.xrns`/
  `.xrni` **explicitly as ZIP** (`extract_zip` via unarc's `ZipArchive`, since unarc detects format
  by extension and would reject the suffix). You browse the extracted tree; the `SampleData/`
  FLAC/OGG/WAV samples play. Full Renoise song *playback* is out of scope (a DAW engine).

Extraction is synchronous on the UI thread (a huge multi-hundred-MB SF2 can hitch on first open;
it's cached after). **REX/RX2** aren't supported: the format is proprietary (RX2 audio is DWOP-
compressed) — decodable only by porting a reverse-engineered codec; deferred, not shipped.

**Decode cache + async load (CPU-intense work).** A tracker re-synthesizes seconds of audio via
`xmrs`, and a MIDI SoundFont can be 100+ MB to load — both far too slow for the UI thread.
`AudioPlayer::open` is split into the device-free `decode_audio` → `DecodedAudio` (the costly step)
and `AudioPlayer::from_decoded` (opens the rodio device). `ensure_audio_loaded`/`toggle_audio` call
**`start_audio_load`**: a `decode_from_cache` hit builds instantly (a memcpy — path+mtime-keyed LRU
`audio_decode_cache`, ~192 MB), a **miss spawns a worker thread** (reads bytes, loads the MIDI
SoundFont if needed, runs `decode_audio`, sends `DecodedAudio` + the `Arc<SoundFont>` back).
`poll_audio_load` (each frame) caches the result and builds the player; `paint_audio_loading_overlay`
dims the viewport + shows an **animated Spinner + "Loading …"** (delayed 0.2s so quick loads don't
flash) — so a slow load reads as *working*, not frozen. `load_full` cancels a pending load on
navigation. Other CPU work is already cached (thumbnails `thumb_tex`, `img_meta`, archive/soundfont
extraction dirs, recolor tiles, minimap, SAUCE); a *persistent on-disk* thumbnail cache (across
restarts) is the remaining unbuilt win. NB **sample-bank extraction** (entering a big `.sf2`) is
still synchronous — the same worker+spinner pattern could wrap it if it bites.

`rodio → cpal → alsa-sys` needs **`libasound2-dev` at BUILD time** on Linux (added to CI + the
first-time-deps list above), so `rodio`/`xmrs`/`midir` are normal (non-feature-gated) deps
(`rustysynth` is pure-Rust/std-only). The slideshow **auto-advance skips PDFs / audio / source-text**
(you'd lose your place / cut a track) — only images, text-mode art and RIP auto-advance.

**The icy ecosystem (Mike Krüger's scene-art formats), the lean way.** Mike's
`icy_engine` *renders* but hard-depends on `icy_net` → **tokio "full"** — too heavy for
a viewer. His **`icy_parser_core`** is the light alternative (no tokio/egui/image, ~2
extra crates): it only *parses*, emitting into a `CommandSink` trait **we implement**.
So the pattern (see `decode/petscii.rs`) is: depend on `icy_parser_core` (pinned by git
`rev` in `Cargo.toml`), implement `CommandSink` to fill our own buffer/canvas, and
render with *our* code (own fonts/palettes) — keeping the pixel-perfect zoom + thumbnail
quality and a lean binary. PETSCII drives `print`/`emit(TerminalCommand)` into a char
grid + C64 font. **RIP** (`decode/rip.rs`) handles `emit_rip(RipCommand)` — a vector
command stream — with **hand-rolled integer BGI primitives** (lines/rects/circles/
ovals/polys/beziers/arcs/flood-fill/8×8 text) onto a 640×350 EGA canvas, no AA, to keep
RIP's pixel-exact look. The primitives are **ported pixel-for-pixel from icy_engine's
BGI renderer** so outlines close and fills don't leak — getting these exactly right was
the whole game (verified by AE pixel-diff vs icy's reference PNGs; 13/16 scenes ≤ 8.6%,
most ≤ 4%): the **line** is the BGI *run-slice* line (whole H/V runs via `fill_x`/
`fill_y`), not plain Bresenham; **béziers** *truncate* their intermediate samples
(`as i32`, not round) — matching `rip_bezier` exactly is what closed the dragon's body
outline (20% → 0.15%); **ellipses/arcs** use the midpoint traversal (`ellipse_offsets`)
and a **circle truncates** the EGA `ASPECT` (only `arc()` rounds) — the 1px that seals
the dragon's eye (`eye_circle_isolation` guards this). **Flood-fill leak guard
(shape-based, not size-based):** even with matched rasterization a residual 1px gap can
let a fill escape, so `flood()` abandons a leak rather than flood the screen. But
abandoning *every* big fill (the old `> W*H/2` rule) wrongly blanked **legitimate
full-screen backgrounds** — the common case in real BBS art. The fix keys on *shape*: a
leak escapes into a finished scene and must weave around every drawn shape, exploding its
**`perimeter²/area`**, whereas a real background is one solid blob (disk≈12.6, square 16).
`flood()` abandons an over-`W*H/2` region only when `perimeter²/area > 40` — the 15 legit
backgrounds in the corpus sit at 16–20 (HOUND, weaving round the dog, at 65), leaks run
95–2185. This **solved the size-guard's "impossible" case**: jdraw's legit 177k-px fill
(p²/a≈20) now fills (56.9%→0.06% vs reference) while garfield's 170k-px leak (p²/a≈112)
is still abandoned (line art preserved, 2.55%). **Irreducible blind spot (measured, not
tunable):** the metric genuinely *overlaps* — HOUND's legit sky (p²/a≈**65**, a
false-positive that stays unfilled) is *more* complex than the real MSG/SH leaks (≈**50**),
and PMID1's empty-sky leak (≈68) is *simpler* than HOUND. So no threshold both fills HOUND
and blocks MSG/SH — raising the `>40` cut to pass HOUND (tried 66) floods MSG/SH. The
genuine fix is making the outline rasterisation exact so nothing leaks and the guard can go
away entirely — i.e. the wholesale icy BGI-renderer port, **not** a per-file hack or a
threshold nudge. Until then HOUND/PMID1 are accepted known-imperfect (project memory).
**Patterned fills
(two BGI quirks — get both or layered dithered art is wrong):** (1) `fill_poly` borders
the shape in the draw colour **only when that colour isn't 0** (`if self.color != 0`,
matching icy_engine + PabloDraw's `FillPoly`/`BGICanvas`) — drawing it unconditionally
painted a black contour seam around every band of a halftone-shaded portrait whose draw
colour is 0 (e.g. ACiD's `US-HUMA1.RIP`). (2) A BGI fill is **opaque**: `fill_span`/
`flood` paint a pattern's *clear* bits with `bkcolor` (black, the BGI default), **not**
leave them transparent — otherwise a 50% dither (`0x55/0xAA`) drawn over an earlier solid
band lets it bleed through the off-bits and the halftone reads too dense. Solid fills
(pattern `0xFF`) and on-black floods are unaffected, so the reference scenes don't move.
**Text:**
font 0 is the 8×8 bitmap; fonts 1–10 (the spec's scalable BGI fonts) are real stroke
fonts in `rip_chr.rs` — the `.CHR` glyphs are stroke lists we render with the same
`line()`. **Stroke text is always drawn thin + solid** (`draw_text` saves `thick`/
`line_pat`, forces `1`/`0xFFFF`, restores after — exactly icy_engine's `out_text_xy` /
PabloDraw's `OutTextXY`): a preceding `LineStyle thick:3` (common right before a title)
would otherwise bold every glyph stroke into a doubled "shadow" (P1-WNC2's "Wind Ninja
Chronicles" block; the fix also cut shadow 1.1%→0.07%, garfield 3.5%→2.6% vs reference).
**Vertical text** (`FontStyle direction:1`) stacks glyphs **top-to-bottom in a column**:
`rip_chr::draw` advances the pen along the text axis (x rightward for horizontal, *y
downward* for vertical) with the cross-axis fixed, and rotates each glyph (`nx = ox +
st.y`, `ny = oy + st.x`). Advancing `pen_x` for *both* directions was the bug that laid
vertical labels out as overlapping rotated glyphs in a row (MAIN/MSG's left-margin "The
Far Side BBS" + "USA"). NB the downward advance is PabloDraw's RIP convention — icy's BGI
`out_text_xy` goes bottom-to-top; we match Pablo (the reference these were diffed against).
*Known limit:* our vertical glyph transform matches Pablo's **direction** but not its
exact per-pixel placement (Pablo's `DrawCharacter` vertical formula isn't published, and
icy's differs in direction), so art that draws vertical stroke text as an outline then
**flood-fills it solid** via a hard-coded seed (MSG's big "USA" — `Fill{529,119}`) has the
seed miss the letters → "USA" renders as a hollow outline instead of solid. Cosmetic; the
real fix is the wholesale icy BGI-renderer port, not a positioning nudge.
**Buttons:** RIP_BUTTON_STYLE/RIP_BUTTON draw the beveled/recessed/chiseled
panels BBS *menus* are built from (`Btn` + `draw_button`/`button_label`, ported from
icy_engine's `add_button`) — so menu screens render (msg5 ≈ 1.5% vs reference); only the
panel visual is drawn (the mouse region is moot for a static viewer). **Image blits:**
RIP_GET_IMAGE/RIP_PUT_IMAGE (`get_image`/`put_image`) capture a screen rect to a clipboard
and paste it under a Copy/Xor/Or/And/Not mode — ported from icy's `image()`/`put_image`,
with the same *upper-exclusive* capture bounds so tiled stamps stay pixel-aligned. This is
how scenes synthesize textured fills (e.g. paleo XOR-stamps shifted copies of a grab to
make a dither, then tiles it: 72%→42% once implemented). **Bézier note:** sample with
icy's exact float ops (`tf.powf(2.0)`/`powf(3.0)`, *not* `tf*tf`) — the ~1-ULP difference
flips a truncated (`as i32`) sample at integer boundaries and was leaving a 1-px outline
gap. **Leak guard — size can't separate, but *shape* can:** a size threshold provably
fails (jdraw's legit 177,631-px fill is *larger* than garfield's 169,711-px leak, so any
size cut that passes jdraw passes garfield's leak). The guard instead keys on
`perimeter²/area` (see the leak-guard note above): jdraw's solid fill (≈20) passes, garfield's
weaving leak (≈112) is abandoned — so jdraw now fills (0.06% vs ref) *and* garfield keeps its
line art (2.55%). Remaining blind spot — a leak that floods an *empty* region (PMID1) is
shape-simple and slips through; a complex *legit* background (HOUND, weaving round the dog,
≈65) is over the cut and stays unfilled. Those few need the per-file outline-gap fix.
icy_parser_core also has ATASCII / Avatar / PCBoard / Renegade / Viewdata / IGS / SkyPix.

## egui version gotcha

Pinned to `eframe = "0.34"` / `image = "0.25"`. egui renames/moves symbols even
between *patch* releases. If `cargo build` complains about an egui symbol, it
almost certainly just moved — check the egui CHANGELOG for that version rather
than assuming a logic bug. Already hit and migrated for 0.34.3:

- `eframe::App::update(ctx, frame)` → **`App::ui(ui, frame)`** is now the required
  method (`update` is deprecated). The handler is given a root `egui::Ui`, not a
  `Context`; mount panels with `panel.show_inside(ui, ..)` instead of
  `panel.show(ctx, ..)`, and get the context via `ui.ctx().clone()`.
- `InputState::raw_scroll_delta` → `smooth_scroll_delta`. Also: a **Ctrl+wheel is
  pre-classified as a zoom gesture** — it lands in `zoom_delta()` and
  `smooth_scroll_delta` stays `0` (egui's `zoom_modifier` defaults to Ctrl/Cmd).
- `ComboBox::from_id_source` → `from_id_salt`; menu actions live in `egui::menu`.
- `Context::wants_keyboard_input()` → **`egui_wants_keyboard_input()`**. Used in the
  global key handler's `typing` guard so hotkeys (ratings, Backspace→ParentDir, R→random
  pack) are suppressed while a text field is focused — the explicit `path_edit`/`search`
  flags don't cover the 16colo search box / advanced-search fields, only this does.

## Testing

`cargo test` runs 198 tests, all headless (188 unit + 10 GUI; plus 11 `#[ignore]`
network / real-trash tests that hit the live 16colo.rs API or the system trash):
- **Unit tests** (`#[cfg(test)] mod tests` per module): PCX decode + sniff,
  `Registry` dispatch (incl. a real PNG via the `image` crate), `make_thumb` /
  `count_colors`, `PixImage` palette expansion, `rating.rs` parse/encode + a
  guarded xattr round-trip, `sorted_filtered_view` (the sort/filter logic,
  extracted from `rebuild_view` so it's testable without a `PixelView`), `sauce`
  (record + COMNT parsing), `sixteen` (JSON → pieces + URL `#`-encoding), the RIP
  rasterizer (golden-scene guards), `viewdb` round-trips, the `blend_toward` tile-bg
  mix, and the sample-pad grid (`Pad::record`/`from_record` round-trip incl.
  loop/pitch/type, `pad_is_audible` solo/mute truth table, and a `wav_bytes_16` →
  `decode_audio` round-trip proving pad-WAV reload needs no separate reader).
- **GUI tests** (`gui_tests` in `app.rs`, via the `egui_kittest` dev-dep with its
  `eframe` feature): `Harness::builder().build_eframe(|cc| PixelView::new(cc,
  CliArgs::default()))` boots the real app with no window and drives menus through
  AccessKit. Custom-*painted* grid tiles have no a11y label, so kittest covers the
  chrome (menus/dialogs/Preferences), not the tiles.

For a **visual** check on KDE Wayland (KWin has no wlroots screencopy, so `grim`
fails): run under XWayland so `xdotool` can target the window, capture with KDE's
`spectacle`:
`env -u WAYLAND_DISPLAY DISPLAY=:1 ./target/release/pixelview --folder DIR &`,
then `WID=$(DISPLAY=:1 xdotool search --name pixelview)`, drive with
`xdotool`/`ydotool` (you're in the `input` group), `spectacle -b -n -f -o shot.png`.

Note: egui's bundled font lacks some glyphs (e.g. `→` U+2192 → tofu); the emoji
arrows `⬅`/`➡` and `…`/`×`/`›`/`★`/`📁` do render. Prefer those or ASCII in UI strings.
