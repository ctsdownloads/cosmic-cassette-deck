//! Audio engine on a dedicated OS thread.
//!
//! GUI ──AudioCmd (mpsc)── audio thread (owns OutputStream + Sink)
//! GUI ◀──SharedStatus (Arc<Mutex>)── audio thread (refreshed every ≤50 ms)
//!
//! Device resilience: a FRESH OutputStream is created on every Load, so the
//! current system default device is picked up at each track change (dock,
//! Bluetooth, etc.). A device change mid-track still requires a track change
//! or stop/play to take effect - rodio has no hotplug events to react to.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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

pub fn spawn(status: Status) -> Sender<AudioCmd> {
    let (tx, rx) = mpsc::channel::<AudioCmd>();

    thread::Builder::new()
        .name("cassette-audio".into())
        .spawn(move || {
            // Stream is rebuilt per Load; kept here so it lives on this thread.
            #[allow(unused_assignments)]
            let mut output: Option<(rodio::OutputStream, rodio::OutputStreamHandle)> = None;
            let mut sink: Option<rodio::Sink> = None;
            let mut loaded = false;
            let mut volume: f32 = 1.0;

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
                    },
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }

                let mut st = status.lock().unwrap();
                match &sink {
                    Some(s) => {
                        st.position = s.get_pos();
                        st.playing = !s.is_paused() && !s.empty();
                        if loaded && s.empty() {
                            st.track_finished = true;
                            loaded = false;
                        }
                    }
                    None => st.playing = false,
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
