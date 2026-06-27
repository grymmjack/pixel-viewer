//! Browse <https://16colo.rs> — the ANSI/ASCII art archive — as a virtual disk:
//! Years → Packs → the art files. A pack is just a ZIP, so once downloaded it hands
//! off to the normal archive-browsing path. Only the year/pack *listing* and the
//! pack *download* are new; both use the public JSON API + the static archive.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Virtual root for the 16colo.rs hierarchy. Borrows the `<…>` sentinel style used
/// for built-in palettes — it can never collide with a real on-disk path.
pub const ROOT: &str = "<16colo.rs>";

const API: &str = "https://api.16colo.rs/v1";
const SITE: &str = "https://16colo.rs";

/// Is `path` somewhere inside the virtual 16colo.rs tree?
pub fn is_remote(path: &Path) -> bool {
    path.starts_with(ROOT)
}

/// The path components below [`ROOT`] (e.g. `["2023", "blocktronics-pack"]`).
pub fn rel_parts(path: &Path) -> Vec<String> {
    path.strip_prefix(ROOT)
        .ok()
        .map(|r| {
            r.components()
                .filter_map(|c| c.as_os_str().to_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The years the archive spans (newest first). 16colo.rs runs 1990 → present.
pub fn years() -> Vec<u32> {
    (1990..=2026).rev().collect()
}

/// A pack listing entry: its name and the full URL of its `.zip`.
pub struct Pack {
    pub name: String,
    pub url: String,
}

/// The newest year the archive holds (also the max in [`years`]). Past years are
/// immutable, so their listings can be cached forever; this one refreshes hourly.
const CURRENT_YEAR: u32 = 2026;

fn cache_dir() -> PathBuf {
    std::env::temp_dir().join("pixelview-16colo")
}

/// A disk-cached pack listing for `year`, if present and still fresh.
fn read_packs_cache(year: u32) -> Option<Vec<Pack>> {
    let path = cache_dir().join(format!("packs-{year}.json"));
    let meta = std::fs::metadata(&path).ok()?;
    if year >= CURRENT_YEAR {
        // Recent year: refresh after an hour (new packs may have been uploaded).
        if meta.modified().ok()?.elapsed().ok()? > std::time::Duration::from_secs(3600) {
            return None;
        }
    }
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    Some(
        v.as_array()?
            .iter()
            .filter_map(|p| {
                Some(Pack {
                    name: p["n"].as_str()?.to_owned(),
                    url: p["u"].as_str()?.to_owned(),
                })
            })
            .collect(),
    )
}

fn write_packs_cache(year: u32, packs: &[Pack]) {
    if packs.is_empty() || std::fs::create_dir_all(cache_dir()).is_err() {
        return;
    }
    let arr: Vec<serde_json::Value> = packs
        .iter()
        .map(|p| serde_json::json!({ "n": p.name, "u": p.url }))
        .collect();
    let _ = std::fs::write(
        cache_dir().join(format!("packs-{year}.json")),
        serde_json::Value::Array(arr).to_string(),
    );
}

/// Fetch the packs released in `year` from the JSON API (cached on disk per year;
/// see [`read_packs_cache`]). The endpoint caps a page at 50, so we follow
/// `page.pages` to collect them all.
pub fn fetch_packs(year: u32) -> Result<Vec<Pack>, String> {
    if let Some(cached) = read_packs_cache(year) {
        return Ok(cached);
    }
    let mut packs = Vec::new();
    let mut page = 1u32;
    loop {
        let url = format!("{API}/year/{year}?pagesize=100&page={page}");
        let body = ureq::get(&url)
            .call()
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())?;
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let Some(results) = v["results"].as_array() else {
            break;
        };
        if results.is_empty() {
            break;
        }
        for p in results {
            // The pack name is usually a string, but a JSON *number* when the pack is
            // literally named after one (e.g. "1990").
            let name = p["name"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| p["name"].as_i64().map(|n| n.to_string()));
            let (Some(name), Some(dl)) = (name, p["download"].as_str()) else {
                continue;
            };
            // `download` is absolute on the year endpoint, site-relative elsewhere.
            let url = if dl.starts_with("http") {
                dl.to_string()
            } else {
                format!("{SITE}{dl}")
            };
            packs.push(Pack { name, url });
        }
        let pages = v["page"]["pages"].as_u64().unwrap_or(1) as u32;
        page += 1;
        if page > pages || page > 60 {
            break;
        }
    }
    packs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    write_packs_cache(year, &packs);
    Ok(packs)
}

/// The conventional zip URL for a pack, used as a fallback when its listing entry
/// isn't cached (e.g. a path typed/navigated directly).
pub fn pack_url(year: u32, pack: &str) -> String {
    format!("{SITE}/archive/{year}/{pack}.zip")
}

/// Download `url` into a per-URL temp cache file and return its path. Cached: an
/// already-downloaded pack is reused rather than re-fetched.
pub fn download(url: &str) -> Result<PathBuf, String> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(format!("{:016x}.zip", hash(url)));
    if dest.exists() {
        return Ok(dest);
    }
    let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(256 * 1024 * 1024) // sanity cap: 256 MB
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, &buf).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// The virtual sub-roots the nav bar exposes besides the year list.
pub const GROUPS: &str = "groups";
pub const ARTISTS: &str = "artists";
pub const LATEST: &str = "latest";
pub const SEARCH: &str = "search";

/// Minimal percent-encoding for an API path segment (group/artist names can have
/// spaces and punctuation).
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let body = ureq::get(url)
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

/// Walk a paginated endpoint (`base` already carries its query, sans `page`), calling
/// `f` on every item across all pages (`page.pages`), capped so a runaway can't spin.
fn paginate<F: FnMut(&serde_json::Value)>(base: &str, mut f: F) -> Result<(), String> {
    let sep = if base.contains('?') { '&' } else { '?' };
    let mut page = 1u32;
    loop {
        let v = get_json(&format!("{base}{sep}page={page}"))?;
        match v["results"].as_array() {
            Some(r) if !r.is_empty() => r.iter().for_each(&mut f),
            _ => break,
        }
        let pages = v["page"]["pages"].as_u64().unwrap_or(1) as u32;
        page += 1;
        if page > pages || page > 80 {
            break;
        }
    }
    Ok(())
}

/// Every group name on 16colo.rs (sorted, case-insensitive). The `/v1/group` list is
/// an array of single-key objects `{ "<group>": { releases, … } }`.
pub fn fetch_groups() -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    paginate(&format!("{API}/group?pagesize=100"), |item| {
        if let Some(obj) = item.as_object() {
            names.extend(obj.keys().cloned());
        }
    })?;
    names.sort_by_key(|s| s.to_lowercase());
    names.dedup();
    Ok(names)
}

/// A group's packs (newest year first) as downloadable [`Pack`]s. `/v1/group/:name`
/// returns `{ results: { packs: { "<year>": ["<pack>", …] } } }`.
pub fn fetch_group_packs(group: &str) -> Result<Vec<Pack>, String> {
    let v = get_json(&format!("{API}/group/{}?pagesize=100", enc(group)))?;
    let mut packs = Vec::new();
    if let Some(by_year) = v["results"]["packs"].as_object() {
        let mut years: Vec<&String> = by_year.keys().collect();
        years.sort_by(|a, b| b.cmp(a)); // newest first
        for y in years {
            let year: u32 = y.parse().unwrap_or(0);
            if let Some(list) = by_year[y].as_array() {
                for p in list.iter().filter_map(|p| p.as_str()) {
                    packs.push(Pack {
                        name: p.to_string(),
                        url: pack_url(year, p),
                    });
                }
            }
        }
    }
    Ok(packs)
}

/// Every artist name on 16colo.rs (sorted). `/v1/artist` items are `{ artist: { name } }`.
pub fn fetch_artists() -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    paginate(&format!("{API}/artist?pagesize=100"), |item| {
        if let Some(n) = item["artist"]["name"].as_str() {
            names.push(n.to_string());
        }
    })?;
    names.sort_by_key(|s| s.to_lowercase());
    names.dedup();
    Ok(names)
}

/// The packs an artist contributed to (newest year first, de-duplicated). The
/// `/v1/artist/:name` body is `{ results: { "<year>": { "<pack>": { files, … } } } }`;
/// we surface the packs (each opens via the normal zip flow) rather than loose pieces.
pub fn fetch_artist_packs(artist: &str) -> Result<Vec<Pack>, String> {
    let v = get_json(&format!("{API}/artist/{}?pagesize=100", enc(artist)))?;
    let mut packs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(by_year) = v["results"].as_object() {
        let mut years: Vec<&String> = by_year.keys().collect();
        years.sort_by(|a, b| b.cmp(a));
        for y in years {
            let year: u32 = y.parse().unwrap_or(0);
            if let Some(map) = by_year[y].as_object() {
                for name in map.keys() {
                    if seen.insert(name.clone()) {
                        packs.push(Pack {
                            name: name.clone(),
                            url: pack_url(year, name),
                        });
                    }
                }
            }
        }
    }
    Ok(packs)
}

/// The most recent releases (packs), newest first. `/v1/latest/releases` items carry
/// `pack` + an absolute `download` URL.
pub fn fetch_latest() -> Result<Vec<Pack>, String> {
    let v = get_json(&format!("{API}/latest/releases?pagesize=50"))?;
    let mut packs = Vec::new();
    if let Some(results) = v["results"].as_array() {
        for p in results {
            if let (Some(name), Some(dl)) = (p["pack"].as_str(), p["download"].as_str()) {
                let url = if dl.starts_with("http") {
                    dl.to_string()
                } else {
                    format!("{SITE}{dl}")
                };
                packs.push(Pack {
                    name: name.to_string(),
                    url,
                });
            }
        }
    }
    Ok(packs)
}

/// Server-side **substring** search for artist names matching `query` — the API's
/// `?filter=` does the matching (9k+ artists, so we never fetch the full list). One
/// page (100) of hits is plenty for a picker.
pub fn search_artists(query: &str) -> Result<Vec<String>, String> {
    let v = get_json(&format!("{API}/artist?pagesize=100&filter={}", enc(query)))?;
    let mut names = Vec::new();
    if let Some(results) = v["results"].as_array() {
        for x in results {
            if let Some(n) = x["artist"]["name"].as_str() {
                names.push(n.to_string());
            }
        }
    }
    Ok(names)
}

/// Server-side substring search for group names matching `query` (each result is a
/// single-key `{ "<group>": {…} }` object, like the group list).
pub fn search_groups(query: &str) -> Result<Vec<String>, String> {
    let v = get_json(&format!("{API}/group?pagesize=100&filter={}", enc(query)))?;
    let mut names = Vec::new();
    if let Some(results) = v["results"].as_array() {
        for x in results {
            if let Some(obj) = x.as_object() {
                names.extend(obj.keys().cloned());
            }
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_parts_splits_the_virtual_path() {
        assert!(is_remote(Path::new(ROOT)));
        assert_eq!(rel_parts(Path::new(ROOT)), Vec::<String>::new());
        let p = Path::new(ROOT).join("2023").join("some-pack");
        assert_eq!(rel_parts(&p), vec!["2023", "some-pack"]);
        assert!(!is_remote(Path::new("/home/x")));
    }

    #[test]
    #[ignore = "hits the live 16colo.rs API + network"]
    fn fetch_and_cache_packs_live() {
        let _ = std::fs::remove_file(cache_dir().join("packs-1990.json"));
        let packs = fetch_packs(1990).expect("fetch 1990");
        assert!(
            packs.iter().any(|p| p.name == "acdu1190"),
            "1990 should contain acdu1190"
        );
        assert!(
            cache_dir().join("packs-1990.json").exists(),
            "disk cache written"
        );
        // Second call is served from the cache (past year → no network) and matches.
        let again = fetch_packs(1990).expect("cached 1990");
        assert_eq!(packs.len(), again.len());
    }

    #[test]
    fn years_are_descending() {
        let y = years();
        assert_eq!(y.first().copied(), Some(2026));
        assert!(y.windows(2).all(|w| w[0] > w[1]));
    }
}
