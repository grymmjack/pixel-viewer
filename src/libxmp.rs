//! Bundled **libxmp** FFI: play the extra tracker formats `xmrs` doesn't
//! (669 / FAR / OKT / MED / AMF / ULT / MTM / STM / …) by rendering them to PCM. The C library
//! is compiled from `vendor/libxmp` by `build.rs`. Each render spins up its own context (no
//! shared state), so it's safe to call from the audio decode worker thread.

use std::os::raw::{c_int, c_long, c_void};

type XmpContext = *mut c_void;

extern "C" {
    fn xmp_create_context() -> XmpContext;
    fn xmp_free_context(ctx: XmpContext);
    fn xmp_load_module_from_memory(ctx: XmpContext, mem: *const c_void, size: c_long) -> c_int;
    fn xmp_release_module(ctx: XmpContext);
    fn xmp_start_player(ctx: XmpContext, rate: c_int, format: c_int) -> c_int;
    fn xmp_play_buffer(ctx: XmpContext, buffer: *mut c_void, size: c_int, loops: c_int) -> c_int;
    fn xmp_end_player(ctx: XmpContext);
    fn xmp_get_frame_info(ctx: XmpContext, info: *mut c_void);
    fn xmp_get_module_info(ctx: XmpContext, info: *mut c_void);
}

/// Lightweight module metadata (no audio synthesis) for the Details pane + grid hover.
#[derive(Clone, Default)]
pub struct TrackerInfo {
    pub channels: u32,
    pub patterns: u32,
    pub instruments: u32,
    pub samples: u32,
    pub length: u32,      // module length in patterns (positions in the order list)
    pub duration_ms: i32, // libxmp's computed total replay time
    pub format: String,   // module `type` string (e.g. "Protracker", "Scream Tracker 3")
}

/// libxmp's computed total replay time in ms (the `total_time` field at byte offset 32 of
/// `struct xmp_frame_info`). We over-allocate an 8-byte-aligned buffer (the struct has an
/// interior pointer + a 64-entry channel array — ~2.6 KB) and read that one field, avoiding a
/// full struct-layout FFI declaration.
unsafe fn total_time_ms(ctx: XmpContext) -> i32 {
    let mut buf = [0u64; 512]; // 4096 bytes, 8-aligned — comfortably larger than the struct
    xmp_get_frame_info(ctx, buf.as_mut_ptr() as *mut c_void);
    let bytes = std::slice::from_raw_parts(buf.as_ptr() as *const u8, 4096);
    i32::from_ne_bytes([bytes[32], bytes[33], bytes[34], bytes[35]])
}

/// Render a module in `bytes` to interleaved-stereo `f32` at 44.1 kHz. Returns `(samples,
/// channels=2, rate)` or `None` if libxmp can't load it / it produced no audio.
pub fn render(bytes: &[u8]) -> Option<(Vec<f32>, u16, u32)> {
    const RATE: c_int = 44_100;
    let cap = RATE as usize * 2 * 600; // 10-minute stereo safety cap
    unsafe {
        let ctx = xmp_create_context();
        if ctx.is_null() {
            return None;
        }
        // No RAII across the FFI boundary, so free the context on every path.
        let out = render_into(ctx, bytes, RATE, cap);
        xmp_free_context(ctx);
        out.map(|samples| (samples, 2u16, RATE as u32))
    }
}

/// # Safety
/// `ctx` must be a live context from `xmp_create_context`. Loads the module, plays it out into a
/// growing buffer, and tears the player/module down (but not the context — the caller frees it).
unsafe fn render_into(ctx: XmpContext, bytes: &[u8], rate: c_int, cap: usize) -> Option<Vec<f32>> {
    if xmp_load_module_from_memory(ctx, bytes.as_ptr() as *const c_void, bytes.len() as c_long) != 0
    {
        return None; // not a module libxmp recognizes
    }
    if xmp_start_player(ctx, rate, 0) != 0 {
        // format 0 = signed 16-bit, stereo, interleaved
        xmp_release_module(ctx);
        return None;
    }
    // Render exactly the song's computed length (many modules loop forever, so relying on the
    // end marker would run to the hard cap). +2s tail; fall back to 5 min if unknown; hard cap.
    let total_ms = total_time_ms(ctx);
    let target = if total_ms > 0 {
        (total_ms as usize + 2000) * rate as usize / 1000
    } else {
        rate as usize * 300
    }
    .saturating_mul(2)
    .min(cap); // in interleaved samples (× 2 for stereo)
    let mut out: Vec<f32> = Vec::new();
    let mut buf = [0i16; 8192]; // interleaved S16 stereo scratch
    loop {
        // `size` is in BYTES. The last arg is a LOOP COUNT, not a bool: `1` = play through once
        // then stop (returns -1); `0` would play forever (only a non-looping module ever ends).
        let ret = xmp_play_buffer(
            ctx,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * 2) as c_int,
            1,
        );
        if ret != 0 {
            break;
        }
        out.extend(buf.iter().map(|&s| s as f32 / 32768.0));
        if out.len() >= target {
            break;
        }
    }
    xmp_end_player(ctx);
    xmp_release_module(ctx);
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Parse a module's structure (channels / patterns / instruments / samples / length + duration)
/// **without** synthesizing audio — cheap enough for a grid-hover tooltip. Returns `None` if
/// libxmp can't load the bytes. Reads the `xmp_module_info` / `xmp_module` C structs by field
/// offset (like `total_time_ms`) to avoid declaring the full struct layout over FFI.
pub fn module_info(bytes: &[u8]) -> Option<TrackerInfo> {
    unsafe {
        let ctx = xmp_create_context();
        if ctx.is_null() {
            return None;
        }
        let out = module_info_into(ctx, bytes);
        xmp_free_context(ctx);
        out
    }
}

/// # Safety
/// `ctx` must be a live context. Loads the module, reads its structure, and tears it down.
unsafe fn module_info_into(ctx: XmpContext, bytes: &[u8]) -> Option<TrackerInfo> {
    if xmp_load_module_from_memory(ctx, bytes.as_ptr() as *const c_void, bytes.len() as c_long) != 0
    {
        return None;
    }
    // start_player scans the sequence to compute its duration (no audio synthesis); it may fail on
    // an odd module, in which case we still report the structure with duration 0.
    let started = xmp_start_player(ctx, 44_100, 0) == 0;
    let duration_ms = if started { total_time_ms(ctx) } else { 0 };
    // `struct xmp_module_info`: the `struct xmp_module *mod` pointer sits at byte offset 24
    // (md5[16] + int vol_base + 4 bytes padding to 8-align the pointer on LP64).
    let mut mi = [0u64; 8]; // 64 bytes, 8-aligned — larger than the struct
    xmp_get_module_info(ctx, mi.as_mut_ptr() as *mut c_void);
    let mod_ptr = *(mi.as_ptr() as *const u8).add(24).cast::<usize>() as *const u8;
    let info = if mod_ptr.is_null() {
        None
    } else {
        // `struct xmp_module`: name[64], type[64], then int fields — pat@128, trk@132, chn@136,
        // ins@140, smp@144, spd@148, bpm@152, len@156.
        let read_i32 = |off: usize| mod_ptr.add(off).cast::<i32>().read_unaligned();
        let type_bytes = std::slice::from_raw_parts(mod_ptr.add(64), 64);
        let end = type_bytes.iter().position(|&b| b == 0).unwrap_or(64);
        let format = String::from_utf8_lossy(&type_bytes[..end])
            .trim()
            .to_string();
        Some(TrackerInfo {
            patterns: read_i32(128).max(0) as u32,
            channels: read_i32(136).max(0) as u32,
            instruments: read_i32(140).max(0) as u32,
            samples: read_i32(144).max(0) as u32,
            length: read_i32(156).max(0) as u32,
            duration_ms,
            format,
        })
    };
    if started {
        xmp_end_player(ctx);
    }
    xmp_release_module(ctx);
    info
}
