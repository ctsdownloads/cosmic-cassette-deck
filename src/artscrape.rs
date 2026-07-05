//! Album-art scraper: MusicBrainz recording search -> Cover Art Archive.
//!
//! Blocking HTTP (ureq) run from a dedicated background thread, one lookup
//! at a time with polite rate limiting (MusicBrainz asks ≤1 req/sec). Every
//! result - image bytes or a "no art" marker - is cached to disk so a given
//! artist/title is only ever fetched once across runs.
//!
//! Flow per album:
//!   1. cache hit?  -> return bytes (or skip if negative-cached)
//!   2. MB search   -> best release id for "artist" + "title"
//!   3. CAA fetch   -> front-cover thumbnail bytes
//!   4. write cache (bytes, or an empty ".none" marker on miss)
//!
//! The GUI hands the scraper a list of (key, artist, title) and a channel;
//! each fetched cover is sent back as (key, png_bytes) for live display.

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const USER_AGENT: &str = "CosmicCassetteDeck/0.3 (github.com/ctsdownloads)";
const MB_MIN_INTERVAL: Duration = Duration::from_millis(1500); // gentle; avoids throttle
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);

/// One album needing art. `key` is the album's stable identity (artist\ttitle),
/// used as both the dedup key and the cache filename stem.
#[derive(Debug, Clone)]
pub struct ArtRequest {
    pub key: String,
    pub artist: String,
    pub title: String,
}

/// A fetched cover: PNG/JPEG bytes ready for `image::Handle::from_bytes`.
#[derive(Debug, Clone)]
pub struct ArtResult {
    pub key: String,
    pub bytes: Vec<u8>,
}

/// Spawn the scraper thread. It processes `requests` in order and sends each
/// successful cover to `out`. Silent on misses (GUI keeps the placeholder).
pub fn spawn(requests: Vec<ArtRequest>, out: Sender<ArtResult>) {
    std::thread::Builder::new()
        .name("cassette-artscrape".into())
        .spawn(move || {
            let cache = cache_dir();
            if let Some(dir) = &cache {
                let _ = std::fs::create_dir_all(dir);
            }
            let mut last_mb = Instant::now() - MB_MIN_INTERVAL;

            let total = requests.len();
            eprintln!("[artscrape] starting: {total} albums to fetch");
            let mut done = 0usize;
            for req in requests {
                done += 1;
                if done % 10 == 0 {
                    eprintln!("[artscrape] progress: {done}/{total}");
                }
                // 1. Disk cache - positive (image) or negative (.none marker).
                if let Some(dir) = &cache {
                    let img_path = dir.join(format!("{}.img", sanitize(&req.key)));
                    let none_path = dir.join(format!("{}.none", sanitize(&req.key)));
                    if none_path.exists() {
                        continue; // known miss; don't refetch
                    }
                    if let Ok(bytes) = std::fs::read(&img_path) {
                        if !bytes.is_empty() {
                            let _ = out.send(ArtResult { key: req.key.clone(), bytes });
                            continue;
                        }
                    }
                }

                // 2+3. Network. Rate-limit MusicBrainz specifically.
                let wait = MB_MIN_INTERVAL.saturating_sub(last_mb.elapsed());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
                last_mb = Instant::now();

                // Fetch with backoff+retry on rate-limit. Up to 4 attempts,
                // doubling the wait each time (1s, 2s, 4s, 8s).
                let mut backoff = Duration::from_secs(1);
                let mut result = None;
                for attempt in 0..4 {
                    let f = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        fetch_cover(&req.artist, &req.title)
                    }))
                    .unwrap_or(Err(FetchError::NotFound));
                    match f {
                        Ok(bytes) if !bytes.is_empty() => {
                            result = Some(bytes);
                            break;
                        }
                        Err(FetchError::RateLimited) if attempt < 3 => {
                            eprintln!("[artscrape] rate-limited, backing off {backoff:?}");
                            std::thread::sleep(backoff);
                            backoff *= 2;
                            last_mb = Instant::now();
                            continue;
                        }
                        _ => break, // NotFound, empty, or out of retries
                    }
                }

                match result {
                    Some(bytes) => {
                        eprintln!("[artscrape]   found ({} bytes)", bytes.len());
                        if let Some(dir) = &cache {
                            let _ = std::fs::write(
                                dir.join(format!("{}.img", sanitize(&req.key))),
                                &bytes,
                            );
                        }
                        let _ = out.send(ArtResult {
                            key: req.key.clone(),
                            bytes,
                        });
                    }
                    None => {
                        eprintln!("[artscrape]   - no art");
                        if let Some(dir) = &cache {
                            let _ = std::fs::write(
                                dir.join(format!("{}.none", sanitize(&req.key))),
                                [],
                            );
                        }
                    }
                }
            }
        })
        .expect("failed to spawn art-scrape thread");
}

/// Error outcome of a cover fetch.
enum FetchError {
    NotFound,
    RateLimited,
}

/// Accurate album-cover lookup, two-tier:
///   Tier 1 - search release-GROUPS by artist + title, filtered to a primary
///            studio Album (no Compilation/Live/Single secondary types). This
///            nails songs whose title is also the album title (High Voltage,
///            Holy Diver, Master of Puppets...).
///   Tier 2 - if that misses, search recordings, collect their release-groups,
///            and pick the best studio Album. This catches songs whose title
///            differs from the album (The Trooper -> Piece of Mind).
/// The earlier bug was searching recordings and grabbing whatever release
/// attached first - which surfaced comps and live albums (wrong covers).
fn fetch_cover(artist: &str, title: &str) -> Result<Vec<u8>, FetchError> {
    // Tier 1: release-group search.
    match best_release_group_rg(artist, title) {
        Ok(Some(rgid)) => {
            if let Some(bytes) = caa_front_group(&rgid) {
                return Ok(bytes);
            }
        }
        Err(FetchError::RateLimited) => return Err(FetchError::RateLimited),
        _ => {}
    }

    // Tier 2: recording search -> studio-album release-group.
    match best_release_group_recording(artist, title) {
        Ok(Some(rgid)) => {
            if let Some(bytes) = caa_front_group(&rgid) {
                return Ok(bytes);
            }
            Err(FetchError::NotFound)
        }
        Err(e) => Err(e),
        Ok(None) => Err(FetchError::NotFound),
    }
}

/// MusicBrainz GET returning parsed JSON, mapping throttle to RateLimited.
fn mb_json(url: &str) -> Result<serde_json::Value, FetchError> {
    let resp = match ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) if code == 429 || code == 503 => {
            return Err(FetchError::RateLimited)
        }
        Err(_) => return Err(FetchError::NotFound),
    };
    resp.into_json().map_err(|_| FetchError::NotFound)
}

/// Tier 1: best studio-album release-group id from a release-group search.
fn best_release_group_rg(artist: &str, title: &str) -> Result<Option<String>, FetchError> {
    let query = format!("artist:\"{}\" AND releasegroup:\"{}\"", artist, title);
    let url = format!(
        "https://musicbrainz.org/ws/2/release-group?query={}&fmt=json&limit=10",
        urlencode(&query)
    );
    let json = mb_json(&url)?;
    let groups = match json.get("release-groups").and_then(|g| g.as_array()) {
        Some(g) => g,
        None => return Ok(None),
    };

    let mut best: Option<(i64, String)> = None;
    for rg in groups {
        if rg.get("primary-type").and_then(|t| t.as_str()) != Some("Album") {
            continue;
        }
        let sec = rg
            .get("secondary-types")
            .and_then(|s| s.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let score = rg.get("score").and_then(|s| s.as_i64()).unwrap_or(0);
        // Penalise compilations/live/etc.; bonus for a pure studio album.
        let adjusted = score - (sec as i64) * 40 + if sec == 0 { 50 } else { 0 };
        // Only accept a strong match (avoids grabbing a loosely-named album).
        if adjusted > 90 {
            if let Some(id) = rg.get("id").and_then(|i| i.as_str()) {
                if best.as_ref().map(|(s, _)| adjusted > *s).unwrap_or(true) {
                    best = Some((adjusted, id.to_string()));
                }
            }
        }
    }
    Ok(best.map(|(_, id)| id))
}

/// Tier 2: best studio-album release-group id from a recording search.
fn best_release_group_recording(
    artist: &str,
    title: &str,
) -> Result<Option<String>, FetchError> {
    let query = format!("artist:\"{}\" AND recording:\"{}\"", artist, title);
    let url = format!(
        "https://musicbrainz.org/ws/2/recording?query={}&fmt=json&limit=15",
        urlencode(&query)
    );
    let json = mb_json(&url)?;
    let recordings = match json.get("recordings").and_then(|r| r.as_array()) {
        Some(r) => r,
        None => return Ok(None),
    };

    let mut best: Option<(i64, String)> = None;
    for rec in recordings.iter().take(15) {
        let Some(rels) = rec.get("releases").and_then(|r| r.as_array()) else {
            continue;
        };
        for rel in rels {
            let rg = match rel.get("release-group") {
                Some(g) => g,
                None => continue,
            };
            if rg.get("primary-type").and_then(|t| t.as_str()) != Some("Album") {
                continue;
            }
            let sec = rg
                .get("secondary-types")
                .and_then(|s| s.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let adjusted = 100 - (sec as i64) * 50 + if sec == 0 { 40 } else { 0 };
            if let Some(id) = rg.get("id").and_then(|i| i.as_str()) {
                if best.as_ref().map(|(s, _)| adjusted > *s).unwrap_or(true) {
                    best = Some((adjusted, id.to_string()));
                }
            }
        }
    }
    Ok(best.map(|(_, id)| id))
}

/// Cover Art Archive: front-cover thumbnail bytes for a RELEASE-GROUP.
fn caa_front_group(rgid: &str) -> Option<Vec<u8>> {
    let meta_url = format!("https://coverartarchive.org/release-group/{}", rgid);
    let resp = ureq::get(&meta_url)
        .set("User-Agent", USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .call()
        .ok()?;
    let json: serde_json::Value = resp.into_json().ok()?;
    let images = json.get("images")?.as_array()?;

    let img = images
        .iter()
        .find(|i| i.get("front").and_then(|f| f.as_bool()).unwrap_or(false))
        .or_else(|| images.first())?;

    let thumbs = img.get("thumbnails");
    let url = thumbs
        .and_then(|t| t.get("500"))
        .or_else(|| thumbs.and_then(|t| t.get("large")))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string())
        .or_else(|| img.get("image").and_then(|u| u.as_str()).map(|s| s.to_string()))?;
    let url = url.replace("http://", "https://");

    let resp = ureq::get(&url)
        .set("User-Agent", USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .call()
        .ok()?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(6 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("cosmic-cassette-deck").join("covers"))
}

/// Filesystem-safe cache filename from an arbitrary key.
fn sanitize(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Minimal percent-encoding for the query string (ureq needs a valid URL).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
