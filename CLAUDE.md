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
SAUCE-driven hints, shown in the Details pane.
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
  thumb.rs           worker pool: thumbnails + image metadata (dims, color count)
  colo_thumb.rs      RemoteThumbs: HTTP worker pool fetching 16colo.rs `tn` PNGs
                     (mirrors ThumbBuilder; results uploaded to thumb_tex by path)
  rating.rs          read/write star ratings via the user.baloo.rating xattr
  ratings.rs         cross-platform ratings sidecar (ratings.json) for virtual art
  anim.rs            decode animated GIF frames + per-frame delays
  decode/
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
    xbin.rs          .xb/.xbin — binary ANSI: palette/font + RLE; shared render_textmode
    bin.rs           .bin — raw char/attr pairs (SAUCE width); idf/adf reuse render_textmode
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
cargo test               # 79 tests (76 unit + 3 headless egui_kittest GUI tests; +1 ignored real-trash)
cargo test gui_tests     # just the egui_kittest UI tests; cargo test <name> for one
```

First-time eframe/winit system deps on Debian/KDE:
```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
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
right-click-removable in both the toolbar and the Places dock. The explorer's
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
`pin_dir`/`smart_on` locals applied after the tile closure.

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
Columns key off a `ColKind` (not a position), so the **file** layout's optional columns
are user-toggled via a `TC_*` bitmask (`table_columns`, persisted; Preferences → "Table
columns") while Name + thumbnail are always shown; archive rows (.zip/…) render the
folder glyph + a format badge like the grid. In the scene layout the **Pack / Year /
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
  `step_device_zoom`. Raster art instead keeps a logical `%` zoom remembered across
  images (`raster_zoom`, persisted as the old `IMG_ZOOM_KEY`; `draw_image_view` writes
  it back each frame), whereas text-mode always reopens at its preference (manual zoom
  is transient). Both persist (`TEXTMODE_ZOOM_KEY`). `viewing_textmode` tracks the kind.
- **CRT aspect** (`crt_aspect`, `CRT_ASPECT_KEY`, **off by default**) is a viewer-only
  toggle shown in the status bar **only for text-mode art**: it stretches the blit
  ≈1.2× vertically (an 80×25 8×16 grid → 4:3) to match non-square VGA pixels — snapped
  to an integer device-pixel scale (see below), so it lands near 1.2× while staying
  crisp. It's a *display* scale, not a zoom change, and never touches the texture.
- **9px VGA cell** (`font_9px`, `FONT_9PX_KEY`, **off by default**) is a separate
  status-bar toggle (next to CRT, text-mode art only) that renders the 8-pixel CP437
  glyph in a **9-dot-wide cell**, the way real VGA text mode did: the 9th column is
  background except for the line-draw range `0xC0..=0xDF`, where it repeats column 8 so
  box rules join (see `ansi::dot_on`). Off = exact 8px cells. Unlike CRT this is a
  *decode-time* change (the texture width grows ~12.5%, 80 cols → 720px), so it's a
  process-global flag (`decode::set_font_9px`, read by `AnsiDecoder`) primed from
  storage on startup; toggling it calls `redecode_full` to rebuild the viewer texture
  in place (keeping zoom/pan). This is why ansilove/16colo (9-dot) render wider than a
  naive 8px blit. Thumbnails aren't re-rendered on toggle (sub-pixel at thumb scale).
- **Baud-rate playback** (ANSImation / "watch RIP draw"). The whole engine is "render
  the first N bytes into a fixed-size canvas": `ansi::TextStream` and `rip::RipStream`
  parse a byte *prefix* (canvas sized from the whole file so frames don't resize). The
  viewer's `Player` (parallel to the GIF `AnimState`) holds the stream + a byte cursor
  advanced at `Baud::cps()` × dt, caching the frame texture by cursor position. cps =
  baud/10 (8N1) **× a V.42bis compression factor** (`(baud/3600).clamp(1,4)`): real
  modems compressed ANSI ~4:1, so 14.4k+ "feels fast" like a real board while ≤2400
  stays an authentic crawl. Rates are the modem ladder (300/1200/2400/4800/9600/14.4k/
  28.8k/33.6k/56k/115.2k); RIP and ANSI keep **independent** remembered speeds
  (`baud_ansi`/`baud_rip`, picked by `Stream::is_rip()`). `Stream::for_file`
  makes a player only for RIP + ANSI/CP437 streams (NOT the binary text-mode formats —
  XBin/BIN/IDF/PETSCII aren't character streams). A controls row (▶/⏸ · ⏮ Replay · byte
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
- **Slideshow** (`auto_next` + `auto_next_secs` 1/3/5/10s, status bar, persisted). In the
  single view, once the file has *settled* (any baud transmission finished — `Player.playing`
  is the "busy" gate) and the delay elapses, `ui_single` steps to the next file. Dwell
  resets on load and while busy. While slideshow is on, RIP + raster (non-text-mode) art
  opens **fit-to-screen** (`load_full` sets `fit_requested`) so it's fully visible; text-mode
  keeps its readable zoom + fit-to-width. Great for flipping through a whole pack hands-free.
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

## Font glyph gotcha (read before adding any icon glyph)

The bundled egui font **lacks the Geometric Shapes block** — `▲`/`▼` (U+25B2/25BC),
`●` (U+25CF) etc. render as tofu (`□`). Confirmed-rendering glyphs: the emoji arrows
`⬅`/`➡`/**`⬆`**/**`⬇`**, `⟲`/`⟳`, `…`/`×`/`›`/`★`/`📁`, `·`. For anything else prefer
ASCII (`*`) or **paint it** (see `drag_handle`'s dots). When in doubt, test in the
real app — tofu has bitten this UI several times.

## Settings & ratings

- **Persistence** uses eframe's `persistence` feature (storage at
  `~/.local/share/pixelview/`). Each setting is its own key (consts on `PixelView`:
  `ZOOM_KEY`, `THUMB_KEY`, `FAV_KEY`, `FOLDER_KEY`, sort/filter keys, `EXPLORER_KEY`,
  `DETAILS_KEY`, `GAP_KEY`/`GAP_Y_KEY` (h/v grid spacing), `CAPTION_KEY` (caption
  bitmask), `KEYMAP_KEY`).
  `persist_egui_memory()` returns `false` — we persist only our keys, in `save()`.
- **Two independent zoom axes:** Ctrl +/- = whole-GUI scale (egui `zoom_factor`);
  Ctrl+wheel = thumbnail tile size only. Ctrl+wheel arrives as `zoom_delta()`, NOT
  `smooth_scroll_delta` (see gotcha).
- **Ratings** (`rating.rs`) read/write `user.baloo.rating` (ASCII `0..10`, 2 per
  star) — the KDE Baloo / Gwenview scheme, so they interoperate with Gwenview and
  the user's `~/git/qb64pe-lab/greywood/sort-by-rating.sh`. Keys 1–5 set, 0 clears
  (removes the attr). Grid rates the selection (or hovered tile); single view rates
  the current image.
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
`ImageCrateDecoder` if its magic bytes could be ambiguous.

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

## Testing

`cargo test` runs 79 tests, all headless (plus 1 `#[ignore]` real-trash round-trip):
- **Unit tests** (`#[cfg(test)] mod tests` per module): PCX decode + sniff,
  `Registry` dispatch (incl. a real PNG via the `image` crate), `make_thumb` /
  `count_colors`, `PixImage` palette expansion, `rating.rs` parse/encode + a
  guarded xattr round-trip, and `sorted_filtered_view` (the sort/filter logic,
  extracted from `rebuild_view` so it's testable without a `PixelView`).
- **GUI tests** (`gui_tests` in `app.rs`, via the `egui_kittest` dev-dep with its
  `eframe` feature): `Harness::builder().build_eframe(|cc| PixelView::new(cc,
  CliArgs::default()))` boots the real app with no window and drives menus through
  AccessKit. Custom-*painted* grid tiles have no a11y label, so kittest covers the
  chrome (menus/dialogs), not the tiles.

For a **visual** check on KDE Wayland (KWin has no wlroots screencopy, so `grim`
fails): run under XWayland so `xdotool` can target the window, capture with KDE's
`spectacle`:
`env -u WAYLAND_DISPLAY DISPLAY=:1 ./target/release/pixelview --folder DIR &`,
then `WID=$(DISPLAY=:1 xdotool search --name pixelview)`, drive with
`xdotool`/`ydotool` (you're in the `input` group), `spectacle -b -n -f -o shot.png`.

Note: egui's bundled font lacks some glyphs (e.g. `→` U+2192 → tofu); the emoji
arrows `⬅`/`➡` and `…`/`×`/`›`/`★`/`📁` do render. Prefer those or ASCII in UI strings.
