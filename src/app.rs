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
const STATE_SAVE_EVERY: Duration = Duration::from_secs(5);
/// How long the cassette-load animation plays before the player appears.
const LOAD_ANIM: Duration = Duration::from_millis(850);
/// How long the case-open → walkman animation plays before the player appears.
const CASE_ANIM: Duration = Duration::from_millis(1600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Player,
    Rack,
    /// Case reveal: the selected album's cassette case, big, cover on the
    /// J-card, over the dimmed rack. Play from here or click out to return.
    Reveal(usize),
    /// Cassette-load animation playing before the player appears. Carries the
    /// track to load when the animation finishes, and when it started.
    Loading {
        album: usize,
        track: usize,
        started: Instant,
    },
    /// Case-open animation: the reveal case swings open, the tape lifts out and
    /// drops into the walkman. Carries the track to load and when it started.
    CaseOpen {
        album: usize,
        track: usize,
        started: Instant,
    },
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
    PlayTrack { album: usize, track: usize },
    OpenCase { album: usize, track: usize }, // click the reveal case: open it, then load
    TogglePlay,
    Stop,
    Next,
    Prev,
    Seek(f32),   // fraction 0.0..=1.0
    SeekBy(f32), // seconds, signed
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
}

pub struct App {
    core: Core,
    screen: Screen,
    library: Vec<Album>,
    scanning: bool,

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
    /// Pre-built handles for the cassette-load animation (empty deck + sprite).
    load_empty: cosmic::widget::image::Handle,
    load_sprite: cosmic::widget::image::Handle,
    /// Split handles of the open-case sprite for the case-open animation.
    case_tray: cosmic::widget::image::Handle,
    case_lid: cosmic::widget::image::Handle,
    case_lid_flipped: cosmic::widget::image::Handle,

    // Animation state
    angle_src: f32,
    angle_tk: f32,
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
        self.position = Duration::ZERO;
        self.playing = true;
        self.stopped = false;
        self.notice = None;
        // Rasterize the label once per track/skin change — not per frame.
        self.label_img = Some(label::render(&song, &album_name, self.skins[self.skin_idx].spec));
        self.screen = Screen::Player;
        self.push_mpris_metadata();
        self.push_mpris(MprisUpdate::Playing);
        self.save_state();
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
    fn step_track(&mut self, offset: i64) {
        let Some((a, t)) = self.current else { return };
        // Spools whirl briefly in the travel direction of the skip.
        self.skip_anim = Some((offset.signum() as i8, 0.35));

        let len = self.library[a].tracks.len() as i64;
        let next = t as i64 + offset;

        if (0..len).contains(&next) {
            self.load_track(a, next as usize);
        } else if offset > 0 {
            if a + 1 < self.library.len() {
                // Side ran out — next tape on the shelf.
                self.load_track(a + 1, 0);
            } else {
                // End of the library.
                let _ = self.audio_tx.send(AudioCmd::Stop);
                self.playing = false;
                self.stopped = true;
                self.position = Duration::ZERO;
                self.push_mpris(MprisUpdate::Stopped);
            }
        } else if a > 0 {
            let prev = a - 1;
            let last = self.library[prev].tracks.len().saturating_sub(1);
            self.load_track(prev, last);
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

        let mut app = App {
            core,
            screen: Screen::Rack,
            library: Vec::new(),
            scanning: false,
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
            load_empty: cosmic::widget::image::Handle::from_bytes(EMPTY_DECK_PNG.to_vec()),
            load_sprite: cosmic::widget::image::Handle::from_bytes(CASSETTE_SPRITE_PNG.to_vec()),
            case_tray,
            case_lid,
            case_lid_flipped,
            angle_src: 0.0,
            angle_tk: 0.0,
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
        // Tick while a tape is loaded and not explicitly stopped. Gating on
        // `self.playing` alone deadlocks: playing is only updated inside a
        // Tick (status mirror), so one tick seeing a stale paused status
        // (e.g. right after a wind-release Resume) kills the subscription
        // permanently while audio plays on. A loaded, non-stopped deck must
        // always tick; only an empty or stopped deck sleeps.
        let active = (self.current.is_some() && !self.stopped)
            || self.wind.is_some()
            || self.skip_anim.is_some()
            || matches!(self.screen, Screen::Loading { .. } | Screen::CaseOpen { .. });
        let ticks = if active {
            cosmic::iced::time::every(TICK).map(Message::Tick)
        } else {
            Subscription::none()
        };

        // Arrow keys navigate the rack. libcosmic itself uses
        // iced_futures::event::listen_with for keyboard/window subscriptions
        // (see libcosmic src/app/cosmic.rs); the closure takes (event, status,
        // window_id). The BrowseStep handler no-ops unless on the rack screen.
        let keys = cosmic::iced::event::listen_with(|event, _status, _id| {
            use cosmic::iced::keyboard::{key::Named, Event as KeyEvent, Key};
            if let cosmic::iced::Event::Keyboard(KeyEvent::KeyPressed { key, .. }) = event {
                match key {
                    Key::Named(Named::ArrowLeft) => Some(Message::BrowseStep(-1)),
                    Key::Named(Named::ArrowRight) => Some(Message::BrowseStep(1)),
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
        match message {
            Message::Tick(now) => {
                let dt = now.duration_since(self.last_tick).as_secs_f32();
                self.last_tick = now;

                // If the cassette-load animation is playing, advance it. When
                // it completes, load the track and switch to the player.
                if let Screen::Loading {
                    album,
                    track,
                    started,
                } = self.screen
                {
                    if now.duration_since(started) >= LOAD_ANIM {
                        // Tape clicks into the transport as the animation lands.
                        let _ = self.audio_tx.send(AudioCmd::Sfx(audio::Sfx::TapeSeat));
                        self.load_track(album, track);
                        // load_track sets screen = Player.
                    }
                    // Nothing else to advance during the animation.
                    return Task::none();
                }

                // Same for the case-open animation (open → lift → drop → seat).
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
                } else if self.playing {
                    let (w_src, w_tk) = spool_omegas(self.progress());
                    self.angle_src = spin(self.angle_src, w_src * dt);
                    self.angle_tk = spin(self.angle_tk, w_tk * dt);
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

            Message::PlayTrack { album, track } => {
                // Play the cassette-load animation first; the track actually
                // loads when the animation completes (see Tick handler).
                self.screen = Screen::Loading {
                    album,
                    track,
                    started: Instant::now(),
                };
            }

            Message::OpenCase { album, track } => {
                // Clicking the reveal case plays the open → lift → insert
                // animation; the track loads when it completes (Tick handler).
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
                let _ = self.audio_tx.send(AudioCmd::Stop);
                self.playing = false;
                self.stopped = true;
                self.position = Duration::ZERO;
                self.push_mpris(MprisUpdate::Stopped);
                self.save_state();
            }

            Message::Next => self.step_track(1),
            Message::Prev => self.step_track(-1),

            Message::SeekBy(secs) => {
                let total = self.duration().as_secs_f32();
                if total > 0.0 && !self.stopped {
                    let target = (self.position.as_secs_f32() + secs).clamp(0.0, total);
                    self.position = Duration::from_secs_f32(target);
                    let _ = self.audio_tx.send(AudioCmd::Seek(self.position));
                }
            }

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

    fn view(&self) -> Element<Message> {
        let content = match self.screen {
            Screen::Player => self.player_view(),
            Screen::Rack => self.rack_view(),
            Screen::Reveal(i) => self.reveal_view(i),
            Screen::Loading { started, .. } => self.loading_view(started),
            Screen::CaseOpen { started, .. } => self.case_open_view(started),
        };
        // The 80s bedroom sits behind the Reveal screen. The rack draws its own
        // dimmed bedroom in-canvas, and the Player is the self-contained jeans
        // deck (a full scene of its own — a room behind it would clash).
        if matches!(self.screen, Screen::Reveal(_) | Screen::CaseOpen { .. }) {
            let bg = widget::image(self.current_bg().clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(cosmic::iced::ContentFit::Cover);
            cosmic::iced::widget::stack![
                bg,
                container(content).width(Length::Fill).height(Length::Fill),
            ]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        } else {
            content
        }
    }
}

// ── Views ──────────────────────────────────────────────────────────────────

impl App {
    fn loading_view(&self, started: Instant) -> Element<Message> {
        let t = (started.elapsed().as_secs_f32() / LOAD_ANIM.as_secs_f32()).clamp(0.0, 1.0);
        let scene = crate::loading::LoadingScene {
            empty: self.skins[self.skin_idx].empty.clone(),
            sprite: self.load_sprite.clone(),
            t,
        };
        let deck = Canvas::new(scene).width(Length::Fill).height(Length::Fill);

        // Mirror the player_view layout EXACTLY so the deck renders at the same
        // size and position — same column, padding, spacing, and the same
        // control rows (shown disabled) reserving the same vertical space.
        // Otherwise the deck jumps/resizes when the animation hands off.
        let seek_row = widget::row::with_children(vec![
            text::monotext(fmt_time(Duration::ZERO)).into(),
            slider(0.0..=1.0, 0.0, |_| Message::Tick(Instant::now()))
                .step(0.001)
                .width(Length::Fill)
                .into(),
            text::monotext(fmt_time(Duration::ZERO)).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let volume_row = widget::row::with_children(vec![
            text::body("Vol").into(),
            slider(0.0..=1.0, self.volume, |_| Message::Tick(Instant::now()))
                .step(0.01)
                .width(Length::Fixed(220.0))
                .into(),
            text::monotext(format!("{:>3.0}%", self.volume * 100.0)).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let nav = widget::row::with_children(vec![
            button::standard("Music Rack").into(),
            button::standard("Open Folder").into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

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

    fn case_open_view(&self, started: Instant) -> Element<Message> {
        let t = (started.elapsed().as_secs_f32() / CASE_ANIM.as_secs_f32()).clamp(0.0, 1.0);
        let scene = CaseOpenScene {
            bedroom: self.current_bg().clone(),
            empty: self.skins[self.skin_idx].empty.clone(),
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
                .step(0.001)
                .width(Length::Fill)
                .into(),
            text::monotext(fmt_time(Duration::ZERO)).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let volume_row = widget::row::with_children(vec![
            text::body("Vol").into(),
            slider(0.0..=1.0, self.volume, |_| Message::Tick(Instant::now()))
                .step(0.01)
                .width(Length::Fixed(220.0))
                .into(),
            text::monotext(format!("{:>3.0}%", self.volume * 100.0)).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let nav = widget::row::with_children(vec![
            button::standard("Music Rack").into(),
            button::standard("Open Folder").into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

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

    fn player_view(&self) -> Element<Message> {
        let scene = CassetteScene {
            skin: &self.skins[self.skin_idx],
            label: self.label_img.as_ref(),
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
            on_eject: Message::ShowRack,
            on_label_click: Message::LabelClicked,
            skip_mode: self.skip_mode,
        };

        let deck = Canvas::new(scene).width(Length::Fill).height(Length::Fill);

        let seek_row = widget::row::with_children(vec![
            text::monotext(fmt_time(self.position)).into(),
            slider(0.0..=1.0, self.progress(), Message::Seek)
                .step(0.001)
                .width(Length::Fill)
                .into(),
            text::monotext(fmt_time(self.duration())).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let volume_row = widget::row::with_children(vec![
            text::body("Vol").into(),
            slider(0.0..=1.0, self.volume, Message::SetVolume)
                .step(0.01)
                .width(Length::Fixed(220.0))
                .into(),
            text::monotext(format!("{:>3.0}%", self.volume * 100.0)).into(),
        ])
        .spacing(12)
        .align_y(Alignment::Center);

        let mut nav_buttons: Vec<Element<Message>> = vec![
            button::standard("Music Rack").on_press(Message::ShowRack).into(),
            button::standard("Open Folder").on_press(Message::OpenFolder).into(),
        ];
        // Skin picker only when more than one skin exists (belt-only: hidden).
        if self.skins.len() > 1 {
            nav_buttons.push(
                button::standard(format!("Walkman: {} ⟳", self.skins[self.skin_idx].spec.name))
                    .on_press(Message::CycleSkin)
                    .into(),
            );
        }
        let nav = widget::row::with_children(nav_buttons).spacing(12);

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

    fn rack_view(&self) -> Element<Message> {
        let header = widget::row::with_children(vec![
            text::title3("Cassette Rack").into(),
            widget::Space::new().width(Length::Fill).into(),
            button::standard("Backdrop ⟳").on_press(Message::CycleBackground).into(),
            button::standard("＋ Image").on_press(Message::PickBackground).into(),
            button::standard("🗑 Remove").on_press(Message::RemoveBackground).into(),
            button::standard("Player").on_press(Message::ShowPlayer).into(),
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
                    text::body("Open a folder of music (MP3, FLAC, OGG, M4A, WAV) to fill the rack.")
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
    fn reveal_view(&self, idx: usize) -> Element<Message> {
        let Some(album) = self.library.get(idx) else {
            return self.rack_view();
        };

        // Consistent top nav — same controls available as every other screen.
        let header = widget::row::with_children(vec![
            text::title2("Music Rack").into(),
            widget::Space::new().width(Length::Fill).into(),
            button::suggested("Player").on_press(Message::ShowPlayer).into(),
            button::suggested("Music Rack").on_press(Message::ShowRack).into(),
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
const CASE_T_PNG: &[u8] = include_bytes!("../assets/case_transparent.png");
/// Empty-deck image + transparent cassette sprite for the load animation.
const EMPTY_DECK_PNG: &[u8] = include_bytes!("../assets/skins/belt/empty.png");
const CASSETTE_SPRITE_PNG: &[u8] = include_bytes!("../assets/cassette_sprite.png");
/// Keyed open cassette-case sprite (green removed), for the case-open animation.
const CASE_OPEN_PNG: &[u8] = include_bytes!("../assets/case_open.png");
/// Extra bundled room backgrounds — drop PNGs in assets/ and list them here to
/// grow the built-in set the "Backdrop ⟳" button cycles through.
const EXTRA_BGS: &[&[u8]] = &[
    // include_bytes!("../assets/backgrounds/arcade.png"),
];

/// A flat solid-colour backdrop (a tiny image stretched to fill).
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
