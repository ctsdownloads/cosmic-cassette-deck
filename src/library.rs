//! Music library scanning: walk a directory, parse MP3 tags with lofty,
//! group into albums, extract embedded cover art.
//!
//! `scan()` is blocking by design - the App runs it inside
//! `tokio::task::spawn_blocking` so a large library never stalls the GUI.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::file::TaggedFileExt;
use lofty::prelude::*;
use lofty::probe::Probe;

use cosmic::widget::image;

#[derive(Debug, Clone)]
pub struct Track {
    pub title: String,
    pub path: PathBuf,
    pub duration: Duration,
    pub number: u32,
    /// Disc number for multi-disc albums (0 when not tagged).
    pub disc: u32,
}

#[derive(Debug, Clone)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub art: Option<image::Handle>,
    /// Raw bytes of the embedded cover (if any), for compositing onto cases.
    pub art_bytes: Option<std::sync::Arc<Vec<u8>>>,
    pub tracks: Vec<Track>,
    /// No album tag: a loose song shown as its own single.
    pub single: bool,
    /// Exact MusicBrainz IDs from the tags (e.g. written by Picard), so the
    /// cover can be fetched for that exact album instead of searched for.
    pub mb_release_group: Option<String>,
    pub mb_release: Option<String>,
}

/// Per-file extras that belong to the album, not the track.
struct AlbumInfo {
    single: bool,
    mb_release_group: Option<String>,
    mb_release: Option<String>,
}

/// Cover image files looked for next to an album's songs, in this order.
const FOLDER_COVER_NAMES: &[&str] = &["cover", "folder", "front", "album"];
const FOLDER_COVER_EXTS: &[&str] = &["jpg", "jpeg", "png"];

const AUDIO_EXTS: &[&str] = &["mp3", "flac", "m4a", "wav"];

/// Songs read so far by the scan in progress, shown while the rack fills.
pub static SCANNED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Recursively scan every folder in `dirs` for audio files and group them
/// into albums (an album split across folders is still one cassette).
/// Returns (albums, count of matching files that could not be parsed).
pub fn scan(dirs: Vec<PathBuf>) -> (Vec<Album>, usize) {
    // Keyed by (artist, album) so two artists with an album named
    // "Greatest Hits" don't collide.
    let mut albums: BTreeMap<(String, String), Album> = BTreeMap::new();
    // Folder of each album's first song, for a cover image file beside it.
    let mut album_dirs: BTreeMap<(String, String), PathBuf> = BTreeMap::new();
    // Which albums have songs in each folder: a cover image file only belongs
    // to an album when its folder holds that album alone.
    let mut dir_albums: BTreeMap<PathBuf, std::collections::BTreeSet<(String, String)>> =
        BTreeMap::new();
    // How each album's artist and title are spelled across its songs, in the
    // order first seen, so the cassette can show the most common spelling.
    let mut spellings: BTreeMap<(String, String), Vec<((String, String), usize)>> =
        BTreeMap::new();
    let mut skipped: usize = 0;
    SCANNED.store(0, std::sync::atomic::Ordering::Relaxed);

    for entry in dirs
        .iter()
        .flat_map(|dir| walkdir::WalkDir::new(dir).follow_links(true))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path().extension().is_some_and(|ext| {
                AUDIO_EXTS.iter().any(|a| ext.eq_ignore_ascii_case(a))
            })
        })
    {
        SCANNED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let Some((shown, track, art_bytes, info)) = read_track(entry.path()) else {
            skipped += 1;
            continue;
        };
        // Songs share a cassette when their tags match apart from capitals,
        // extra spaces or curly vs straight quotes.
        let key = (match_key(&shown.0), match_key(&shown.1));
        {
            let seen = spellings.entry(key.clone()).or_default();
            match seen.iter_mut().find(|(s, _)| *s == shown) {
                Some((_, n)) => *n += 1,
                None => seen.push((shown.clone(), 1)),
            }
            if let Some(dir) = entry.path().parent() {
                album_dirs.entry(key.clone()).or_insert_with(|| dir.to_path_buf());
                dir_albums.entry(dir.to_path_buf()).or_default().insert(key.clone());
            }
            let album = albums.entry(key.clone()).or_insert_with(|| Album {
                title: shown.1.clone(),
                artist: shown.0.clone(),
                art: None,
                art_bytes: None,
                tracks: Vec::new(),
                single: info.single,
                mb_release_group: None,
                mb_release: None,
            });
            if album.mb_release_group.is_none() {
                album.mb_release_group = info.mb_release_group;
            }
            if album.mb_release.is_none() {
                album.mb_release = info.mb_release;
            }
            if album.art_bytes.is_none() {
                if let Some(bytes) = art_bytes {
                    album.art = Some(image::Handle::from_bytes((*bytes).clone()));
                    album.art_bytes = Some(bytes);
                }
            }
            album.tracks.push(track);
        }
    }

    // Show the spelling most of the album's songs use (first seen on a tie).
    for (key, album) in albums.iter_mut() {
        if let Some(seen) = spellings.get(key) {
            let mut best = &seen[0];
            for s in seen {
                if s.1 > best.1 {
                    best = s;
                }
            }
            album.artist = best.0 .0.clone();
            album.title = best.0 .1.clone();
        }
    }

    // No embedded picture? Use a cover image saved next to the songs, but
    // only from a folder that holds this album alone: a cover.jpg in a folder
    // of several albums (or loose songs) isn't theirs.
    for (key, album) in albums.iter_mut() {
        if album.art_bytes.is_some() || album.single {
            continue;
        }
        let Some(dir) = album_dirs.get(key) else { continue };
        if dir_albums.get(dir).is_some_and(|set| set.len() > 1) {
            continue;
        }
        if let Some(bytes) = folder_cover(dir) {
            let bytes = std::sync::Arc::new(bytes);
            album.art = Some(image::Handle::from_bytes((*bytes).clone()));
            album.art_bytes = Some(bytes);
        }
    }

    // Rack order: by artist then album, ignoring case ("a-ha" sits with the A's).
    let mut out: Vec<Album> = albums.into_values().collect();
    out.sort_by_cached_key(|a| (a.artist.to_lowercase(), a.title.to_lowercase()));
    for album in &mut out {
        // Disc 1 in order, then disc 2, and so on.
        album.tracks.sort_by(|a, b| {
            a.disc
                .cmp(&b.disc)
                .then(a.number.cmp(&b.number))
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.path.cmp(&b.path))
        });
        // Two copies of the same song (same disc, track number and title)
        // play once: the first by file path is kept.
        album.tracks.dedup_by(|b, a| {
            a.disc == b.disc && a.number == b.number && match_key(&a.title) == match_key(&b.title)
        });
    }
    (out, skipped)
}

/// Text used to decide whether two tags name the same album or artist:
/// lowercase, spaces trimmed and collapsed, curly quotes made straight.
fn match_key(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '\u{2032}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201F}' | '\u{2033}' => '"',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// First cover image file in `dir` (cover/folder/front/album .jpg/.jpeg/.png,
/// any case) that reads and decodes as an image.
fn folder_cover(dir: &Path) -> Option<Vec<u8>> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    for name in FOLDER_COVER_NAMES {
        for path in &files {
            let stem_ok = path
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name));
            let ext_ok = path.extension().is_some_and(|e| {
                FOLDER_COVER_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x))
            });
            if stem_ok && ext_ok {
                if let Ok(bytes) = std::fs::read(path) {
                    if ::image::load_from_memory(&bytes).is_ok() {
                        return Some(bytes);
                    }
                }
            }
        }
    }
    None
}

/// Parse one file. Returns ((artist, album), track, cover_art, album info).
fn read_track(
    path: &Path,
) -> Option<((String, String), Track, Option<std::sync::Arc<Vec<u8>>>, AlbumInfo)> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let duration = tagged.properties().duration();

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    // Parse "Artist - Title" from the filename for untagged files. Splits on
    // the FIRST " - " (spaced hyphen), so internal hyphens survive: "a-ha",
    // "American Hi-Fi". A leading track number ("01 - ", "03. ") is stripped.
    // Each file becomes its own single: album == title.
    let parse_filename = || -> (String, String, u32) {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Strip a leading track number.
        let (number, rest) = {
            let trimmed = stem.trim_start();
            let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
            let after = trimmed[digits.len()..].trim_start();
            // Only treat as a track number if a separator follows the digits.
            if !digits.is_empty()
                && after.starts_with(['-', '.', ')'])
            {
                let body = after[1..].trim_start().to_string();
                (digits.parse().unwrap_or(0), body)
            } else {
                (0, stem.clone())
            }
        };

        // Split on the first spaced hyphen.
        if let Some(idx) = rest.find(" - ") {
            let artist = rest[..idx].trim().to_string();
            let title = rest[idx + 3..].trim().to_string();
            let artist = if artist.is_empty() { "Unknown Artist".into() } else { artist };
            let title = if title.is_empty() { rest.trim().to_string() } else { title };
            (artist, title, number)
        } else {
            ("Unknown Artist".into(), rest.trim().to_string(), number)
        }
    };

    let mb_id = |key: &ItemKey| {
        tag.and_then(|t| t.get_string(key))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let info = AlbumInfo {
        single: !tag.is_some_and(|t| t.album().is_some_and(|c| !c.trim().is_empty())),
        mb_release_group: mb_id(&ItemKey::MusicBrainzReleaseGroupId),
        mb_release: mb_id(&ItemKey::MusicBrainzReleaseId),
    };

    let disc = tag.and_then(|t| t.disk()).unwrap_or(0);
    let (title, album, artist, number, art) = match tag {
        Some(tag) => {
            let art = tag
                .pictures()
                .iter()
                .find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront)
                .or_else(|| tag.pictures().first())
                .map(|p| std::sync::Arc::new(p.data().to_vec()));

            // Tags present but sparse? Fall back to filename parsing for any
            // missing field so partially-tagged files still group sensibly.
            let has_artist = tag.artist().is_some_and(|c| !c.trim().is_empty());
            let has_album = tag.album().is_some_and(|c| !c.trim().is_empty());
            let has_title = tag.title().is_some_and(|c| !c.trim().is_empty());

            let (fn_artist, fn_title, fn_number) = parse_filename();

            let title = tag
                .title()
                .filter(|_| has_title)
                .map(|c| c.into_owned())
                .unwrap_or(fn_title.clone());
            // Album Artist, when set, names the whole album: songs credited
            // "Artist feat. Someone" and compilations stay on one cassette.
            let album_artist = tag
                .get_string(&ItemKey::AlbumArtist)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && has_album);
            let artist = album_artist.unwrap_or_else(|| {
                tag.artist()
                    .filter(|_| has_artist)
                    .map(|c| c.into_owned())
                    .unwrap_or(fn_artist)
            });
            // No album tag -> single: album is the track title.
            let album = tag
                .album()
                .filter(|_| has_album)
                .map(|c| c.into_owned())
                .unwrap_or_else(|| title.clone());
            let number = if tag.track().unwrap_or(0) > 0 {
                tag.track().unwrap_or(0)
            } else {
                fn_number
            };
            (title, album, artist, number, art)
        }
        None => {
            // Untagged: parse the filename. Each file is its own single spine.
            let (artist, title, number) = parse_filename();
            (title.clone(), title, artist, number, None)
        }
    };

    Some((
        (artist.clone(), album.clone()),
        Track {
            title,
            path: path.to_path_buf(),
            duration,
            number,
            disc,
        },
        art,
        info,
    ))
}
