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
}

#[derive(Debug, Clone)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub art: Option<image::Handle>,
    /// Raw bytes of the embedded cover (if any), for compositing onto cases.
    pub art_bytes: Option<std::sync::Arc<Vec<u8>>>,
    pub tracks: Vec<Track>,
}

const AUDIO_EXTS: &[&str] = &["mp3", "flac", "ogg", "oga", "m4a", "wav"];

/// Recursively scan `dir` for audio files and group them into albums.
/// Returns (albums, count of matching files that could not be parsed).
pub fn scan(dir: PathBuf) -> (Vec<Album>, usize) {
    // Keyed by (artist, album) so two artists with an album named
    // "Greatest Hits" don't collide.
    let mut albums: BTreeMap<(String, String), Album> = BTreeMap::new();
    let mut skipped: usize = 0;

    for entry in walkdir::WalkDir::new(&dir)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path().extension().is_some_and(|ext| {
                AUDIO_EXTS.iter().any(|a| ext.eq_ignore_ascii_case(a))
            })
        })
    {
        let Some((key, track, art_bytes)) = read_track(entry.path()) else {
            skipped += 1;
            continue;
        };
        {
            let album = albums.entry(key.clone()).or_insert_with(|| Album {
                title: key.1.clone(),
                artist: key.0.clone(),
                art: None,
                art_bytes: None,
                tracks: Vec::new(),
            });
            if album.art_bytes.is_none() {
                if let Some(bytes) = art_bytes {
                    album.art = Some(image::Handle::from_bytes((*bytes).clone()));
                    album.art_bytes = Some(bytes);
                }
            }
            album.tracks.push(track);
        }
    }

    let mut out: Vec<Album> = albums.into_values().collect();
    let _ = &out;
    for album in &mut out {
        album
            .tracks
            .sort_by(|a, b| a.number.cmp(&b.number).then_with(|| a.title.cmp(&b.title)));
    }
    (out, skipped)
}

/// Parse one file. Returns ((artist, album), track, cover_art).
fn read_track(
    path: &Path,
) -> Option<((String, String), Track, Option<std::sync::Arc<Vec<u8>>>)> {
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
            let artist = tag
                .artist()
                .filter(|_| has_artist)
                .map(|c| c.into_owned())
                .unwrap_or(fn_artist);
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
        },
        art,
    ))
}
