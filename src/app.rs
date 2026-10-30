//! The libcosmic Application: Model–View–Update.
//!
//! Screens: Player ⇄ Rack ⇄ AlbumDetail. Audio runs on its own thread
//! (audio.rs); MPRIS on another (mpris.rs). This module sends commands,
//! polls status on Tick, and renders the photographic deck (cassette.rs)
//! with the pre-rasterized label (label.rs).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cosmic::app::{Core, Task};
use cosmic::iced::widget::canvas::Canvas;
use cosmic::iced::{Alignment, Length, Subscription};
use cosmic::widget::{self, button, container, slider, text};
use cosmic::{Application, Element};

use crate::audio::{self, AudioCmd, Status};
use crate::cassette::{load_skins, bedroom_handle, spool_omegas, CassetteScene, LoadedSkin};
use crate::label;
use crate::library::{self, Album};
use crate::mpris::{self, MprisCmd, MprisUpdate};
use crate::rack_scene::RackScene;
use crate::artscrape::{self, ArtRequest, ArtResult};
use crate::case_open::{self, CaseOpenScene};

const TICK: Duration = Duration::from_millis(16); // ~60 fps: keeps fast spool
// rotation under the wagon-wheel aliasing limit of the spoked hubs
/// Slow idle tick: a tape is loaded but paused/idle. Keeps position and state
/// fresh (and self-heals a stale `playing` flag) without a 60 fps CPU burn.
const HEARTBEAT: Duration = Duration::from_millis(250);
const STATE_SAVE_EVERY: Duration = Duration::from_secs(5);
/// How long the case-open → walkman animation plays before the player appears.
const CASE_ANIM: Duration = Duration::from_millis(1600);
/// Eject: the tape slides up out of the window, then the rack opens.
const EJECT_ANIM: Duration = Duration::from_millis(700);
/// Tape swap at an album change: old tape out, short beat, new tape in.
const SWAP_ANIM: Duration = Duration::from_millis(1300);
/// After an eject, the rack fades in from the deck's charcoal.
const RACK_FADE: Duration = Duration::from_millis(280);
/// Reel spin-up / spin-down time constant (seconds) on play, pause and stop.
const REEL_TAU: f32 = 0.12;

/// Project home, shown in and opened from the About card.
const REPO_URL: &str = "https://github.com/ctsdownloads/cosmic-cassette-deck";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Player,
    Rack,
    /// Case reveal: the selected album's cassette case, big, cover on the
    /// J-card, over the dimmed rack. Play from here or click out to return.
    Reveal(usize),
    /// Case-open animation: the reveal case swings open, the tape lifts out and
    /// drops into the walkman. Carries the track to load and when it started.
    CaseOpen {
        album: usize,
        track: usize,
        started: Instant,
    },
    /// Eject on the player: the tape slides up out of the window, then the
    /// tape is unloaded and the rack opens.
    Eject { started: Instant },
    /// Album change on the player: the old tape slides out and the next
    /// album's tape slides in, then that track loads.
    Swap {
        album: usize,
        track: usize,
        started: Instant,
    },
}

impl Screen {
    /// An animation owns the screen; user input is ignored until it ends.
    fn animating(&self) -> bool {
        matches!(
            self,
            Screen::CaseOpen { .. } | Screen::Eject { .. } | Screen::Swap { .. }
        )
    }
}

/// Eject: tape out of the window, `drop` 0 -> 1, fast then settling, then a
/// short beat on the empty deck before the rack opens.
fn eject_drop(t: f32) -> f32 {
    let k = (t / 0.75).clamp(0.0, 1.0);
    1.0 - (1.0 - k).powi(3)
}

/// Tape swap: old tape out (0 -> 1), beat, new tape in (1 -> 0). The second
/// value is true once the new tape (and its label) is the one showing.
fn swap_drop(t: f32) -> (f32, bool) {
    if t < 0.40 {
        let k = t / 0.40;
        (1.0 - (1.0 - k).powi(3), false)
    } else if t < 0.50 {
        (1.0, true)
    } else {
        let k = ((t - 0.50) / 0.50).clamp(0.0, 1.0);
        (1.0 - k * k, true)
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick(Instant),
    OpenFolder,
    FolderChosen(Option<PathBuf>),
    LibraryLoaded { albums: Vec<Album>, skipped: usize },
    ShowRack,
    ShowPlayer,
    LabelClicked, // clicking the deck label opens the current album art
    AlbumSelected(usize),
    CloseReveal,
    OpenCase { album: usize, track: usize }, // click the reveal case: open it, then load
    TogglePlay,
    Stop,
    Next,
    Prev,
    Seek(f32),   // fraction 0.0..=1.0
    SetVolume(f32),
    WindStart(i8), // +1 fwd, -1 back (hold-to-wind from the deck buttons)
    WindStop,
    ToggleSkipMode, // FUNCT tab: FF/REW become track skip while latched
    BrowseStep(i32), // fan browser: step ±1 through albums
    CycleSkin,
    CycleBackground,
    PickBackground,
    BackgroundChosen(Option<PathBuf>),
    RemoveBackground,
    ArtLoaded(ArtResult),
    Mpris(MprisCmd),
    ToggleAbout,
    OpenRepo,
    ToggleBackdrop,
}

pub struct App {
    core: Core,
    screen: Screen,
    library: Vec<Album>,
    scanning: bool,
    /// About card visible (a full-view panel over the current screen).
    show_about: bool,

    /// Currently loaded (album_idx, track_idx), if a tape is in the deck.
    current: Option<(usize, usize)>,

    audio_tx: Sender<AudioCmd>,
    status: Status,
    mpris_tx: tokio::sync::mpsc::UnboundedSender<MprisUpdate>,

    // Skin rendering
    skins: Vec<LoadedSkin>,
    skin_idx: usize,
    /// All room backgrounds: built-ins first, then user-added. `current_bg()`
    /// selects by `bg_idx`. Shared by the rack, album screen, and case-open.
    backgrounds: Vec<cosmic::widget::image::Handle>,
    /// Source path per background (None = built-in), for persistence.
    bg_sources: Vec<Option<PathBuf>>,
    bg_idx: usize,
    /// Raw transparent-case PNG bytes, for compositing covers onto cases.
    case_png: std::sync::Arc<Vec<u8>>,
    /// Which album is centred in the fan browser.
    browse_index: usize,
    /// Raw cover bytes per album index (scraped), for case compositing.
    cover_bytes: std::collections::HashMap<usize, std::sync::Arc<Vec<u8>>>,
    /// Composed case sprites for the current fan window (centre first).
    fan_cases: Vec<cosmic::widget::image::Handle>,
    label_img: Option<cosmic::widget::image::Handle>,
    /// Label for the track an insert animation is loading, rendered when the
    /// animation starts so the tape slides in already labelled.
    incoming_label: Option<cosmic::widget::image::Handle>,
    /// Album-cover wallpaper for the tape an insert or swap is loading,
    /// decoded when it starts so it can fade in (only while Wallpaper is on).
    incoming_backdrop: Option<cosmic::widget::image::Handle>,
    /// Blurred album-cover backdrop shown behind the deck on the Player page.
    player_backdrop: Option<cosmic::widget::image::Handle>,
    /// Whether the Player draws the cover backdrop (toggled on the Player page,
    /// persisted; off by default).
    show_backdrop: bool,
    /// Pre-built handles for the cassette-load animation (empty deck + sprite).
    load_sprite: cosmic::widget::image::Handle,
    /// Split handles of the open-case sprite for the case-open animation.
    case_tray: cosmic::widget::image::Handle,
    case_lid: cosmic::widget::image::Handle,
    case_lid_flipped: cosmic::widget::image::Handle,

    // Animation state
    angle_src: f32,
    angle_tk: f32,
    /// Reel speed factor, 0 (still) to 1 (play speed). Eases toward the
    /// target so the reels spin up and down instead of starting dead.
    reel_speed: f32,
    /// When the rack started fading in after an eject (None when not fading).
    rack_fade: Option<Instant>,
    /// Solid page-background image, for fades drawn inside canvases.
    page_fill: cosmic::widget::image::Handle,
    last_tick: Instant,

    // Mirrored from SharedStatus each tick (view() stays lock-free)
    position: Duration,
    playing: bool,
    /// True after Stop: the sink is gone, so Play must reload, not Resume.
    stopped: bool,
    /// Hold-to-wind in progress: +1 = FF, -1 = REW.
    wind: Option<i8>,
    /// Whether playback should resume when the wind key is released.
    wind_resume: bool,
    /// FUNCT latched: FF/REW skip tracks instead of winding.
    skip_mode: bool,
    /// Brief correct-direction spool burst after a track skip: (dir, secs left).
    skip_anim: Option<(i8, f32)>,

    // Persistence / UX
    music_dir: Option<PathBuf>,
    volume: f32,
    /// (track path, position secs) restored from the last session.
    pending_resume: Option<(PathBuf, f32)>,
    last_save: Instant,
    /// One-line status/error surfaced to the user.
    notice: Option<String>,
}

impl App {
    fn current_track(&self) -> Option<(&Album, &library::Track)> {
        let (a, t) = self.current?;
        let album = self.library.get(a)?;
        let track = album.tracks.get(t)?;
        Some((album, track))
    }

    fn duration(&self) -> Duration {
        self.current_track()
            .map(|(_, t)| t.duration)
            .unwrap_or(Duration::ZERO)
    }

    fn progress(&self) -> f32 {
        let d = self.duration().as_secs_f32();
        if d <= 0.0 {
            0.0
        } else {
            (self.position.as_secs_f32() / d).clamp(0.0, 1.0)
        }
    }

    fn push_mpris(&self, u: MprisUpdate) {
        let _ = self.mpris_tx.send(u);
    }

    /// Rebuild the fan window's composed case sprites around browse_index.
    /// Order: centre, +1, -1, +2, -2, ... (matches rack_scene draw order).
    fn rebuild_fan(&mut self) {
        let n = self.library.len();
        if n == 0 {
            self.fan_cases.clear();
            return;
        }
        let compose_at = |idx: usize,
                          case_png: &[u8],
                          covers: &std::collections::HashMap<usize, std::sync::Arc<Vec<u8>>>,
                          library: &[Album]|
         -> cosmic::widget::image::Handle {
            // Prefer a scraped cover; else the file's embedded art; else blank.
            let scraped = covers.get(&idx).map(|b| b.as_slice());
            let embedded = library
                .get(idx)
                .and_then(|a| a.art_bytes.as_ref())
                .map(|b| b.as_slice());
            let bytes = scraped.or(embedded);
            crate::case_scene::compose(case_png, bytes)
        };

        let mut out = Vec::new();
        // centre
        out.push(compose_at(
            self.browse_index,
            &self.case_png,
            &self.cover_bytes,
            &self.library,
        ));
        // neighbours +d then -d
        for d in 1..=crate::rack_scene::FAN_EACH_SIDE {
            let ri = (self.browse_index + d) % n;
            out.push(compose_at(ri, &self.case_png, &self.cover_bytes, &self.library));
            let li = (self.browse_index + n - (d % n)) % n;
            out.push(compose_at(li, &self.case_png, &self.cover_bytes, &self.library));
        }
        self.fan_cases = out;
    }

    fn refresh_rack_overlay(&mut self) {
        self.rebuild_fan();
    }

    /// Collect albums with no embedded art and scrape covers for them.
    fn start_art_scrape(&mut self) {
        let requests: Vec<ArtRequest> = self
            .library
            .iter()
            .filter(|a| a.art.is_none())
            .map(|a| ArtRequest {
                key: album_art_key(a),
                artist: a.artist.clone(),
                title: a.title.clone(),
            })
            .collect();
        if requests.is_empty() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel::<ArtResult>();
        if let Some(slot) = ART_RX.get() {
            *slot.lock().unwrap() = Some(rx);
        } else {
            let _ = ART_RX.set(std::sync::Mutex::new(Some(rx)));
        }
        artscrape::spawn(requests, tx);
    }

    fn push_mpris_metadata(&self) {
        if let Some((album, track)) = self.current_track() {
            self.push_mpris(MprisUpdate::Metadata {
                title: track.title.clone(),
                artist: album.artist.clone(),
                album: album.title.clone(),
                length_secs: track.duration.as_secs_f64(),
            });
        }
    }

    /// Scraped cover for `album` if present, else the file's embedded art.
    fn album_cover_bytes(&self, album: usize) -> Option<Vec<u8>> {
        if let Some(b) = self.cover_bytes.get(&album) {
            return Some((**b).clone());
        }
        self.library
            .get(album)
            .and_then(|a| a.art_bytes.as_ref())
            .map(|b| (**b).clone())
    }

    /// (Re)build the blurred backdrop for the loaded album's cover. `None` when
    /// no tape is loaded or its cover art hasn't arrived yet.
    fn refresh_backdrop(&mut self) {
        let bytes = self.current.and_then(|(album, _)| self.album_cover_bytes(album));
        self.player_backdrop = bytes.and_then(|b| cover_backdrop(&b));
    }

    fn load_track(&mut self, album: usize, track: usize) {
        let Some((path, song, album_name)) = self.library.get(album).and_then(|a| {
            a.tracks
                .get(track)
                .map(|t| (t.path.clone(), t.title.clone(), a.title.clone()))
        }) else {
            return;
        };
        let _ = self.audio_tx.send(AudioCmd::Load(path));
        self.current = Some((album, track));
        match self.incoming_backdrop.take() {
            // Already decoded for the insert/swap that brought this tape in.
            Some(b) if self.show_backdrop => self.player_backdrop = Some(b),
            _ => self.refresh_backdrop(),
        }
        self.position = Duration::ZERO;
        self.playing = true;
        self.stopped = false;
        self.notice = None;
        // Rasterize the label once per track/skin change — not per frame.
        self.label_img = Some(label::render(&song, &album_name, self.skins[self.skin_idx].spec));
        self.incoming_label = None;
        self.screen = Screen::Player;
        self.push_mpris_metadata();
        self.push_mpris(MprisUpdate::Playing);
        self.save_state();
    }

    /// Load a track from a different album. On the player this swaps tapes
    /// on screen (old one out, new one in); anywhere else it loads directly.
    fn change_tape(&mut self, album: usize, track: usize) {
        if self.screen != Screen::Player {
            self.load_track(album, track);
            return;
        }
        let _ = self.audio_tx.send(AudioCmd::Stop);
        self.playing = false;
        self.skip_anim = None;
        self.incoming_label = self.render_label_for(album, track);
        self.incoming_backdrop = self.backdrop_for(album);
        self.screen = Screen::Swap {
            album,
            track,
            started: Instant::now(),
        };
    }

    /// Finish an eject: the deck is empty, the rack opens on the ejected album.
    fn unload_tape(&mut self) {
        let _ = self.audio_tx.send(AudioCmd::Stop);
        if let Some((album, _)) = self.current {
            if album < self.library.len() {
                self.browse_index = album;
                self.rebuild_fan();
            }
        }
        self.current = None;
        self.label_img = None;
        self.player_backdrop = None;
        self.playing = false;
        self.stopped = false;
        self.reel_speed = 0.0;
        self.position = Duration::ZERO;
        self.push_mpris(MprisUpdate::Stopped);
        self.save_state();
        self.screen = Screen::Rack;
        self.rack_fade = Some(Instant::now());
    }

    /// Wallpaper for an album about to load, when Wallpaper is on.
    fn backdrop_for(&self, album: usize) -> Option<cosmic::widget::image::Handle> {
        if !self.show_backdrop {
            return None;
        }
        self.album_cover_bytes(album).and_then(|b| cover_backdrop(&b))
    }

    /// Label for a track that is about to load (insert animations).
    fn render_label_for(&self, album: usize, track: usize) -> Option<cosmic::widget::image::Handle> {
        let a = self.library.get(album)?;
        let t = a.tracks.get(track)?;
        Some(label::render(&t.title, &a.title, self.skins[self.skin_idx].spec))
    }

    /// Load a track paused at a given position (session resume).
    fn resume_track(&mut self, album: usize, track: usize, pos_secs: f32) {
        self.load_track(album, track);
        let pos = Duration::from_secs_f32(pos_secs.max(0.0));
        let _ = self.audio_tx.send(AudioCmd::Pause);
        let _ = self.audio_tx.send(AudioCmd::Seek(pos));
        self.position = pos;
        self.playing = false;
        self.push_mpris(MprisUpdate::Paused);
        // Session resume loads the tape but must NOT jump to the player — the
        // app always opens on the Cassette Rack. (load_track sets Player.)
        self.screen = Screen::Rack;
    }

    /// Move ±1 track, continuing across album boundaries in rack order.
    /// While browsing, playback moves on without leaving the screen, and the
    /// rack follows the tape unless you've browsed to a different album.
    fn step_track(&mut self, offset: i64) {
        let screen = self.screen;
        let old_album = self.current.map(|(a, _)| a);
        self.step_track_inner(offset);

        if matches!(screen, Screen::Rack | Screen::Reveal(_)) {
            self.screen = screen;
        }
        let new_album = self.current.map(|(a, _)| a);
        if screen == Screen::Rack && new_album != old_album && old_album == Some(self.browse_index) {
            if let Some(album) = new_album {
                self.browse_index = album;
                self.rebuild_fan();
            }
        }
    }

    fn step_track_inner(&mut self, offset: i64) {
        let Some((a, t)) = self.current else { return };
        // Spools whirl briefly in the travel direction of the skip.
        self.skip_anim = Some((offset.signum() as i8, 0.35));

        let len = self.library[a].tracks.len() as i64;
        let next = t as i64 + offset;

        if (0..len).contains(&next) {
            self.load_track(a, next as usize);
        } else if offset > 0 {
            // Side ran out: next tape on the shelf, wrapping to the first.
            self.change_tape((a + 1) % self.library.len(), 0);
        } else if a > 0 {
            let prev = a - 1;
            let last = self.library[prev].tracks.len().saturating_sub(1);
            self.change_tape(prev, last);
        } else {
            self.load_track(a, 0); // before the very first track: restart it
        }
    }

    /// The currently selected room background (rack + album + case-open).
    fn current_bg(&self) -> &cosmic::widget::image::Handle {
        &self.backgrounds[self.bg_idx]
    }

    fn save_state(&self) {
        let Some(path) = state_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut out = String::new();
        if let Some(dir) = &self.music_dir {
            out.push_str(&format!("music_dir={}\n", dir.display()));
        }
        out.push_str(&format!("volume={}\n", self.volume));
        out.push_str(&format!("skin={}\n", self.skins[self.skin_idx].spec.id));
        out.push_str(&format!("bg_idx={}\n", self.bg_idx));
        out.push_str(&format!("backdrop={}\n", self.show_backdrop as u8));
        let bg_files: Vec<String> = self
            .bg_sources
            .iter()
            .filter_map(|p| p.as_ref().map(|p| p.display().to_string()))
            .collect();
        if !bg_files.is_empty() {
            out.push_str(&format!("bg_files={}\n", bg_files.join("\t")));
        }
        if let Some((_, track)) = self.current_track() {
            out.push_str(&format!("last_track={}\n", track.path.display()));
            out.push_str(&format!("last_pos_secs={}\n", self.position.as_secs_f32()));
        }
        let _ = std::fs::write(path, out);
    }
}

impl Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "io.github.ctsdownloads.CosmicCassetteDeck";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Message>) {
        let status: Status = Arc::new(Mutex::new(audio::SharedStatus::default()));
        let audio_tx = audio::spawn(status.clone());
        let mpris_tx = mpris::spawn();

        let state = read_state();
        let volume = state
            .get("volume")
            .and_then(|v| v.parse::<f32>().ok())
            .map(|v| v.clamp(0.0, 1.0))
            .unwrap_or(0.85);
        let _ = audio_tx.send(AudioCmd::Volume(volume));

        let music_dir = state
            .get("music_dir")
            .map(PathBuf::from)
            .filter(|d| d.is_dir())
            .or_else(load_music_dir_legacy);

        let skins = load_skins();
        let skin_idx = state
            .get("skin")
            .and_then(|id| skins.iter().position(|s| s.spec.id == id))
            .unwrap_or(0);

        let pending_resume = match (state.get("last_track"), state.get("last_pos_secs")) {
            (Some(track), pos) => {
                let p = PathBuf::from(track);
                p.is_file().then(|| {
                    (p, pos.and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0))
                })
            }
            _ => None,
        };

        // Room backgrounds: bedroom + a couple of built-ins, then any the user
        // added before (only files that still exist).
        let bedroom = bedroom_handle();
        let mut backgrounds = vec![
            bedroom,
            solid_bg([18, 18, 22]),
            gradient_bg([54, 38, 68], [20, 22, 34]),
        ];
        for b in EXTRA_BGS {
            backgrounds.push(cosmic::widget::image::Handle::from_bytes(b.to_vec()));
        }
        let mut bg_sources: Vec<Option<PathBuf>> = vec![None; backgrounds.len()];
        if let Some(list) = state.get("bg_files") {
            for p in list.split('\t').filter(|s| !s.is_empty()) {
                let path = PathBuf::from(p);
                if let Ok(bytes) = std::fs::read(&path) {
                    backgrounds.push(cosmic::widget::image::Handle::from_bytes(bytes));
                    bg_sources.push(Some(path));
                }
            }
        }
        let bg_idx = state
            .get("bg_idx")
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|i| *i < backgrounds.len())
            .unwrap_or(0);

        // Split the open-case sprite into tray + lid (+ flipped lid) once.
        let (case_tray, case_lid, case_lid_flipped) = case_open::split(CASE_OPEN_PNG)
            .unwrap_or_else(|| {
                let blank = cosmic::widget::image::Handle::from_rgba(1, 1, vec![0, 0, 0, 0]);
                (blank.clone(), blank.clone(), blank)
            });

        let show_backdrop = state.get("backdrop").map(|v| v == "1").unwrap_or(false);

        let mut app = App {
            core,
            screen: Screen::Rack,
            library: Vec::new(),
            scanning: false,
            show_about: false,
            current: None,
            audio_tx,
            status,
            mpris_tx,
            skins,
            skin_idx,
            backgrounds,
            bg_sources,
            bg_idx,
            case_png: std::sync::Arc::new(CASE_T_PNG.to_vec()),
            browse_index: 0,
            cover_bytes: std::collections::HashMap::new(),
            fan_cases: Vec::new(),
            label_img: None,
            incoming_label: None,
            incoming_backdrop: None,
            player_backdrop: None,
            show_backdrop,
            load_sprite: cosmic::widget::image::Handle::from_bytes(CASSETTE_SPRITE_PNG.to_vec()),
            case_tray,
            case_lid,
            case_lid_flipped,
            angle_src: 0.0,
            angle_tk: 0.0,
            reel_speed: 0.0,
            rack_fade: None,
            page_fill: solid_bg([23, 23, 28]),
            last_tick: Instant::now(),
            position: Duration::ZERO,
            playing: false,
            stopped: false,
            wind: None,
            wind_resume: false,
            skip_mode: false,
            skip_anim: None,
            music_dir: music_dir.clone(),
            volume,
            pending_resume,
            last_save: Instant::now(),
            notice: None,
        };

        // Remembered library: auto-scan on startup so the rack is ready
        // without clicking through the folder picker every launch.
        let startup = match music_dir {
            Some(dir) => {
                app.scanning = true;
                scan_task(dir)
            }
            None => Task::none(),
        };
        (app, startup)
    }

    fn subscription(&self) -> Subscription<Message> {
        // Tick cadence has three tiers. Gating on `self.playing` alone
        // deadlocks: playing is only updated inside a Tick (status mirror), so
        // one tick seeing a stale paused status (e.g. right after a wind-release
        // Resume) would kill the subscription permanently while audio plays on.
        // Safety net: a loaded, non-stopped deck never fully sleeps — it keeps
        // at least a slow heartbeat, so a stale `self.playing` self-heals within
        // one HEARTBEAT (the next tick mirrors the real status and jumps back to
        // 60 fps). Only an empty or stopped deck sleeps entirely.
        //   - 60 fps while genuinely animating or playing,
        //   - slow heartbeat while a tape is loaded but paused/idle,
        //   - nothing when no tape is loaded.
        let animating = self.wind.is_some()
            || self.skip_anim.is_some()
            || self.screen.animating()
            || self.playing
            || self.reel_speed > 0.0
            || self.rack_fade.is_some();
        let has_tape = self.current.is_some() && !self.stopped;
        let ticks = if animating {
            cosmic::iced::time::every(TICK).map(Message::Tick)
        } else if has_tape {
            cosmic::iced::time::every(HEARTBEAT).map(Message::Tick)
        } else {
            Subscription::none()
        };

        // Arrow keys navigate the rack. libcosmic itself uses
        // iced_futures::event::listen_with for keyboard/window subscriptions
        // (see libcosmic src/app/cosmic.rs); the closure takes (event, status,
        // window_id). The BrowseStep handler no-ops unless on the rack screen.
        // Space plays/pauses and S stops (S again, stopped on the player,
        // ejects) - only when no widget took the key (e.g. a focused button)
        // and no modifier is held, so Ctrl+S and the like pass through.
        let keys = cosmic::iced::event::listen_with(|event, status, _id| {
            use cosmic::iced::keyboard::{key::Named, Event as KeyEvent, Key};
            if let cosmic::iced::Event::Keyboard(KeyEvent::KeyPressed { key, modifiers, .. }) = event {
                let free = status == cosmic::iced::event::Status::Ignored && modifiers.is_empty();
                match key.as_ref() {
                    Key::Named(Named::ArrowLeft) => Some(Message::BrowseStep(-1)),
                    Key::Named(Named::ArrowRight) => Some(Message::BrowseStep(1)),
                    Key::Character(c) if free && c == " " => Some(Message::TogglePlay),
                    Key::Character(c) if free && c.eq_ignore_ascii_case("s") => Some(Message::Stop),
                    _ => None,
                }
            } else {
                None
            }
        });

        Subscription::batch(vec![
            ticks,
            keys,
            Subscription::run(mpris_commands),
            Subscription::run(art_results),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // While an animation owns the screen, user input (clicks, keys, media
        // keys) is dropped so it can't load or unload a tape mid-animation.
        if self.screen.animating()
            && matches!(
                message,
                Message::TogglePlay
                    | Message::Stop
                    | Message::Next
                    | Message::Prev
                    | Message::Seek(_)
                    | Message::WindStart(_)
                    | Message::WindStop
                    | Message::ToggleSkipMode
                    | Message::ShowRack
                    | Message::ShowPlayer
                    | Message::LabelClicked
                    | Message::AlbumSelected(_)
                    | Message::CloseReveal
                    | Message::OpenCase { .. }
                    | Message::CycleSkin
                    | Message::OpenFolder
                    | Message::Mpris(_)
            )
        {
            return Task::none();
        }
        match message {
            Message::Tick(now) => {
                let dt = now.duration_since(self.last_tick).as_secs_f32();
                self.last_tick = now;

                if self.rack_fade.is_some_and(|t0| now.duration_since(t0) >= RACK_FADE) {
                    self.rack_fade = None;
                }

                // Case-open animation (open → lift → fade → slide in → seat).
                if let Screen::CaseOpen {
                    album,
                    track,
                    started,
                } = self.screen
                {
                    if now.duration_since(started) >= CASE_ANIM {
                        // Tape drops in and seats as the case-open animation ends.
                        let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::TapeSeat));
                        self.load_track(album, track);
                        // load_track sets screen = Player.
                    }
                    return Task::none();
                }

                // Eject: when the tape is out, unload it and open the rack.
                if let Screen::Eject { started } = self.screen {
                    if now.duration_since(started) >= EJECT_ANIM {
                        self.unload_tape();
                    }
                    return Task::none();
                }

                // Tape swap: when the new tape seats, load its track.
                if let Screen::Swap {
                    album,
                    track,
                    started,
                } = self.screen
                {
                    if now.duration_since(started) >= SWAP_ANIM {
                        let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::TapeSeat));
                        self.load_track(album, track);
                    }
                    return Task::none();
                }

                // Hold-to-wind: audio is paused; spin the position locally at
                // wind speed and whirl the spools. The real seek happens once,
                // on WindStop — per-tick decoder seeks would stutter.
                if let Some(dir) = self.wind {
                    const WIND_RATE: f32 = 12.0; // 12× tape speed (audio position)
                    const WIND_VISUAL: f32 = 3.0; // spool spin multiplier
                    let total = self.duration().as_secs_f32();
                    if total > 0.0 {
                        let cur = self.position.as_secs_f32();
                        let target = (cur + dt * WIND_RATE * dir as f32).clamp(0.0, total);
                        self.position = Duration::from_secs_f32(target);
                        let (w_src, w_tk) = spool_omegas(self.progress());
                        let k = WIND_VISUAL * dir as f32;
                        self.angle_src = spin(self.angle_src, w_src * dt * k);
                        self.angle_tk = spin(self.angle_tk, w_tk * dt * k);
                    }
                    return Task::none();
                }

                let (finished, error) = {
                    let mut st = self.status.lock().unwrap();
                    self.position = st.position;
                    self.playing = st.playing;
                    (std::mem::take(&mut st.track_finished), st.error.take())
                };
                if let Some(e) = error {
                    self.notice = Some(e);
                }

                if finished {
                    // Auto-stop clunk as the tape runs out, before we advance.
                    let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::EndClack));
                    self.step_track(1); // auto-advance (crosses albums)
                    return Task::none();
                }

                if let Some((dir, left)) = self.skip_anim {
                    const SKIP_VISUAL: f32 = 3.0;
                    let (w_src, w_tk) = spool_omegas(self.progress());
                    let k = SKIP_VISUAL * dir as f32;
                    self.angle_src = spin(self.angle_src, w_src * dt * k);
                    self.angle_tk = spin(self.angle_tk, w_tk * dt * k);
                    let left = left - dt;
                    self.skip_anim = (left > 0.0).then_some((dir, left));
                } else {
                    // Reels ease toward play speed or toward a stop.
                    let target = if self.playing { 1.0 } else { 0.0 };
                    let k = 1.0 - (-dt / REEL_TAU).exp();
                    self.reel_speed += (target - self.reel_speed) * k;
                    if target == 0.0 && self.reel_speed < 0.01 {
                        self.reel_speed = 0.0;
                    }
                    if self.reel_speed > 0.0 {
                        let (w_src, w_tk) = spool_omegas(self.progress());
                        self.angle_src = spin(self.angle_src, w_src * dt * self.reel_speed);
                        self.angle_tk = spin(self.angle_tk, w_tk * dt * self.reel_speed);
                    }
                }

                // Periodic state save so a crash/quit resumes close to here.
                if self.playing && self.last_save.elapsed() >= STATE_SAVE_EVERY {
                    self.last_save = now;
                    self.save_state();
                }
            }

            Message::OpenFolder => {
                return cosmic::task::future(async {
                    let dir = rfd::AsyncFileDialog::new()
                        .set_title("Choose a music directory")
                        .pick_folder()
                        .await;
                    Message::FolderChosen(dir.map(|d| d.path().to_path_buf()))
                });
            }

            Message::FolderChosen(Some(dir)) => {
                self.music_dir = Some(dir.clone());
                self.save_state();
                self.scanning = true;
                return scan_task(dir);
            }
            Message::FolderChosen(None) => {}

            Message::LibraryLoaded { albums, skipped } => {
                self.scanning = false;
                self.library = albums;
                let _ = self.audio_tx.send(AudioCmd::Stop);
                self.current = None;
                self.player_backdrop = None;
                self.label_img = None;
                self.playing = false;
                self.stopped = false;
                self.screen = Screen::Rack;
                self.browse_index = 0;
                self.notice = (skipped > 0)
                    .then(|| format!("Skipped {skipped} unreadable file(s) during scan"));

                // Session resume: reload last tape, paused where it was.
                if let Some((path, pos)) = self.pending_resume.take() {
                    if let Some((a, t)) = find_track(&self.library, &path) {
                        self.resume_track(a, t, pos);
                    }
                }

                self.refresh_rack_overlay();
                self.start_art_scrape();
            }

            Message::ArtLoaded(result) => {
                // Match the fetched cover to its album; keep the raw bytes for
                // compositing onto cases, store a Handle for the reveal, and
                // rebuild the fan so the new cover appears.
                if let Some(idx) = self
                    .library
                    .iter()
                    .position(|a| album_art_key(a) == result.key)
                {
                    let bytes = std::sync::Arc::new(result.bytes);
                    self.cover_bytes.insert(idx, bytes.clone());
                    // If this cover is for the tape currently in the deck,
                    // rebuild its backdrop now that art has arrived.
                    if self.current.map(|(a, _)| a) == Some(idx) {
                        self.refresh_backdrop();
                    }
                    self.library[idx].art = Some(
                        cosmic::widget::image::Handle::from_bytes((*bytes).clone()),
                    );
                    // Only recompose the fan if this album is in the visible
                    // window (centre ± FAN_EACH_SIDE, wrapping around the ends).
                    let n = self.library.len();
                    let d = crate::rack_scene::FAN_EACH_SIDE as i64;
                    let cur = self.browse_index as i64;
                    let this = idx as i64;
                    let nn = n as i64;
                    let in_window = (-d..=d).any(|off| (cur + off).rem_euclid(nn) == this);
                    if in_window {
                        self.rebuild_fan();
                    }
                }
            }

            Message::ShowRack => {
                // Jump the rack to the album that's currently playing.
                if let Some((album, _track)) = self.current {
                    if album < self.library.len() {
                        self.browse_index = album;
                        self.rebuild_fan();
                    }
                }
                self.screen = Screen::Rack;
            }
            Message::ShowPlayer => self.screen = Screen::Player,
            Message::ToggleAbout => self.show_about = !self.show_about,
            Message::ToggleBackdrop => {
                self.show_backdrop = !self.show_backdrop;
                self.save_state();
            }
            Message::OpenRepo => {
                // Hand the repo URL to the desktop's default browser.
                let _ = std::process::Command::new("xdg-open").arg(REPO_URL).spawn();
            }
            Message::LabelClicked => {
                // Open the currently-playing album's art screen.
                if let Some((album, _track)) = self.current {
                    if album < self.library.len() {
                        self.screen = Screen::Reveal(album);
                    }
                }
            }
            Message::AlbumSelected(i) => self.screen = Screen::Reveal(i),
            Message::CloseReveal => self.screen = Screen::Rack,

            Message::OpenCase { album, track } => {
                // Clicking the reveal case plays the open → lift → insert
                // animation; the track loads when it completes (Tick handler).
                self.incoming_label = self.render_label_for(album, track);
                self.incoming_backdrop = self.backdrop_for(album);
                self.screen = Screen::CaseOpen {
                    album,
                    track,
                    started: Instant::now(),
                };
            }

            Message::TogglePlay => {
                let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::KeyPress));
                if let Some((a, t)) = self.current {
                    if self.stopped {
                        // Tape-deck semantics: Stop keeps the tape loaded;
                        // Play restarts it from the beginning.
                        self.load_track(a, t);
                    } else {
                        let cmd = if self.playing { AudioCmd::Pause } else { AudioCmd::Resume };
                        self.playing = !self.playing;
                        let _ = self.audio_tx.send(cmd);
                        self.push_mpris(if self.playing {
                            MprisUpdate::Playing
                        } else {
                            MprisUpdate::Paused
                        });
                        self.save_state();
                    }
                }
            }

            Message::Stop => {
                let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::KeyPress));
                // Stop/eject key: pressed again while stopped, it ejects.
                if self.stopped && self.current.is_some() && self.screen == Screen::Player {
                    let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::TapeSeat));
                    self.screen = Screen::Eject { started: Instant::now() };
                    return Task::none();
                }
                let _ = self.audio_tx.send(AudioCmd::Stop);
                self.playing = false;
                self.stopped = true;
                self.position = Duration::ZERO;
                self.push_mpris(MprisUpdate::Stopped);
                self.save_state();
            }

            Message::Next => self.step_track(1),
            Message::Prev => self.step_track(-1),

            Message::SetVolume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                let _ = self.audio_tx.send(AudioCmd::Volume(self.volume));
                self.save_state();
            }

            Message::WindStart(dir) => {
                if self.current.is_some() && !self.stopped && self.wind.is_none() {
                    self.wind = Some(dir);
                    self.wind_resume = self.playing;
                    if self.playing {
                        // Real decks mute while winding.
                        let _ = self.audio_tx.send(AudioCmd::Pause);
                    }
                    // Button click, then the FF/REW whirr for as long as held.
                    let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::KeyPress));
                    let _ = self.audio_tx.send(AudioCmd::Wind(true));
                }
            }

            Message::ToggleSkipMode => {
                // FUNCT tab: button click, and stop any wind that was running.
                let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::KeyPress));
                let _ = self.audio_tx.send(AudioCmd::Wind(false));
                if self.wind.take().is_some() {
                    let _ = self.audio_tx.send(AudioCmd::Seek(self.position));
                    if self.wind_resume {
                        let _ = self.audio_tx.send(AudioCmd::Resume);
                        self.playing = true;
                    }
                }
                self.skip_mode = !self.skip_mode;
            }

            Message::WindStop => {
                let _ = self.audio_tx.send(AudioCmd::Wind(false));
                if self.wind.take().is_some() {
                    let _ = self.audio_tx.send(AudioCmd::Seek(self.position));
                    if self.wind_resume {
                        let _ = self.audio_tx.send(AudioCmd::Resume);
                        self.playing = true;
                        self.push_mpris(MprisUpdate::Playing);
                    }
                    self.save_state();
                }
            }

            Message::Seek(frac) => {
                let d = self.duration();
                if d > Duration::ZERO {
                    let target = d.mul_f32(frac.clamp(0.0, 1.0));
                    self.position = target; // optimistic; audio thread confirms
                    let _ = self.audio_tx.send(AudioCmd::Seek(target));
                }
            }

            Message::CycleSkin => {
                self.skin_idx = (self.skin_idx + 1) % self.skins.len();
                // Label dimensions differ per skin — regenerate for the tape.
                if let Some((album, track)) = self.current_track() {
                    let (song, alb) = (track.title.clone(), album.title.clone());
                    self.label_img =
                        Some(label::render(&song, &alb, self.skins[self.skin_idx].spec));
                }
                self.save_state();
            }

            Message::CycleBackground => {
                if !self.backgrounds.is_empty() {
                    self.bg_idx = (self.bg_idx + 1) % self.backgrounds.len();
                    self.save_state();
                }
            }
            Message::PickBackground => {
                return cosmic::task::future(async {
                    let file = rfd::AsyncFileDialog::new()
                        .set_title("Choose a background image")
                        .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                        .pick_file()
                        .await;
                    Message::BackgroundChosen(file.map(|f| f.path().to_path_buf()))
                });
            }
            Message::BackgroundChosen(Some(path)) => {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        self.backgrounds
                            .push(cosmic::widget::image::Handle::from_bytes(bytes));
                        self.bg_sources.push(Some(path));
                        self.bg_idx = self.backgrounds.len() - 1;
                        self.save_state();
                    }
                    Err(e) => self.notice = Some(format!("Couldn't load background: {e}")),
                }
            }
            Message::BackgroundChosen(None) => {}

            Message::RemoveBackground => {
                // Only user-added backgrounds can be removed; built-ins stay.
                if self.bg_sources.get(self.bg_idx).is_some_and(|s| s.is_some()) {
                    self.backgrounds.remove(self.bg_idx);
                    self.bg_sources.remove(self.bg_idx);
                    if self.bg_idx >= self.backgrounds.len() {
                        self.bg_idx = self.backgrounds.len().saturating_sub(1);
                    }
                    self.save_state();
                } else {
                    self.notice = Some(
                        "That's a built-in background — it can't be removed. Cycle to an image you added with ＋ Image to remove it."
                            .into(),
                    );
                }
            }

            Message::BrowseStep(delta) => {
                // Only navigate the fan when actually on the rack view.
                if matches!(self.screen, Screen::Rack) {
                    let n = self.library.len();
                    if n > 0 {
                        let i = (self.browse_index as i64 + delta as i64).rem_euclid(n as i64);
                        self.browse_index = i as usize;
                        self.rebuild_fan();
                    }
                }
            }

            Message::Mpris(cmd) => {
                let mapped = match cmd {
                    MprisCmd::PlayPause => Some(Message::TogglePlay),
                    MprisCmd::Play => (!self.playing).then_some(Message::TogglePlay),
                    MprisCmd::Pause => self.playing.then_some(Message::TogglePlay),
                    MprisCmd::Stop => Some(Message::Stop),
                    MprisCmd::Next => Some(Message::Next),
                    MprisCmd::Prev => Some(Message::Prev),
                };
                if let Some(m) = mapped {
                    return self.update(m);
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        if self.show_about {
            return with_page_bg(self.about_view());
        }
        let content = match self.screen {
            Screen::Player | Screen::Eject { .. } | Screen::Swap { .. } => self.player_view(),
            Screen::Rack => self.rack_view(),
            Screen::Reveal(i) => self.reveal_view(i),
            Screen::CaseOpen { started, .. } => self.case_open_view(started),
        };
        // Backdrop layers behind the content, bottom to top, each with an
        // opacity: the 80s bedroom behind Reveal/CaseOpen, and the album-cover
        // wallpaper behind the Player deck (when on). Animations cross-fade
        // them so the background never jumps. The rack draws its own dimmed
        // bedroom in-canvas. Under everything is the charcoal page.
        let cover = |h: &Option<cosmic::widget::image::Handle>| {
            if self.show_backdrop { h.clone() } else { None }
        };
        let mut layers: Vec<(cosmic::widget::image::Handle, f32)> = Vec::new();
        match self.screen {
            Screen::Player => {
                layers.extend(cover(&self.player_backdrop).map(|h| (h, 1.0)));
            }
            Screen::Eject { started } => {
                // The wallpaper leaves with the tape; the rack fades in after.
                let t = started.elapsed().as_secs_f32() / EJECT_ANIM.as_secs_f32();
                layers.extend(cover(&self.player_backdrop).map(|h| (h, 1.0 - eject_drop(t))));
            }
            Screen::Swap { started, .. } => {
                // Old album's wallpaper cross-fades to the new one's while the
                // window is empty between tapes.
                let t = started.elapsed().as_secs_f32() / SWAP_ANIM.as_secs_f32();
                let k = ((t - 0.40) / 0.10).clamp(0.0, 1.0);
                layers.extend(cover(&self.player_backdrop).map(|h| (h, 1.0 - k)));
                layers.extend(cover(&self.incoming_backdrop).map(|h| (h, k)));
            }
            Screen::CaseOpen { started, .. } => {
                // The bedroom fades out as the deck fades in; the new album's
                // wallpaper (if on) fades in under it, so the screen is already
                // the player's when the tape lands.
                let a = case_open::backdrop_alpha(started.elapsed().as_secs_f32() / CASE_ANIM.as_secs_f32());
                layers.extend(cover(&self.incoming_backdrop).map(|h| (h, 1.0 - a)));
                layers.push((self.current_bg().clone(), a));
            }
            Screen::Reveal(_) => layers.push((self.current_bg().clone(), 1.0)),
            Screen::Rack => {}
        }
        layers.retain(|(_, a)| *a > 0.0);
        let page = if layers.is_empty() {
            content
        } else {
            let mut stack: Vec<Element<Message>> = layers
                .into_iter()
                .map(|(h, a)| {
                    widget::image(h)
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .content_fit(cosmic::iced::ContentFit::Cover)
                        .opacity(a)
                        .into()
                })
                .collect();
            stack.push(container(content).width(Length::Fill).height(Length::Fill).into());
            cosmic::iced::widget::Stack::with_children(stack)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };
        with_page_bg(page)
    }
}

// ── Views ──────────────────────────────────────────────────────────────────

impl App {
    /// Full-view About card: app name, version, and a link to the repo.
    fn about_view(&self) -> Element<'_, Message> {
        let card = widget::column::with_children(vec![
            text::title2("Cassette Deck").into(),
            text::body(format!("Version {}", env!("CARGO_PKG_VERSION"))).into(),
            text::caption(REPO_URL).into(),
            widget::row::with_children(vec![
                button::suggested("Open on GitHub").on_press(Message::OpenRepo).into(),
                button::standard("Close").on_press(Message::ToggleAbout).into(),
            ])
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
        ])
        .spacing(16)
        .align_x(Alignment::Center);
        container(card).center(Length::Fill).padding(24).into()
    }

    fn case_open_view(&self, started: Instant) -> Element<'_, Message> {
        let t = (started.elapsed().as_secs_f32() / CASE_ANIM.as_secs_f32()).clamp(0.0, 1.0);
        let scene = CaseOpenScene {
            skin: &self.skins[self.skin_idx],
            label: self.incoming_label.as_ref(),
            angle_src: self.angle_src,
            angle_tk: self.angle_tk,
            tray: self.case_tray.clone(),
            lid: self.case_lid.clone(),
            lid_flipped: self.case_lid_flipped.clone(),
            tape: self.load_sprite.clone(),
            t,
        };
        let deck = Canvas::new(scene).width(Length::Fill).height(Length::Fill);

        // Mirror loading_view / player_view layout EXACTLY so the deck fits at
        // the same size and the hand-off to the player doesn't jump.
        let seek_row = widget::row::with_children(vec![
            text::monotext(fmt_time(Duration::ZERO)).into(),
            slider(0.0..=1.0, 0.0, |_| Message::Tick(Instant::now()))
                .step(0.001_f32)
                .width(Length::Fill)
                .into(),
            text::monotext(fmt_time(Duration::ZERO)).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let volume_row = widget::row::with_children(vec![
            text::body("Vol").into(),
            slider(0.0..=1.0, self.volume, |_| Message::Tick(Instant::now()))
                .step(0.01_f32)
                .width(Length::Fixed(220.0))
                .into(),
            text::monotext(format!("{:>3.0}%", self.volume * 100.0)).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        // Same button row as the player, so nothing changes when it hands off.
        // (Presses are dropped while the animation runs; see update().)
        let nav = self.deck_nav();

        let content = widget::column::with_capacity(4)
            .spacing(14)
            .padding(16)
            .align_x(Alignment::Center)
            .push(deck)
            .push(seek_row)
            .push(volume_row)
            .push(nav);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// The Player's button row, also shown during the insert animation.
    fn deck_nav(&self) -> Element<'_, Message> {
        let mut nav_buttons: Vec<Element<Message>> = vec![
            button::suggested("Music Rack").on_press(Message::ShowRack).into(),
            button::suggested("Open Folder").on_press(Message::OpenFolder).into(),
            button::suggested("About").on_press(Message::ToggleAbout).into(),
            button::suggested(if self.show_backdrop {
                "Wallpaper: On"
            } else {
                "Wallpaper: Off"
            })
            .on_press(Message::ToggleBackdrop)
            .into(),
        ];
        // Skin picker only when more than one skin exists (belt-only: hidden).
        if self.skins.len() > 1 {
            nav_buttons.push(
                button::suggested(format!("Walkman: {} ⟳", self.skins[self.skin_idx].spec.name))
                    .on_press(Message::CycleSkin)
                    .into(),
            );
        }
        widget::row::with_children(nav_buttons).spacing(12).into()
    }

    fn player_view(&self) -> Element<'_, Message> {
        // How far the tape is out of the window, and which label it carries:
        // seated normally, empty with no tape, moving during eject / swap.
        let (drop, label) = match self.screen {
            Screen::Eject { started } => {
                let t = started.elapsed().as_secs_f32() / EJECT_ANIM.as_secs_f32();
                (eject_drop(t), self.label_img.as_ref())
            }
            Screen::Swap { started, .. } => {
                let t = started.elapsed().as_secs_f32() / SWAP_ANIM.as_secs_f32();
                let (d, incoming) = swap_drop(t.clamp(0.0, 1.0));
                let l = if incoming { self.incoming_label.as_ref() } else { self.label_img.as_ref() };
                (d, l)
            }
            _ if self.current.is_none() => (1.0, None),
            _ => (0.0, self.label_img.as_ref()),
        };
        let scene = CassetteScene {
            skin: &self.skins[self.skin_idx],
            label,
            angle_src: self.angle_src,
            angle_tk: self.angle_tk,
            on_toggle_play: Message::TogglePlay,
            on_stop: Message::Stop,
            on_wind_fwd: Message::WindStart(1),
            on_wind_back: Message::WindStart(-1),
            on_wind_stop: Message::WindStop,
            on_next: Message::Next,
            on_prev: Message::Prev,
            on_toggle_mode: Message::ToggleSkipMode,
            on_label_click: Message::LabelClicked,
            skip_mode: self.skip_mode,
            // The charcoal page shows through the letterbox; covers too.
            fill_letterbox: false,
            drop,
            interactive: !self.screen.animating(),
            play_latched: self.screen == Screen::Player
                && self.current.is_some()
                && self.playing
                && !self.stopped,
        };

        let deck = Canvas::new(scene).width(Length::Fill).height(Length::Fill);

        let seek_row = widget::row::with_children(vec![
            text::monotext(fmt_time(self.position)).into(),
            slider(0.0..=1.0, self.progress(), Message::Seek)
                .step(0.001_f32)
                .width(Length::Fill)
                .into(),
            text::monotext(fmt_time(self.duration())).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let volume_row = widget::row::with_children(vec![
            text::body("Vol").into(),
            slider(0.0..=1.0, self.volume, Message::SetVolume)
                .step(0.01_f32)
                .width(Length::Fixed(220.0))
                .into(),
            text::monotext(format!("{:>3.0}%", self.volume * 100.0)).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

let nav = self.deck_nav();

        let mut content = widget::column::with_capacity(7)
            .spacing(14)
            .padding(16)
            .align_x(Alignment::Center)
            .push(deck);

        if self.current.is_none() {
            content =
                content.push(text::body("No tape loaded — pick a song from the Music Rack."));
        }
        if let Some(n) = &self.notice {
            content = content.push(text::caption(n.clone()));
        }

        content = content
            .push(seek_row)
            .push(volume_row)
            .push(nav);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn rack_view(&self) -> Element<'_, Message> {
        let header = widget::row::with_children(vec![
            text::title3("Cassette Rack").into(),
            widget::Space::new().width(Length::Fill).into(),
            button::standard("Backdrop ⟳").on_press(Message::CycleBackground).into(),
            button::standard("＋ Image").on_press(Message::PickBackground).into(),
            button::standard("🗑 Remove").on_press(Message::RemoveBackground).into(),
            button::standard("Player").on_press(Message::ShowPlayer).into(),
            button::standard("About").on_press(Message::ToggleAbout).into(),
            button::suggested("Open Folder").on_press(Message::OpenFolder).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        if self.scanning {
            let body = container(text::body("Scanning your tapes…")).center(Length::Fill);
            return widget::column::with_capacity(2)
                .spacing(12)
                .padding(20)
                .push(header)
                .push(body)
                .into();
        }
        if self.library.is_empty() {
            let body = container(
                widget::column::with_children(vec![
                    text::title4("The rack is empty").into(),
                    text::body("Open a folder of music (MP3, FLAC, M4A, WAV) to fill the rack.")
                        .into(),
                    button::suggested("Open Folder").on_press(Message::OpenFolder).into(),
                ])
                .spacing(12)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill);
            return widget::column::with_capacity(2)
                .spacing(12)
                .padding(20)
                .push(header)
                .push(body)
                .into();
        }

        // The fanned-cases browser over the bedroom background.
        let scene = RackScene {
            background: self.current_bg(),
            cases: &self.fan_cases,
            albums: &self.library,
            current: self.browse_index,
            fade: self.rack_fade.map(|t0| {
                let k = (t0.elapsed().as_secs_f32() / RACK_FADE.as_secs_f32()).clamp(0.0, 1.0);
                (&self.page_fill, 1.0 - k * (2.0 - k))
            }),
        };
        let canvas = Canvas::new(scene).width(Length::Fill).height(Length::Fill);

        // Current album title + counter + Play, below the fan. Show scrape
        // progress so the user knows covers are still arriving.
        let cur = self.library.get(self.browse_index);
        let title = cur
            .map(|a| format!("{} – {}", a.artist, a.title))
            .unwrap_or_default();
        let covered = self
            .library
            .iter()
            .enumerate()
            .filter(|(i, a)| a.art_bytes.is_some() || self.cover_bytes.contains_key(i))
            .count();
        let total = self.library.len();
        let counter = if covered < total {
            format!(
                "{} / {}   ·   {} covers loaded",
                self.browse_index + 1,
                total,
                covered
            )
        } else {
            format!("{} / {}", self.browse_index + 1, total)
        };

        let controls = widget::column::with_capacity(3)
            .spacing(8)
            .align_x(Alignment::Center)
            .push(text::title4(title))
            .push(text::caption(counter))
            .push(
                widget::row::with_capacity(3)
                    .spacing(16)
                    .align_y(Alignment::Center)
                    .push(button::standard("‹ Prev").on_press(Message::BrowseStep(-1)))
                    .push(
                        button::suggested("▶ Play").on_press(Message::OpenCase {
                            album: self.browse_index,
                            track: 0,
                        }),
                    )
                    .push(button::standard("Next ›").on_press(Message::BrowseStep(1))),
            );

        // Canvas takes the flexible space; controls sit in a fixed footer that
        // is always visible (the earlier bug: Fill canvas pushed nav off-screen).
        widget::column::with_capacity(3)
            .spacing(10)
            .padding(20)
            .push(header)
            .push(
                container(canvas)
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .push(
                container(controls)
                    .width(Length::Fill)
                    .center_x(Length::Fill),
            )
            .into()
    }

    /// Case reveal: the selected album's cassette case shown large, the cover
    /// art filling the J-card, with a Play button. Backed by the rack.
    fn reveal_view(&self, idx: usize) -> Element<'_, Message> {
        let Some(album) = self.library.get(idx) else {
            return self.rack_view();
        };

        // Consistent top nav — same controls available as every other screen.
        let header = widget::row::with_children(vec![
            container(text::title2("Music Rack").class(cosmic::iced::Color::WHITE))
                .padding(8)
                .style(|_theme| cosmic::iced::widget::container::Style {
                    background: Some(cosmic::iced::Background::Color(
                        cosmic::iced::Color::from_rgba(0.0, 0.0, 0.0, 0.55),
                    )),
                    border: cosmic::iced::Border {
                        radius: 8.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .into(),
            widget::Space::new().width(Length::Fill).into(),
            button::suggested("Player").on_press(Message::ShowPlayer).into(),
            button::suggested("Music Rack").on_press(Message::ShowRack).into(),
            button::suggested("About").on_press(Message::ToggleAbout).into(),
            button::suggested("Open Folder").on_press(Message::OpenFolder).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        // Compose the cover onto the TRANSPARENT case (no black box) — same as
        // the fan. Prefer scraped bytes, else embedded art.
        let cover_bytes = self
            .cover_bytes
            .get(&idx)
            .map(|b| b.as_slice())
            .or_else(|| album.art_bytes.as_ref().map(|b| b.as_slice()));
        let composed = crate::case_scene::compose(&self.case_png, cover_bytes);
        // Clicking the case plays the open → lift → insert animation.
        let case: Element<Message> = widget::mouse_area(
            widget::image(composed).width(Length::Fixed(340.0)),
        )
        .on_press(Message::OpenCase { album: idx, track: 0 })
        .into();

        let title = text::title3(format!("{} – {}", album.artist, album.title));
        // If THIS album is the one playing, also show the current song title.
        let now_playing: Option<Element<Message>> = match self.current {
            Some((a, t)) if a == idx => album
                .tracks
                .get(t)
                .map(|track| text::body(format!("♪ Now playing: {}", track.title)).into()),
            _ => None,
        };
        // Play/Pause reflects state: if THIS album is the one playing, show
        // Pause (and toggle); otherwise show Play (and start it).
        let this_album_playing =
            matches!(self.current, Some((a, _)) if a == idx) && self.playing;
        let play_btn = if this_album_playing {
            button::suggested("⏸ Pause").on_press(Message::TogglePlay)
        } else if matches!(self.current, Some((a, _)) if a == idx) {
            // This album is loaded but paused — resume via toggle.
            button::suggested("▶ Play").on_press(Message::TogglePlay)
        } else {
            // A different (or no) album — start this one via the case-open.
            button::suggested("▶ Play").on_press(Message::OpenCase { album: idx, track: 0 })
        };
        // Play + Back side by side, in a fixed row that always stays on screen.
        let buttons = widget::row::with_capacity(2)
            .spacing(12)
            .align_y(Alignment::Center)
            .push(play_btn)
            .push(button::suggested("← Back to rack").on_press(Message::CloseReveal));

        // Case takes the flexible middle; title + buttons pinned in a footer
        // that's always visible (the case alone must not consume the buttons).
        let mut body = widget::column::with_capacity(4)
            .spacing(14)
            .align_x(Alignment::Center)
            .push(container(case).center_x(Length::Fill).height(Length::Fill))
            .push(title);
        if let Some(np) = now_playing {
            body = body.push(np);
        }
        let body = body.push(buttons);

        // Header pinned at top; body (case + footer controls) fills the rest.
        widget::column::with_capacity(2)
            .spacing(10)
            .padding(20)
            .push(header)
            .push(body)
            .into()
    }
}

// ── MPRIS command bridge ────────────────────────────────────────────────────

/// Streams media-key commands out of the MPRIS thread into the update loop.
/// Stable identity for an album, shared by the scraper cache and result match.
/// Raw bytes of the transparent case sprite, for compositing covers.
/// Decode cover art into a full-window backdrop handle. Runs once per track
/// load, not per frame.
fn cover_backdrop(bytes: &[u8]) -> Option<cosmic::widget::image::Handle> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(cosmic::widget::image::Handle::from_rgba(w, h, rgba.into_raw()))
}

const CASE_T_PNG: &[u8] = include_bytes!("../assets/case_transparent.png");
/// Empty-deck image + transparent cassette sprite for the load animation.
const CASSETTE_SPRITE_PNG: &[u8] = include_bytes!("../assets/cassette_sprite.png");
/// Keyed open cassette-case sprite (green removed), for the case-open animation.
const CASE_OPEN_PNG: &[u8] = include_bytes!("../assets/case_open.png");
/// Extra bundled room backgrounds — drop PNGs in assets/ and list them here to
/// grow the built-in set the "Backdrop ⟳" button cycles through.
const EXTRA_BGS: &[&[u8]] = &[
    // include_bytes!("../assets/backgrounds/arcade.png"),
];

/// A flat solid-colour backdrop (a tiny image stretched to fill).
/// The one background colour behind every screen: the same charcoal the
/// deck's letterbox uses, so the area around the deck and behind the controls
/// never changes shade between screens or during the animations.
const PAGE_BG: cosmic::iced::Color = cosmic::iced::Color::from_rgb(0.09, 0.09, 0.11);

fn with_page_bg(content: Element<'_, Message>) -> Element<'_, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .class(cosmic::theme::Container::custom(|_| cosmic::iced::widget::container::Style {
            background: Some(cosmic::iced::Background::Color(PAGE_BG)),
            ..Default::default()
        }))
        .into()
}

fn solid_bg(rgb: [u8; 3]) -> cosmic::widget::image::Handle {
    let px = [rgb[0], rgb[1], rgb[2], 255];
    let mut buf = Vec::with_capacity(4 * 4 * 4);
    for _ in 0..16 {
        buf.extend_from_slice(&px);
    }
    cosmic::widget::image::Handle::from_rgba(4, 4, buf)
}

/// A vertical two-stop gradient backdrop (a 1×N strip stretched to fill).
fn gradient_bg(top: [u8; 3], bot: [u8; 3]) -> cosmic::widget::image::Handle {
    let h = 64u32;
    let mut buf = Vec::with_capacity((h * 4) as usize);
    for y in 0..h {
        let t = y as f32 / (h - 1) as f32;
        for c in 0..3 {
            buf.push((top[c] as f32 * (1.0 - t) + bot[c] as f32 * t) as u8);
        }
        buf.push(255);
    }
    cosmic::widget::image::Handle::from_rgba(1, h, buf)
}

fn album_art_key(a: &Album) -> String {
    format!("{}\t{}", a.artist, a.title)
}

/// Receiver for scraped covers, handed to the subscription stream once.
static ART_RX: std::sync::OnceLock<std::sync::Mutex<Option<std::sync::mpsc::Receiver<ArtResult>>>> =
    std::sync::OnceLock::new();

/// Bridges the blocking art-scrape thread (std mpsc) into the iced update loop.
/// Polls the receiver on a short timer; ends when the channel closes.
fn art_results() -> impl cosmic::iced::futures::Stream<Item = Message> {
    let rx = ART_RX.get().and_then(|m| m.lock().ok()?.take());
    cosmic::iced::futures::stream::unfold(rx, |rx| async move {
        let rx = rx?;
        loop {
            match rx.try_recv() {
                Ok(result) => return Some((Message::ArtLoaded(result), Some(rx))),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            }
        }
    })
}

fn mpris_commands() -> impl cosmic::iced::futures::Stream<Item = Message> {
    let rx = mpris::CMD_RX.get().and_then(|m| m.lock().ok()?.take());
    cosmic::iced::futures::stream::unfold(rx, |mut rx| async move {
        let cmd = rx.as_mut()?.recv().await?;
        Some((Message::Mpris(cmd), rx))
    })
}

// ── Persistence ─────────────────────────────────────────────────────────────


/// Advance a spool angle, clamping the per-frame step so fast winds never
/// exceed the wagon-wheel aliasing limit of the spoked hubs (~0.35 rad/frame
/// against 6–8-fold spoke symmetry). Preserves direction exactly.
fn spin(angle: f32, delta: f32) -> f32 {
    let d = delta.clamp(-0.35, 0.35);
    (angle + d).rem_euclid(std::f32::consts::TAU)
}

fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("cosmic-cassette-deck"))
}

fn state_path() -> Option<PathBuf> {
    Some(config_dir()?.join("state"))
}

fn read_state() -> HashMap<String, String> {
    let Some(path) = state_path() else {
        return HashMap::new();
    };
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Pre-0.3 versions stored only the folder, in a bare `music-dir` file.
fn load_music_dir_legacy() -> Option<PathBuf> {
    let contents = std::fs::read_to_string(config_dir()?.join("music-dir")).ok()?;
    let dir = PathBuf::from(contents.trim());
    dir.is_dir().then_some(dir)
}

fn find_track(library: &[Album], path: &Path) -> Option<(usize, usize)> {
    library.iter().enumerate().find_map(|(a, album)| {
        album
            .tracks
            .iter()
            .position(|t| t.path == path)
            .map(|t| (a, t))
    })
}

fn scan_task(dir: PathBuf) -> Task<Message> {
    cosmic::task::future(async move {
        let (albums, skipped) = tokio::task::spawn_blocking(move || library::scan(dir))
            .await
            .unwrap_or_default();
        Message::LibraryLoaded { albums, skipped }
    })
}

fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

