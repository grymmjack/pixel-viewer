//! Per-format **accent colors** — the tint used for a file type's badge, waveform, and
//! music-note tile. Held in a process-global map (primed from settings on startup, read by the
//! grid/table *and* the off-thread thumbnailer) and user-editable in Preferences → Format colors.

use eframe::egui;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

static COLORS: OnceLock<RwLock<HashMap<String, [u8; 3]>>> = OnceLock::new();

fn map() -> &'static RwLock<HashMap<String, [u8; 3]>> {
    COLORS.get_or_init(|| RwLock::new(defaults()))
}

/// The accent color (RGB) for a file extension; falls back to the "other" entry.
pub fn color(ext: &str) -> [u8; 3] {
    let m = map().read().unwrap();
    m.get(&ext.to_ascii_lowercase())
        .copied()
        .or_else(|| m.get("other").copied())
        .unwrap_or([80, 200, 220])
}

/// The accent color as an egui `Color32`.
pub fn color32(ext: &str) -> egui::Color32 {
    let [r, g, b] = color(ext);
    egui::Color32::from_rgb(r, g, b)
}

/// Override a format's color (from the Preferences picker).
pub fn set(ext: &str, rgb: [u8; 3]) {
    map().write().unwrap().insert(ext.to_ascii_lowercase(), rgb);
}

/// Reset every format to its built-in default.
pub fn reset_all() {
    *map().write().unwrap() = defaults();
}

/// The formats surfaced in Preferences, in a stable grouped order: `(ext, label)`.
pub const EDITABLE: &[(&str, &str)] = &[
    ("wav", "WAV"),
    ("mp3", "MP3"),
    ("ogg", "OGG / OGA"),
    ("flac", "FLAC"),
    ("mid", "MIDI"),
    ("rad", "RAD (AdLib)"),
    ("mod", "MOD"),
    ("s3m", "S3M"),
    ("xm", "XM"),
    ("it", "IT"),
    ("sf2", "SoundFont"),
    ("sfz", "SFZ"),
    ("dls", "DLS"),
    ("xi", "XI"),
    ("xrni", "XRNI"),
    ("xrns", "XRNS"),
    ("other", "Other formats"),
];

/// Extensions that mirror an editable key (so a picker for "ogg" also sets "oga", "mid" sets
/// "midi"/"kar"/"rmi", etc.) — keeps aliases in sync with the one control shown.
pub fn aliases(ext: &str) -> &'static [&'static str] {
    match ext {
        "ogg" => &["ogg", "oga"],
        "mid" => &["mid", "midi", "kar", "rmi"],
        _ => &[],
    }
}

/// Persisted overrides — the entries that differ from the defaults, as `"ext r g b"` records.
pub fn overrides_record() -> Vec<String> {
    let d = defaults();
    map()
        .read()
        .unwrap()
        .iter()
        .filter(|(k, v)| d.get(*k) != Some(*v))
        .map(|(k, v)| format!("{} {} {} {}", k, v[0], v[1], v[2]))
        .collect()
}

/// Apply persisted overrides (from `overrides_record`) on startup.
pub fn apply_records(records: &[String]) {
    let mut m = map().write().unwrap();
    for rec in records {
        let parts: Vec<&str> = rec.split_whitespace().collect();
        if let [ext, r, g, b] = parts[..] {
            if let (Ok(r), Ok(g), Ok(b)) = (r.parse(), g.parse(), b.parse()) {
                m.insert(ext.to_ascii_lowercase(), [r, g, b]);
            }
        }
    }
}

fn defaults() -> HashMap<String, [u8; 3]> {
    let entries: &[(&str, [u8; 3])] = &[
        ("wav", [76, 200, 100]),   // green
        ("mp3", [240, 150, 45]),   // orange
        ("ogg", [150, 225, 130]),  // light green
        ("oga", [150, 225, 130]),  //   (alias)
        ("flac", [100, 205, 175]), // teal-green
        ("mid", [240, 130, 190]),  // pink
        ("midi", [240, 130, 190]),
        ("kar", [240, 130, 190]),
        ("rmi", [240, 130, 190]),
        ("rad", [255, 95, 175]), // bright pink
        ("mod", [70, 130, 245]), // bright blue
        ("s3m", [55, 215, 235]), // bright cyan
        ("xm", [55, 220, 185]),  // bright teal
        ("it", [90, 240, 200]),  // brighter teal
        ("sf2", [170, 110, 225]), // purple
        ("sfz", [200, 160, 240]), // light purple
        ("dls", [70, 200, 220]),  // cyan
        ("xi", [225, 205, 70]),   // yellow
        ("xrni", [250, 230, 55]), // bright yellow
        ("xrns", [240, 70, 70]),  // bright red
        ("other", [80, 200, 220]), // cyan
    ];
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect()
}
