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
        let ret = xmp_play_buffer(ctx, buf.as_mut_ptr() as *mut c_void, (buf.len() * 2) as c_int, 1);
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
