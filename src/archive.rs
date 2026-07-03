//! Archive browsing: treat `.zip`/`.lha`/`.arj`/… as **virtual folders** by
//! extracting them once into a per-archive temp cache directory, then letting the
//! rest of the app browse that directory like any other folder. This reuses the
//! whole existing pipeline (thumbnailer, decoder, selection) — only navigation
//! (enter the archive, remap breadcrumbs/up) needs to know archives exist.
//!
//! Extraction is read-only and best-effort; per-entry failures are skipped rather
//! than aborting the whole archive. Names are sanitized so a hostile archive can't
//! escape the cache directory (zip-slip).

use std::io;
use std::path::{Path, PathBuf};

/// Browsable (multi-entry) archive extensions, lower-case. Single-file
/// compressions (`.gz`/`.bz2`/`.Z`/`.ice`/`.pi9`/`.sq*`) are intentionally
/// excluded — they decompress to one file, not a folder.
const ARCHIVE_EXTS: &[&str] = &[
    "zip", "7z", "rar", "lha", "lzh", "tar", "ace", "arj", "arc", "pak", "zoo", "ha", "uc2", "sqz",
    "hyp", "tgz", "tbz", "wad",
];

/// True if `path` looks like a browsable archive (by extension, case-insensitive).
pub fn is_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ARCHIVE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Extract every entry of `archive` into a per-archive temp directory and return
/// that directory. Cached: keyed by the archive's path + size + mtime, so an
/// unchanged archive is extracted only once. A `.pv_extracted` marker signals a
/// complete prior extraction (so a half-written cache is redone).
pub fn extract_to_cache(archive: &Path) -> io::Result<PathBuf> {
    let dest = cache_dir(archive)?;
    let marker = dest.join(".pv_extracted");
    if marker.exists() {
        return Ok(dest);
    }
    // Stale/partial cache → start clean.
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest)?;
    extract_all(archive, &dest)?;
    let _ = std::fs::write(&marker, b"");
    Ok(dest)
}

/// The deterministic cache directory for `archive`: `<temp>/pixelview-archives/
/// <stem>-<hash>` where the hash folds in size + mtime so edits invalidate it.
fn cache_dir(archive: &Path) -> io::Result<PathBuf> {
    let meta = std::fs::metadata(archive)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let key = format!("{}|{}|{}", archive.display(), meta.len(), mtime);
    let stem = archive
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
    let safe: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    Ok(std::env::temp_dir()
        .join("pixelview-archives")
        .join(format!("{safe}-{:016x}", hash_str(&key))))
}

fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Walk the archive sequentially, writing each file entry under `dest`.
fn extract_all(archive: &Path, dest: &Path) -> io::Result<()> {
    // Quake PAK / Doom WAD aren't compression archives (unarc-rs can't read them) — they're
    // flat "directory of lumps/files" containers. Detect by magic and extract them ourselves,
    // so the app can browse their sounds (→ waveform tiles), textures (→ image tiles), etc.
    let head = read_head(archive, 4);
    if head.starts_with(b"PACK") {
        return extract_pak(archive, dest);
    }
    if head.starts_with(b"IWAD") || head.starts_with(b"PWAD") {
        return extract_wad(archive, dest);
    }

    use unarc_rs::unified::ArchiveFormat;
    let mut a = ArchiveFormat::open_path(archive).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("open {}: {e}", archive.display()),
        )
    })?;
    while let Some(entry) = a
        .next_entry()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("read entry: {e}")))?
    {
        let name = entry.name().to_string();
        let is_dir = name.ends_with('/') || name.ends_with('\\');
        let Some(out) = safe_join(dest, &name) else {
            continue; // unsafe (zip-slip) or empty → skip
        };
        if is_dir {
            let _ = std::fs::create_dir_all(&out);
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Best-effort per entry: a single bad member shouldn't kill the browse.
        match a.read(&entry) {
            Ok(data) => {
                let _ = std::fs::write(&out, data);
            }
            Err(_) => continue,
        }
    }
    Ok(())
}

/// The first `n` bytes of a file (fewer if shorter), for magic-byte detection.
fn read_head(path: &Path, n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    match std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)) {
        Ok(got) => {
            buf.truncate(got);
            buf
        }
        Err(_) => Vec::new(),
    }
}

fn u32le(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// Quake **PAK**: `"PACK"` + dir offset + dir size, then 64-byte entries
/// (`name[56]` incl. subpaths, `offset`, `size`). Entries carry real filenames, so
/// their `.wav`/`.pcx`/`.tga`/… members render directly once written out.
fn extract_pak(archive: &Path, dest: &Path) -> io::Result<()> {
    let data = std::fs::read(archive)?;
    let dir_off = u32le(&data, 4).unwrap_or(0);
    let dir_size = u32le(&data, 8).unwrap_or(0);
    let count = dir_size / 64;
    for i in 0..count {
        let e = dir_off + i * 64;
        let Some(rec) = data.get(e..e + 64) else {
            break;
        };
        let name_end = rec[..56].iter().position(|&b| b == 0).unwrap_or(56);
        let name = String::from_utf8_lossy(&rec[..name_end]).to_string();
        let off = u32le(rec, 56).unwrap_or(0);
        let size = u32le(rec, 60).unwrap_or(0);
        let (Some(out), Some(bytes)) = (safe_join(dest, &name), data.get(off..off + size)) else {
            continue;
        };
        if let Some(parent) = out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&out, bytes);
    }
    Ok(())
}

/// Doom **WAD**: `"IWAD"/"PWAD"` + lump count + dir offset, then 16-byte entries
/// (`offset`, `size`, `name[8]`). Lumps are raw and un-suffixed, so we sniff each
/// one's magic and give it a real extension (png/jpg/bmp/wav/ogg/txt) — recognizable
/// resources (common in modern ZDoom WADs) then show as tiles; the rest land as `.lmp`.
fn extract_wad(archive: &Path, dest: &Path) -> io::Result<()> {
    let data = std::fs::read(archive)?;
    let count = u32le(&data, 4).unwrap_or(0);
    let dir_off = u32le(&data, 8).unwrap_or(0);
    for i in 0..count {
        let e = dir_off + i * 16;
        let Some(rec) = data.get(e..e + 16) else {
            break;
        };
        let off = u32le(rec, 0).unwrap_or(0);
        let size = u32le(rec, 4).unwrap_or(0);
        let name_end = rec[8..16].iter().position(|&b| b == 0).unwrap_or(8);
        let raw_name = String::from_utf8_lossy(&rec[8..8 + name_end]).to_string();
        // Sanitize the lump name to a filename (WAD names can contain `\`, `/`, etc.).
        let name: String = raw_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if name.is_empty() || size == 0 {
            continue;
        }
        let Some(bytes) = data.get(off..off + size) else {
            continue;
        };
        let file = format!("{}.{}", name, sniff_lump_ext(bytes));
        if let Some(out) = safe_join(dest, &file) {
            let _ = std::fs::write(&out, bytes);
        }
    }
    Ok(())
}

/// Map a lump's leading bytes to a viewable extension, else `lmp` (classic Doom
/// patch/flat/DMX lumps have no standard container, so they stay opaque `.lmp`).
fn sniff_lump_ext(b: &[u8]) -> &'static str {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if b.starts_with(b"BM") {
        "bmp"
    } else if b.starts_with(b"GIF8") {
        "gif"
    } else if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WAVE") {
        "wav"
    } else if b.starts_with(b"OggS") {
        "ogg"
    } else if b.starts_with(b"ID3") || b.starts_with(&[0xFF, 0xFB]) {
        "mp3"
    } else if b
        .iter()
        .take(256)
        .all(|&c| c == b'\t' || c == b'\n' || c == b'\r' || (0x20..0x7f).contains(&c))
    {
        "txt"
    } else {
        "lmp"
    }
}

/// Join a (possibly hostile) archive-internal `name` under `base`, rejecting any
/// `..` that would escape `base`. Backslash separators (DOS archives) are
/// normalized, leading slashes are stripped (absolute paths get reparented under
/// `base`, which is safe). Returns `None` for an empty/escaping path.
fn safe_join(base: &Path, name: &str) -> Option<PathBuf> {
    use std::path::Component;
    let normalized = name.replace('\\', "/");
    let rel = Path::new(normalized.trim_start_matches('/'));
    let mut out = base.to_path_buf();
    let mut depth = 0i32;
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => {
                out.push(c);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return None; // would escape `base`
                }
                out.pop();
                depth -= 1;
            }
            // RootDir / Prefix can't appear after trimming on Unix; reject defensively.
            _ => return None,
        }
    }
    (out != base).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_archive_classifies_by_extension() {
        for ok in ["a.zip", "B.LHA", "pack.arj", "x.7z", "art.zoo", "f.tgz"] {
            assert!(is_archive(Path::new(ok)), "{ok} should be an archive");
        }
        for no in ["a.png", "b.ans", "c.gz", "d.ice", "noext", "e.txt"] {
            assert!(!is_archive(Path::new(no)), "{no} should NOT be an archive");
        }
    }

    #[test]
    fn safe_join_blocks_zip_slip() {
        let base = Path::new("/cache/x");
        assert_eq!(
            safe_join(base, "sub/art.ans"),
            Some(PathBuf::from("/cache/x/sub/art.ans"))
        );
        // DOS backslashes are normalized to nested dirs.
        assert_eq!(
            safe_join(base, "DIR\\PIC.PCX"),
            Some(PathBuf::from("/cache/x/DIR/PIC.PCX"))
        );
        // Leading slash is stripped → reparented safely under base.
        assert_eq!(
            safe_join(base, "/etc/passwd"),
            Some(PathBuf::from("/cache/x/etc/passwd"))
        );
        // A descent then escape that nets back inside is allowed.
        assert_eq!(
            safe_join(base, "a/../b.png"),
            Some(PathBuf::from("/cache/x/b.png"))
        );
        // Escapes are rejected.
        assert_eq!(safe_join(base, "../../etc/passwd"), None);
        assert_eq!(safe_join(base, "a/../../escape"), None);
        assert_eq!(safe_join(base, ""), None);
    }

    #[test]
    fn wad_and_pak_are_archives() {
        assert!(is_archive(Path::new("doom.wad")));
        assert!(is_archive(Path::new("pak0.PAK")));
    }

    #[test]
    fn extracts_a_quake_pak() {
        // Build a minimal PAK: header(12) + file data + 64-byte directory.
        let file_name = b"sound/hi.txt";
        let file_data = b"hello pak";
        let data_off = 12u32;
        let dir_off = 12 + file_data.len() as u32;
        let mut pak = Vec::new();
        pak.extend_from_slice(b"PACK");
        pak.extend_from_slice(&dir_off.to_le_bytes());
        pak.extend_from_slice(&64u32.to_le_bytes()); // one 64-byte entry
        pak.extend_from_slice(file_data);
        let mut name56 = [0u8; 56];
        name56[..file_name.len()].copy_from_slice(file_name);
        pak.extend_from_slice(&name56);
        pak.extend_from_slice(&data_off.to_le_bytes());
        pak.extend_from_slice(&(file_data.len() as u32).to_le_bytes());

        let dir = std::env::temp_dir().join(format!("pv_pak_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let arc = dir.join("test.pak");
        std::fs::write(&arc, &pak).unwrap();
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        extract_all(&arc, &out).unwrap();
        let got = std::fs::read(out.join("sound").join("hi.txt")).unwrap();
        assert_eq!(got, file_data);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wad_lump_extension_sniffing() {
        assert_eq!(sniff_lump_ext(&[0x89, b'P', b'N', b'G', 0, 0]), "png");
        assert_eq!(sniff_lump_ext(b"RIFF\0\0\0\0WAVEfmt "), "wav");
        assert_eq!(sniff_lump_ext(b"MAP01 plain text"), "txt");
        assert_eq!(sniff_lump_ext(&[0, 1, 2, 3, 200]), "lmp");
    }
}
