# pixelview

A fast, pixel-art-first image viewer in Rust + [egui/eframe](https://github.com/emilk/egui).
This is a working scaffold: a pluggable decoder registry, a virtualized
thumbnail grid, and a nearest-neighbor zoom view. It's meant to be handed to
[Claude Code](https://docs.anthropic.com/en/docs/claude-code) and grown.

## Build & run

```sh
cargo run --release
```

On Debian/KDE you may need the eframe/winit system deps once:

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

eframe defaults to the **wgpu** backend, which is the right call here — it's what
gives us pixel-perfect nearest-neighbor textures. It runs fine on KDE Plasma 6 /
Wayland. If you ever hit a GPU/driver issue, you can switch eframe to the `glow`
renderer via a feature flag.

## Controls

- **Open folder** — scans for any extension a decoder claims.
- **Click a thumbnail** — opens the single-image view.
- **Mouse wheel / pinch** — zoom (anchored on the view, nearest-neighbor).
- **Drag** — pan. **1:1** resets zoom. **← / →** — previous / next image.
- **⬅ Grid** — back to the tile view.

## The two pixel-art decisions baked in

1. **Nearest-neighbor everywhere it's displayed.** Textures are uploaded with
   `TextureOptions::NEAREST`, so zooming never smears. Thumbnails are scaled by a
   nearest sampler that integer-up-scales small sprites (so a 16×16 sprite fills
   its tile crisply) — see `make_thumb` in `src/thumb.rs`.
2. **Palettes are preserved, not flattened.** `PixImage` keeps the original
   `indices` + `palette` in `indexed` when a format is palette-based, alongside
   the RGBA used for display (`src/image_types.rs`). That's what makes palette
   swap / cycling / accurate re-export possible later. The PCX decoder fills this
   in; the `image`-crate decoder leaves it `None`.

## Architecture map

```
src/
  main.rs           eframe entry / window setup
  app.rs            PixelView: eframe::App, grid + single views, texture cache
  image_types.rs    PixImage (RGBA + optional indexed/palette)
  thumb.rs          background worker pool + nearest-neighbor thumbnailer
  decode/
    mod.rs          Decoder trait + Registry (sniff-then-extension dispatch)
    builtin.rs      image-crate decoder: png/gif/bmp/jpeg/webp/tga/tiff/pnm/qoi
    pcx.rs          hand-written, palette-preserving PCX decoder
```

Decoding/scaling run on worker threads; only CPU pixel buffers cross back to the
UI thread, which uploads them to GPU textures lazily. The grid is virtualized via
`ScrollArea::show_rows`, so a folder with thousands of images only ever renders
the visible rows.

## How to extend

**Add a format** (e.g. Amiga IFF/ILBM, LBM, ANSI): copy `decode/pcx.rs`,
implement the `Decoder` trait, and add one line to `Registry::with_builtins` in
`decode/mod.rs`. PCX is the reference for the palette-preserving path. The `image`
crate also has a `hooks` module if you'd rather register a decoder with it
directly.

**Add a view** (palette inspector, sprite-sheet slicer, GIF playback): right now
the two views are methods on `PixelView` (`ui_grid`, `ui_single`) switched by the
`Mode` enum. To make views truly pluggable, lift them behind a small trait —
`trait View { fn ui(&mut self, app: &mut AppState, ui: &mut Ui); }` — and hold a
`Box<dyn View>`. The decoder registry is the pattern to mirror.

## Known next steps (good Claude Code tasks)

- **Persistent thumbnail cache** keyed on `(path, mtime, size)` so thumbnails
  survive restarts and invalidate on edit — basically your `pixelart-thumbcache`
  approach, on disk. The in-memory `requested` set is the hook to replace.
- **Texture atlas** for very large folders (one texture per thumbnail is fine for
  hundreds, not tens of thousands).
- **Integer box-average** thumbnail option alongside nearest, for softer minified
  previews — swap the one `make_thumb` function.
- **Palette inspector** view that reads `PixImage::indexed`.
- **Animation**: GIF currently decodes to its first frame only.

## Version note

Pinned to `eframe = "0.34"` and `image = "0.25"` (current as of mid-2026). egui
moves fast and renames things between releases — if `cargo build` complains about
an egui symbol (e.g. `raw_scroll_delta`, `ColorImage::from_rgba_unmultiplied`,
`TextureOptions::NEAREST`), it almost certainly just moved; check the egui
CHANGELOG for that version and adjust. Everything else is stable std/Rust.
