//! Audio engine on a dedicated OS thread.
//!
//! GUI ──AudioCmd (mpsc)──▶ audio thread (owns OutputStream + Sink)
//! GUI ◀──SharedStatus (Arc<Mutex>)── audio thread (refreshed every ≤50 ms)
//!
//! Device resilience: a FRESH OutputStream is created on every Load, so the
//! current system default device is picked up at each track change (dock,
//! Bluetooth, etc.). A device change mid-track still requires a track change
//! or stop/play to take effect — rodio has no hotplug events to react to.
//!
//! Sound effects: a SECOND, persistent OutputStream lives on this same thread
//! (created once, never rebuilt), so mechanism/UI cues play even with no track
//! loaded and survive the per-Load music-stream rebuild. One-shots are fired
//! detached via `play_raw`; the wind (FF/REW) whirr owns a persistent `Sink`.
//! The effects output is opened against the default device at startup and does
//! not follow later device changes; acceptable for short UI feedback.

use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rodio::Source; // for convert_samples()/amplify() combinators on our source

#[derive(Debug)]
pub enum AudioCmd {
    /// Stop whatever is playing, decode `path`, start playback from 0.
    Load(PathBuf),
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    /// Set volume, 0.0..=1.0.
    Volume(f32),
    /// Fire a one-shot mechanism/UI cue.
    Sfx(Sfx),
    /// Start (true) / stop (false) the fast-forward / rewind whirr loop.
    Wind(bool),
    /// The deck eats the tape: play the loaded song from here, going wrong
    /// (see `tape_eat::chew`), then stop dead.
    Chew(Duration),
    /// Back in the deck after the tape was wound back: play the loaded song
    /// from here, with a short warble as the crinkled stretch goes past.
    Unchew(Duration),
}

/// One-shot sound effects (mechanism / UI feedback).
#[derive(Debug, Clone, Copy)]
pub enum Sfx {
    /// A transport button was pressed (play / stop / wind / funct).
    KeyPress,
    /// A cassette seats into the transport (load / case-open animation lands).
    TapeSeat,
    /// Auto-stop clunk when a tape reaches its end.
    EndClack,
    /// The cassette case's lid unlatches and swings open.
    CaseOpen,
    /// The tape is lifted out of the case.
    TapeLift,
    /// The tape slides down into the deck, just before it seats.
    TapeSlide,
    /// The deck's door springs open to let the tape out.
    EjectPop,
    /// The headphone plug pulled out of its jack.
    PlugOut,
    /// The headphone plug pushed back in.
    PlugIn,
    /// The deck chewing the tape.
    TapeCrunch,
    /// A pencil winding the slack back into the cassette.
    PencilWind,
}

#[derive(Debug, Default, Clone)]
pub struct SharedStatus {
    pub position: Duration,
    pub playing: bool,
    /// Set once when a loaded track drains; GUI consumes it and advances.
    pub track_finished: bool,
    /// One-shot error for the GUI to surface (decode/device failures).
    pub error: Option<String>,
}

pub type Status = Arc<Mutex<SharedStatus>>;

// ── Sound-effect assets ─────────────────────────────────────────────────────
// Embedded FLAC clips, decoded once at startup via `symphonia-flac`.
// NB: rodio's `symphonia-vorbis` feature enables the Vorbis *codec* but not
// the OGG *container* reader, so .ogg files fail to decode here. FLAC needs
// no extra feature and is lossless, so the loop seam stays sample-exact.
// Drop-in replaceable: overwrite the files (same names) and rebuild.
const SFX_KEY_PRESS: &[u8] = include_bytes!("../assets/sfx/key_press.flac");
const SFX_TAPE_SEAT: &[u8] = include_bytes!("../assets/sfx/tape_seat.flac");
const SFX_END_CLACK: &[u8] = include_bytes!("../assets/sfx/end_clack.flac");
const SFX_CASE_OPEN: &[u8] = include_bytes!("../assets/sfx/case_open.flac");
const SFX_TAPE_LIFT: &[u8] = include_bytes!("../assets/sfx/tape_lift.flac");
const SFX_TAPE_SLIDE: &[u8] = include_bytes!("../assets/sfx/tape_slide.flac");
const SFX_EJECT_POP: &[u8] = include_bytes!("../assets/sfx/eject_pop.flac");
const SFX_WIND_LOOP: &[u8] = include_bytes!("../assets/sfx/wind_loop.flac");
const SFX_PLUG_OUT: &[u8] = include_bytes!("../assets/sfx/plug_out.flac");
const SFX_PLUG_IN: &[u8] = include_bytes!("../assets/sfx/plug_in.flac");
const SFX_TAPE_CRUNCH: &[u8] = include_bytes!("../assets/sfx/tape_crunch.flac");
const SFX_PENCIL_WIND: &[u8] = include_bytes!("../assets/sfx/pencil_wind.flac");

/// Effect loudness, independent of the music volume slider — UI feedback
/// shouldn't disappear when the music is turned down.
const VOL_ONESHOT: f32 = 0.7;
const VOL_WIND: f32 = 0.4;

/// Load-time crossfade applied to looped clips so they wrap with no seam
/// (see `Pcm::seamless`). Robust to the few-ms length change lossy codecs
/// introduce, which would otherwise click on a pitched loop.
const LOOP_XFADE_MS: f32 = 40.0;

pub fn spawn(status: Status) -> Sender<AudioCmd> {
    let (tx, rx) = mpsc::channel::<AudioCmd>();

    thread::Builder::new()
        .name("cassette-audio".into())
        .spawn(move || {
            // Music stream is rebuilt per Load; kept here so it lives on this
            // thread. The effects engine owns a *separate* persistent stream.
            #[allow(unused_assignments)]
            let mut output: Option<(rodio::OutputStream, rodio::OutputStreamHandle)> = None;
            let mut sink: Option<rodio::Sink> = None;
            let mut loaded = false;
            let mut volume: f32 = 1.0;
            // The song in the deck, for the tape-eating extra to replay.
            let mut loaded_path: Option<PathBuf> = None;

            // Persistent effects output (None ⇒ effects silently disabled).
            let mut sfx = SfxEngine::new();

            let report = |status: &Status, msg: String| {
                eprintln!("audio: {msg}");
                status.lock().unwrap().error = Some(msg);
            };

            loop {
                match rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(cmd) => match cmd {
                        AudioCmd::Load(path) => {
                            if let Some(old) = sink.take() {
                                old.stop();
                            }
                            // Fresh stream: re-resolves the default device.
                            match rodio::OutputStream::try_default() {
                                Ok(pair) => output = Some(pair),
                                Err(e) => {
                                    report(&status, format!("no audio output device: {e}"));
                                    loaded = false;
                                    continue;
                                }
                            }
                            let handle = &output.as_ref().unwrap().1;
                            loaded_path = Some(path.clone());

                            match open_source(&path) {
                                Ok(source) => match rodio::Sink::try_new(handle) {
                                    Ok(new_sink) => {
                                        new_sink.set_volume(volume);
                                        new_sink.append(source);
                                        new_sink.play();
                                        sink = Some(new_sink);
                                        loaded = true;
                                        let mut st = status.lock().unwrap();
                                        st.position = Duration::ZERO;
                                        st.track_finished = false;
                                        st.playing = true;
                                    }
                                    Err(e) => {
                                        report(&status, format!("audio sink error: {e}"));
                                        loaded = false;
                                    }
                                },
                                Err(e) => {
                                    report(
                                        &status,
                                        format!(
                                            "couldn't play {}: {e}",
                                            path.file_name()
                                                .map(|n| n.to_string_lossy().into_owned())
                                                .unwrap_or_else(|| path.display().to_string())
                                        ),
                                    );
                                    loaded = false;
                                }
                            }
                        }
                        AudioCmd::Pause => {
                            if let Some(s) = &sink {
                                s.pause();
                            }
                        }
                        AudioCmd::Resume => {
                            if let Some(s) = &sink {
                                s.play();
                            }
                        }
                        AudioCmd::Stop => {
                            if let Some(s) = sink.take() {
                                s.stop();
                            }
                            loaded = false;
                            let mut st = status.lock().unwrap();
                            st.position = Duration::ZERO;
                            st.playing = false;
                            st.track_finished = false;
                        }
                        AudioCmd::Seek(pos) => {
                            if let Some(s) = &sink {
                                let _ = s.try_seek(pos);
                            }
                        }
                        AudioCmd::Volume(v) => {
                            volume = v.clamp(0.0, 1.0);
                            if let Some(s) = &sink {
                                s.set_volume(volume);
                            }
                        }
                        AudioCmd::Sfx(cue) => {
                            if let Some(engine) = &sfx {
                                engine.fire(cue);
                            }
                        }
                        AudioCmd::Wind(on) => {
                            if let Some(engine) = &mut sfx {
                                engine.set_wind(on);
                            }
                        }
                        AudioCmd::Chew(at) | AudioCmd::Unchew(at) => {
                            let chew = matches!(cmd, AudioCmd::Chew(_));
                            let (Some(path), Some((_, handle))) = (&loaded_path, &output) else {
                                continue;
                            };
                            if let Some(old) = sink.take() {
                                old.stop();
                            }
                            let started = open_source(path).map_err(|e| e.to_string()).and_then(|dec| {
                                let fx = TapeFx::new(dec.convert_samples(), chew);
                                let s = rodio::Sink::try_new(handle).map_err(|e| e.to_string())?;
                                s.pause();
                                s.set_volume(volume);
                                s.append(fx);
                                let _ = s.try_seek(at);
                                s.play();
                                Ok(s)
                            });
                            match started {
                                Ok(s) => {
                                    sink = Some(s);
                                    // While it chews, the song running out isn't
                                    // the end of the track.
                                    loaded = !chew;
                                    let mut st = status.lock().unwrap();
                                    st.position = at;
                                    st.track_finished = false;
                                    st.playing = true;
                                }
                                Err(e) => {
                                    report(&status, format!("audio error: {e}"));
                                    loaded = false;
                                }
                            }
                        }
                    },
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }

                // The music sink is the single source of truth for "playing".
                let playing = match &sink {
                    Some(s) => !s.is_paused() && !s.empty(),
                    None => false,
                };
                {
                    let mut st = status.lock().unwrap();
                    if let Some(s) = &sink {
                        st.position = s.get_pos();
                        if loaded && s.empty() {
                            st.track_finished = true;
                            loaded = false;
                        }
                    }
                    st.playing = playing;
                }
            }
        })
        .expect("failed to spawn audio thread");

    tx
}

fn open_source(
    path: &PathBuf,
) -> Result<rodio::Decoder<BufReader<File>>, Box<dyn std::error::Error>> {
    let file = BufReader::new(File::open(path)?);
    Ok(rodio::Decoder::new(file)?)
}

// ── Effects engine ──────────────────────────────────────────────────────────

/// Fully-decoded PCM for one clip, cheap to replay (shared, ref-counted).
#[derive(Clone)]
struct Pcm {
    channels: u16,
    sample_rate: u32,
    data: Arc<Vec<i16>>,
}

impl Pcm {
    /// Decode an embedded encoded clip fully into interleaved i16 PCM.
    fn decode(bytes: &'static [u8]) -> Result<Pcm, String> {
        let dec = rodio::Decoder::new(Cursor::new(bytes)).map_err(|e| format!("decode: {e}"))?;
        let channels = dec.channels().max(1);
        let sample_rate = dec.sample_rate().max(1);
        let data: Vec<i16> = dec.collect();
        if data.is_empty() {
            return Err("empty clip".into());
        }
        Ok(Pcm {
            channels,
            sample_rate,
            data: Arc::new(data),
        })
    }

    /// Return a copy trimmed so it loops seamlessly: the head is cross-faded
    /// (raised cosine) into the material that follows the loop point, so
    /// `buf[last] → buf[0]` is continuous. Independent of the source codec's
    /// exact sample count, which is why it survives lossy OGG re-encoding.
    fn seamless(self, xfade_ms: f32) -> Pcm {
        let n = self.data.len();
        let ch = self.channels as usize; // ≥ 1 by construction
        let x = ((self.sample_rate as f32 * xfade_ms / 1000.0) as usize) * ch;
        if x == 0 || n <= 2 * x {
            return self;
        }
        let m = n - x; // looped length (frames * ch)
        let xf = (x / ch).max(1);
        let mut out = self.data[..m].to_vec();
        for i in 0..x {
            let frame = i / ch;
            let w = 0.5 * (1.0 - (std::f32::consts::PI * frame as f32 / xf as f32).cos());
            let head = self.data[i] as f32;
            let cont = self.data[m + i] as f32;
            out[i] = (head * w + cont * (1.0 - w)).round().clamp(-32768.0, 32767.0) as i16;
        }
        Pcm {
            channels: self.channels,
            sample_rate: self.sample_rate,
            data: Arc::new(out),
        }
    }
}

/// A cheap replayable source over shared PCM; finite (one-shot) or looping.
struct PcmSource {
    data: Arc<Vec<i16>>,
    pos: usize,
    channels: u16,
    sample_rate: u32,
    looping: bool,
}

impl PcmSource {
    fn new(pcm: &Pcm, looping: bool) -> Self {
        PcmSource {
            data: pcm.data.clone(),
            pos: 0,
            channels: pcm.channels,
            sample_rate: pcm.sample_rate,
            looping,
        }
    }
}

impl Iterator for PcmSource {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<i16> {
        let n = self.data.len();
        if n == 0 {
            return None;
        }
        if self.pos >= n {
            if self.looping {
                self.pos = 0;
            } else {
                return None;
            }
        }
        let s = self.data[self.pos];
        self.pos += 1;
        Some(s)
    }
}

impl Source for PcmSource {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        if self.looping {
            None
        } else {
            Some(self.data.len().saturating_sub(self.pos))
        }
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.channels
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        if self.looping {
            None
        } else {
            let frames = self.data.len() as u64 / self.channels.max(1) as u64;
            Some(Duration::from_secs_f64(
                frames as f64 / self.sample_rate.max(1) as f64,
            ))
        }
    }
}

/// Owns the persistent effects output and the wind loop sink. Every effect
/// shares one OutputStream that is created once and never rebuilt.
struct SfxEngine {
    // Field order matters for drop: the sink stops before the stream closes.
    wind_sink: Option<rodio::Sink>,
    handle: rodio::OutputStreamHandle,
    _stream: rodio::OutputStream,

    key_press: Option<Pcm>,
    tape_seat: Option<Pcm>,
    end_clack: Option<Pcm>,
    case_open: Option<Pcm>,
    tape_lift: Option<Pcm>,
    tape_slide: Option<Pcm>,
    eject_pop: Option<Pcm>,
    plug_out: Option<Pcm>,
    plug_in: Option<Pcm>,
    tape_crunch: Option<Pcm>,
    pencil_wind: Option<Pcm>,
    wind: Option<Pcm>,
}

impl SfxEngine {
    /// Build the effects output; `None` if no device — effects go silent
    /// while music (which resolves its own device per Load) still works.
    fn new() -> Option<SfxEngine> {
        let (stream, handle) = match rodio::OutputStream::try_default() {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("audio: sound effects disabled, no output device: {e}");
                return None;
            }
        };

        let load = |bytes: &'static [u8], seam: Option<f32>, what: &str| -> Option<Pcm> {
            match Pcm::decode(bytes) {
                Ok(pcm) => Some(match seam {
                    Some(ms) => pcm.seamless(ms),
                    None => pcm,
                }),
                Err(e) => {
                    eprintln!("audio: sfx {what}: {e}");
                    None
                }
            }
        };

        Some(SfxEngine {
            key_press: load(SFX_KEY_PRESS, None, "key_press"),
            tape_seat: load(SFX_TAPE_SEAT, None, "tape_seat"),
            end_clack: load(SFX_END_CLACK, None, "end_clack"),
            case_open: load(SFX_CASE_OPEN, None, "case_open"),
            tape_lift: load(SFX_TAPE_LIFT, None, "tape_lift"),
            tape_slide: load(SFX_TAPE_SLIDE, None, "tape_slide"),
            eject_pop: load(SFX_EJECT_POP, None, "eject_pop"),
            plug_out: load(SFX_PLUG_OUT, None, "plug_out"),
            plug_in: load(SFX_PLUG_IN, None, "plug_in"),
            tape_crunch: load(SFX_TAPE_CRUNCH, None, "tape_crunch"),
            pencil_wind: load(SFX_PENCIL_WIND, None, "pencil_wind"),
            wind: load(SFX_WIND_LOOP, Some(LOOP_XFADE_MS), "wind_loop"),
            wind_sink: None,
            handle,
            _stream: stream,
        })
    }

    /// Play a one-shot cue, detached, mixed over anything already sounding.
    fn fire(&self, cue: Sfx) {
        let pcm = match cue {
            Sfx::KeyPress => &self.key_press,
            Sfx::TapeSeat => &self.tape_seat,
            Sfx::EndClack => &self.end_clack,
            Sfx::CaseOpen => &self.case_open,
            Sfx::TapeLift => &self.tape_lift,
            Sfx::TapeSlide => &self.tape_slide,
            Sfx::EjectPop => &self.eject_pop,
            Sfx::PlugOut => &self.plug_out,
            Sfx::PlugIn => &self.plug_in,
            Sfx::TapeCrunch => &self.tape_crunch,
            Sfx::PencilWind => &self.pencil_wind,
        };
        if let Some(pcm) = pcm {
            let src = PcmSource::new(pcm, false)
                .convert_samples::<f32>()
                .amplify(VOL_ONESHOT);
            let _ = self.handle.play_raw(src);
        }
    }

    /// Start a looping sink for `pcm` at `vol`, or `None` if unavailable.
    fn start_loop(
        pcm: &Option<Pcm>,
        handle: &rodio::OutputStreamHandle,
        vol: f32,
    ) -> Option<rodio::Sink> {
        let pcm = pcm.as_ref()?;
        let sink = rodio::Sink::try_new(handle).ok()?;
        sink.set_volume(vol);
        sink.append(PcmSource::new(pcm, true));
        sink.play();
        Some(sink)
    }

    /// Wind whirr toggled by the transport (idempotent on both edges).
    fn set_wind(&mut self, on: bool) {
        if on {
            if self.wind_sink.is_none() {
                self.wind_sink = Self::start_loop(&self.wind, &self.handle, VOL_WIND);
            }
        } else if let Some(sink) = self.wind_sink.take() {
            sink.stop();
        }
    }
}

// ── The tape-eating extra ───────────────────────────────────────────────────

/// The song as the tape misbehaves: played at a changing speed (the pitch
/// moves with it), through a treble cut that opens and closes, at a changing
/// loudness. Chewing, it goes wrong (`tape_eat::chew`) over the motor's
/// strain and stops dead; afterwards it warbles briefly (`tape_eat::warble`)
/// and then plays on clean, exactly as normal.
struct TapeFx<S: Source<Item = f32>> {
    inner: S,
    chew: bool,
    channels: usize,
    rate_hz: u32,
    /// Two neighbouring input frames, and how far between them we are.
    a: Vec<f32>,
    b: Vec<f32>,
    frac: f64,
    /// The frame being handed out, sample by sample.
    out: Vec<f32>,
    out_i: usize,
    /// Treble-cut memory per channel.
    lp: Vec<f32>,
    /// Frames handed out so far (the effect's clock).
    frames: u64,
    motor_phase: f32,
    motor_lp: f32,
    done: bool,
}

impl<S: Source<Item = f32>> TapeFx<S> {
    fn new(inner: S, chew: bool) -> Self {
        let channels = inner.channels().max(1) as usize;
        let rate_hz = inner.sample_rate().max(1);
        let mut fx = TapeFx {
            inner,
            chew,
            channels,
            rate_hz,
            a: vec![0.0; channels],
            b: vec![0.0; channels],
            frac: 0.0,
            out: vec![0.0; channels],
            out_i: channels,
            lp: vec![0.0; channels],
            frames: 0,
            motor_phase: 0.0,
            motor_lp: 0.0,
            done: false,
        };
        fx.refill();
        fx
    }

    fn read_frame(&mut self, into_b: bool) -> bool {
        for c in 0..self.channels {
            match self.inner.next() {
                Some(v) => {
                    if into_b {
                        self.b[c] = v;
                    } else {
                        self.a[c] = v;
                    }
                }
                None => return false,
            }
        }
        true
    }

    fn refill(&mut self) {
        self.frac = 0.0;
        if !(self.read_frame(false) && self.read_frame(true)) {
            self.done = true;
        }
    }

    /// The next output frame into `out`; false when the song ends (or, while
    /// chewing, when the deck stops dead).
    fn next_frame(&mut self) -> bool {
        if self.done {
            return false;
        }
        let t = self.frames as f32 / self.rate_hz as f32;
        if !self.chew && t >= crate::tape_eat::WARBLE_DONE {
            // Past the crinkled stretch: the song exactly as it is.
            self.out.copy_from_slice(&self.a);
            self.out_i = 0;
            self.frames += 1;
            std::mem::swap(&mut self.a, &mut self.b);
            if !self.read_frame(true) {
                self.done = true;
            }
            return true;
        }
        let (speed, cutoff, gain) = if self.chew {
            if t >= crate::tape_eat::WARBLE {
                self.done = true;
                return false;
            }
            crate::tape_eat::chew(t)
        } else {
            let (speed, cutoff) = crate::tape_eat::warble(t);
            (speed, cutoff, 1.0)
        };
        let k = 1.0 - (-std::f32::consts::TAU * cutoff / self.rate_hz as f32).exp();
        let f = self.frac as f32;
        // The motor straining under the music in the last part of the chew.
        let whine = if self.chew {
            let (hz, amp) = crate::tape_eat::motor(t);
            self.motor_phase = (self.motor_phase + hz / self.rate_hz as f32).fract();
            let saw = 2.0 * self.motor_phase - 1.0;
            let mk = 1.0 - (-std::f32::consts::TAU * 1800.0 / self.rate_hz as f32).exp();
            self.motor_lp += mk * (saw - self.motor_lp);
            self.motor_lp * amp
        } else {
            0.0
        };
        for c in 0..self.channels {
            let v = self.a[c] + (self.b[c] - self.a[c]) * f;
            self.lp[c] += k * (v - self.lp[c]);
            self.out[c] = self.lp[c] * gain + whine;
        }
        self.out_i = 0;
        self.frames += 1;
        self.frac += speed as f64;
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            std::mem::swap(&mut self.a, &mut self.b);
            if !self.read_frame(true) {
                self.done = true;
                break;
            }
        }
        true
    }
}

impl<S: Source<Item = f32>> Iterator for TapeFx<S> {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        if self.out_i >= self.channels && !self.next_frame() {
            return None;
        }
        let v = self.out[self.out_i];
        self.out_i += 1;
        Some(v)
    }
}

impl<S: Source<Item = f32>> Source for TapeFx<S> {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.channels as u16
    }

    fn sample_rate(&self) -> u32 {
        self.rate_hz
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)?;
        self.done = false;
        self.refill();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A steady tone standing in for a song.
    fn tone(secs: f32) -> rodio::source::TakeDuration<rodio::source::SineWave> {
        rodio::source::SineWave::new(440.0).take_duration(Duration::from_secs_f32(secs))
    }

    #[test]
    fn chewing_stops_dead_after_the_warble() {
        let fx = TapeFx::new(tone(10.0), true);
        let rate = fx.sample_rate() as f32;
        let n = fx.count() as f32;
        let secs = n / rate;
        assert!((secs - crate::tape_eat::WARBLE).abs() < 0.01, "{secs}");
    }

    #[test]
    fn after_the_warble_it_plays_clean() {
        let src = tone(5.0);
        let rate = src.sample_rate() as usize;
        let fx: Vec<f32> = TapeFx::new(src, false).collect();
        let plain: Vec<f32> = tone(5.0).collect();
        // Warbling speed means slightly more or less of the song; the length
        // still ends up within a few milliseconds of it.
        assert!((fx.len() as i64 - plain.len() as i64).unsigned_abs() < (rate / 50) as u64);
        // Past the warble, the song comes through untouched, sample for
        // sample (allowing for where the warble left it).
        let tail = &fx[fx.len() - rate..fx.len() - 2];
        let found = (0..rate / 50).any(|shift| {
            let start = plain.len() - rate - shift;
            tail.iter().zip(&plain[start..]).all(|(a, b)| a == b)
        });
        assert!(found, "clean tail doesn't match the song");
    }
}
