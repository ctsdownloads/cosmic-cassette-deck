//! Song lyrics, for the lyric sheet.
//!
//! Looked for in this order, stopping at the first that has words:
//!   1. the song file's own tags (USLT in MP3, LYRICS in FLAC, ©lyr in M4A)
//!   2. a `.lrc` file next to the song, with the same name
//!   3. LRCLIB (lrclib.net), a free online lyrics database
//!
//! What LRCLIB sends back is cached on disk, and so is "nothing there" or
//! "instrumental", so each song is only looked up online once. A failed
//! connection is not cached: the song is tried again the next time.
//!
//! Lyrics in LRC form ("[01:23.45] words") keep their times, so a later
//! version can follow along with the song; the sheet shows the words.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;

const USER_AGENT: &str = concat!(
    "CosmicCassetteDeck/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/ctsdownloads/cosmic-cassette-deck)"
);
const API: &str = "https://lrclib.net/api";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// A song LRCLIB had nothing for is asked about again after this long, in
/// case someone has added it since.
const NONE_TTL: Duration = Duration::from_secs(14 * 24 * 3600);
/// Online lookups go one at a time, however many songs ask at once.
static NET: Mutex<()> = Mutex::new(());

/// One line of lyrics, with its time in the song when the source had one.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub at: Option<f32>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Found(Arc<Lyrics>),
    /// No lyrics anywhere, or the song is an instrumental.
    None,
    /// LRCLIB couldn't be reached. Try again later.
    Offline,
}

/// The song to find lyrics for.
#[derive(Debug, Clone)]
pub struct Request {
    pub path: PathBuf,
    pub artist: String,
    pub title: String,
    /// The album name, or None for a loose song.
    pub album: Option<String>,
    pub secs: f32,
}

/// Find a song's lyrics. Blocking: run it off the UI thread.
pub fn lookup(req: &Request) -> Outcome {
    if let Some(l) = from_tags(&req.path).or_else(|| from_lrc_file(&req.path)) {
        return Outcome::Found(Arc::new(l));
    }
    if req.artist.trim().is_empty() || req.artist == "Unknown Artist" || req.title.trim().is_empty()
    {
        return Outcome::None;
    }
    let cache = cache_path(req);
    if let Some(found) = cache.as_ref().and_then(|p| read_cache(p)) {
        return found;
    }
    let _one_at_a_time = NET.lock().unwrap_or_else(|e| e.into_inner());
    // Another lookup for the same song may have finished while this waited.
    if let Some(found) = cache.as_ref().and_then(|p| read_cache(p)) {
        return found;
    }
    match online(req) {
        Ok(Some(raw)) => match parse(&raw) {
            Some(l) => {
                if let Some(p) = &cache {
                    write_cache(p, Some(&raw));
                }
                Outcome::Found(Arc::new(l))
            }
            None => {
                if let Some(p) = &cache {
                    write_cache(p, None);
                }
                Outcome::None
            }
        },
        Ok(None) => {
            if let Some(p) = &cache {
                write_cache(p, None);
            }
            Outcome::None
        }
        Err(()) => Outcome::Offline,
    }
}

// ── Local sources ───────────────────────────────────────────────────────────

fn from_tags(path: &Path) -> Option<Lyrics> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    tagged
        .tags()
        .iter()
        .find_map(|t| t.get_string(&ItemKey::Lyrics).and_then(parse))
}

fn from_lrc_file(path: &Path) -> Option<Lyrics> {
    ["lrc", "LRC"].iter().find_map(|ext| {
        let bytes = std::fs::read(path.with_extension(ext)).ok()?;
        parse(&String::from_utf8_lossy(&bytes))
    })
}

// ── Parsing ─────────────────────────────────────────────────────────────────

/// Plain or LRC lyrics into lines. LRC header tags ([ar:], [ti:], ...) and
/// word times (<01:02.03>) are dropped; a line with several times is placed
/// at each of them. Runs of blank lines become one, the gap between verses.
/// None when there are no words at all.
pub fn parse(raw: &str) -> Option<Lyrics> {
    let mut lines: Vec<Line> = Vec::new();
    let mut timed = false;
    for raw_line in raw.lines() {
        let mut rest = raw_line.trim();
        let mut times = Vec::new();
        let mut header = false;
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else { break };
            let tag = &inner[..end];
            match lrc_time(tag) {
                Some(t) => times.push(t),
                None if tag.contains(':') && times.is_empty() => header = true,
                None => break,
            }
            rest = inner[end + 1..].trim_start();
        }
        if header && rest.is_empty() {
            continue;
        }
        let text = strip_word_times(rest).trim().to_string();
        if times.is_empty() {
            lines.push(Line { at: None, text });
        } else {
            timed = true;
            for t in times {
                lines.push(Line {
                    at: Some(t),
                    text: text.clone(),
                });
            }
        }
    }
    if timed {
        // Lines with several times were added once per time; put them in order.
        lines.sort_by(|a, b| {
            a.at.unwrap_or(f32::MAX)
                .partial_cmp(&b.at.unwrap_or(f32::MAX))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    for l in lines {
        if l.text.is_empty() && out.last().is_none_or(|p: &Line| p.text.is_empty()) {
            continue;
        }
        out.push(l);
    }
    while out.last().is_some_and(|l| l.text.is_empty()) {
        out.pop();
    }
    (!out.is_empty()).then_some(Lyrics { lines: out })
}

/// "mm:ss", "mm:ss.xx" or "mm:ss:xx" into seconds.
fn lrc_time(tag: &str) -> Option<f32> {
    let (m, s) = tag.split_once(':')?;
    let m: u32 = m.trim().parse().ok()?;
    let s = s.trim().replacen(':', ".", 1);
    let s: f32 = s.parse().ok()?;
    (s < 60.0).then_some(m as f32 * 60.0 + s)
}

fn strip_word_times(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        match rest[i + 1..].find('>') {
            Some(j) if lrc_time(&rest[i + 1..i + 1 + j]).is_some() => rest = &rest[i + 2 + j..],
            _ => {
                out.push('<');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ── LRCLIB ──────────────────────────────────────────────────────────────────

/// Ok(Some(lyrics)), Ok(None) when LRCLIB has none or it's an instrumental,
/// Err when LRCLIB couldn't be reached.
fn online(req: &Request) -> Result<Option<String>, ()> {
    // The exact song: artist, title, album and length all matching.
    let mut get = ureq::get(&format!("{API}/get"))
        .set("User-Agent", USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .query("artist_name", &req.artist)
        .query("track_name", &req.title);
    if let Some(album) = &req.album {
        get = get.query("album_name", album);
    }
    if req.secs > 0.0 {
        get = get.query("duration", &format!("{}", req.secs.round() as u64));
    }
    match get.call() {
        Ok(r) => {
            let v: serde_json::Value = r.into_json().map_err(|_| ())?;
            if v.get("instrumental").and_then(|i| i.as_bool()) == Some(true) {
                return Ok(None);
            }
            if let Some(words) = words_of(&v) {
                return Ok(Some(words));
            }
        }
        Err(ureq::Error::Status(404, _)) => {}
        Err(_) => return Err(()),
    }

    // Not an exact match (another release, a slightly different length or
    // album name): search by artist and title, and keep only real matches.
    let mut titles = vec![req.title.clone()];
    let bare = bare_title(&req.title);
    if !bare.is_empty() && bare != req.title {
        titles.push(bare);
    }
    for title in titles {
        let resp = ureq::get(&format!("{API}/search"))
            .set("User-Agent", USER_AGENT)
            .timeout(HTTP_TIMEOUT)
            .query("artist_name", &req.artist)
            .query("track_name", &title)
            .call();
        let results: Vec<serde_json::Value> = match resp {
            Ok(r) => r.into_json().map_err(|_| ())?,
            Err(ureq::Error::Status(404, _)) => continue,
            Err(_) => return Err(()),
        };
        if let Some(best) = pick(req, &results) {
            return Ok(best);
        }
    }
    Ok(None)
}

/// The synced lyrics if there are any, else the plain ones.
fn words_of(v: &serde_json::Value) -> Option<String> {
    ["syncedLyrics", "plainLyrics"].iter().find_map(|k| {
        v.get(*k)
            .and_then(|s| s.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    })
}

/// From search results, the best match for the song: Some(Some(words)),
/// Some(None) when the matches say it's an instrumental, None when nothing
/// in the results is this song.
fn pick(req: &Request, results: &[serde_json::Value]) -> Option<Option<String>> {
    let artist = norm(&req.artist);
    let title = norm(&bare_title(&req.title));
    let matches: Vec<&serde_json::Value> = results
        .iter()
        .filter(|v| {
            let a = norm(v.get("artistName").and_then(|s| s.as_str()).unwrap_or(""));
            let t = norm(&bare_title(
                v.get("trackName").and_then(|s| s.as_str()).unwrap_or(""),
            ));
            !a.is_empty()
                && (a == artist || a.contains(&artist) || artist.contains(&a))
                && t == title
        })
        .collect();
    if matches.is_empty() {
        return None;
    }
    let off = |v: &serde_json::Value| -> f32 {
        let d = v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0) as f32;
        if req.secs > 0.0 && d > 0.0 {
            (d - req.secs).abs()
        } else {
            0.0
        }
    };
    let instrumental =
        |v: &serde_json::Value| v.get("instrumental").and_then(|i| i.as_bool()) == Some(true);
    // Versions about the same length as this file, if any.
    let close: Vec<&serde_json::Value> =
        matches.iter().copied().filter(|v| off(v) <= 10.0).collect();
    let pool = if close.is_empty() { matches } else { close };
    let sung: Vec<&serde_json::Value> = pool
        .iter()
        .copied()
        .filter(|v| !instrumental(v) && words_of(v).is_some())
        .collect();
    let silent = pool.iter().filter(|v| instrumental(v)).count();
    if sung.is_empty() || silent > sung.len() {
        return Some(None);
    }
    // Synced first, then the closest length.
    let best = sung.iter().min_by(|a, b| {
        let synced = |v: &serde_json::Value| {
            v.get("syncedLyrics")
                .and_then(|s| s.as_str())
                .is_none_or(|s| s.trim().is_empty())
        };
        synced(a).cmp(&synced(b)).then(
            off(a)
                .partial_cmp(&off(b))
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    })?;
    Some(words_of(best))
}

/// A title without "(Remastered 2011)", "[Live]" or " - 2003 Remaster".
fn bare_title(t: &str) -> String {
    let mut s = String::with_capacity(t.len());
    let mut depth = 0i32;
    for c in t.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            c if depth == 0 => s.push(c),
            _ => {}
        }
    }
    let s = match s.find(" - ") {
        Some(i) if i > 0 => s[..i].to_string(),
        _ => s,
    };
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lowercase letters and digits only, for comparing names.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

// ── Cache ───────────────────────────────────────────────────────────────────

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("cosmic-cassette-deck").join("lyrics"))
}

/// The cache file stem for a song: a hash of who, what and which album.
fn cache_path(req: &Request) -> Option<PathBuf> {
    let key = format!(
        "{}\t{}\t{}",
        norm(&req.artist),
        norm(&req.title),
        req.album.as_deref().map(norm).unwrap_or_default()
    );
    // FNV-1a, 64-bit: stable across runs and builds.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(cache_dir()?.join(format!("{h:016x}")))
}

fn read_cache(stem: &Path) -> Option<Outcome> {
    if let Ok(raw) = std::fs::read_to_string(stem.with_extension("lrc")) {
        if let Some(l) = parse(&raw) {
            return Some(Outcome::Found(Arc::new(l)));
        }
    }
    let none = stem.with_extension("none");
    let age = std::fs::metadata(&none)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .unwrap_or_default();
    (age < NONE_TTL).then_some(Outcome::None)
}

fn write_cache(stem: &Path, raw: Option<&str>) {
    if let Some(dir) = stem.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match raw {
        Some(raw) => {
            let _ = std::fs::write(stem.with_extension("lrc"), raw);
            let _ = std::fs::remove_file(stem.with_extension("none"));
        }
        None => {
            let _ = std::fs::write(stem.with_extension("none"), b"");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lines_keep_one_gap_between_verses() {
        let l = parse("\n\nfirst line\nsecond\n\n\n\nthird\n\n").unwrap();
        let texts: Vec<&str> = l.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["first line", "second", "", "third"]);
        assert!(l.lines.iter().all(|l| l.at.is_none()));
    }

    #[test]
    fn lrc_times_headers_and_repeats() {
        let raw = "[ar:Someone]\n[ti:Song]\n[00:20.50]later\n[00:05.00][00:30.00]chorus\n[00:10.00]<00:10.00>word <00:10.50>times\n";
        let l = parse(raw).unwrap();
        let got: Vec<(f32, &str)> = l
            .lines
            .iter()
            .map(|l| (l.at.unwrap(), l.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (5.0, "chorus"),
                (10.0, "word times"),
                (20.5, "later"),
                (30.0, "chorus")
            ]
        );
    }

    #[test]
    fn nothing_but_blanks_is_no_lyrics() {
        assert!(parse("\n  \n[00:01.00]\n").is_none());
    }

    #[test]
    fn titles_lose_version_notes() {
        assert_eq!(bare_title("One (Remastered 2017)"), "One");
        assert_eq!(bare_title("Hangar 18 - 2004 Remix"), "Hangar 18");
        assert_eq!(bare_title("Song [Live]"), "Song");
    }
}
