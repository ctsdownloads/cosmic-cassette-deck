//! MPRIS (org.mpris.MediaPlayer2) integration: media keys, playerctl, and
//! desktop panel controls.
//!
//! Runs on a dedicated thread with its own current-thread tokio runtime,
//! serving D-Bus via the `mpris-server` crate. Two channels bridge it to
//! the GUI:
//!   • commands  (MPRIS -> GUI): media-key presses arrive as `MprisCmd`,
//!     consumed by an iced Subscription stream (see app.rs).
//!   • updates   (GUI -> MPRIS): track metadata and playback status.
//!
//! NOTE: this is the most API-drift-prone module in the app - mpris-server
//! wraps zbus and its builder/callback names move between versions. If the
//! build breaks here, `cargo doc -p mpris-server --open` shows the current
//! shape; the structure below stays valid.

use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Debug, Clone, Copy)]
pub enum MprisCmd {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Prev,
}

#[derive(Debug, Clone)]
pub enum MprisUpdate {
    Metadata {
        title: String,
        artist: String,
        album: String,
        length_secs: f64,
    },
    Playing,
    Paused,
    Stopped,
}

/// The GUI's subscription takes this receiver exactly once at startup.
pub static CMD_RX: OnceLock<Mutex<Option<UnboundedReceiver<MprisCmd>>>> = OnceLock::new();

/// Spawn the MPRIS thread. Returns the sender for state updates.
/// If D-Bus is unavailable the thread logs and exits; the app runs fine
/// without media-key support.
pub fn spawn() -> UnboundedSender<MprisUpdate> {
    let (cmd_tx, cmd_rx) = unbounded_channel::<MprisCmd>();
    let _ = CMD_RX.set(Mutex::new(Some(cmd_rx)));

    let (upd_tx, mut upd_rx) = unbounded_channel::<MprisUpdate>();

    let _ = thread::spawn_named(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("mpris: runtime: {e}");
                return;
            }
        };
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, async move {
            let player = match mpris_server::Player::builder(
                "io.github.ctsdownloads.CosmicCassetteDeck",
            )
            .identity("Cosmic Cassette Deck")
            .can_play(true)
            .can_pause(true)
            .can_go_next(true)
            .can_go_previous(true)
            .can_control(true)
            .can_seek(false)
            .build()
            .await
            {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("mpris: D-Bus unavailable, media keys disabled: {e}");
                    return;
                }
            };

            let t = cmd_tx.clone();
            player.connect_play_pause(move |_| {
                let _ = t.send(MprisCmd::PlayPause);
            });
            let t = cmd_tx.clone();
            player.connect_play(move |_| {
                let _ = t.send(MprisCmd::Play);
            });
            let t = cmd_tx.clone();
            player.connect_pause(move |_| {
                let _ = t.send(MprisCmd::Pause);
            });
            let t = cmd_tx.clone();
            player.connect_stop(move |_| {
                let _ = t.send(MprisCmd::Stop);
            });
            let t = cmd_tx.clone();
            player.connect_next(move |_| {
                let _ = t.send(MprisCmd::Next);
            });
            let t = cmd_tx.clone();
            player.connect_previous(move |_| {
                let _ = t.send(MprisCmd::Prev);
            });

            let run = player.run();
            tokio::pin!(run);

            loop {
                tokio::select! {
                    _ = &mut run => break,
                    upd = upd_rx.recv() => match upd {
                        None => break, // GUI gone
                        Some(u) => apply(&player, u).await,
                    }
                }
            }
        });
    });

    upd_tx
}

async fn apply(player: &mpris_server::Player, update: MprisUpdate) {
    use mpris_server::{Metadata, PlaybackStatus, Time};

    match update {
        MprisUpdate::Metadata {
            title,
            artist,
            album,
            length_secs,
        } => {
            let meta = Metadata::builder()
                .title(title)
                .artist([artist])
                .album(album)
                .length(Time::from_secs(length_secs as i64))
                .build();
            let _ = player.set_metadata(meta).await;
        }
        MprisUpdate::Playing => {
            let _ = player.set_playback_status(PlaybackStatus::Playing).await;
        }
        MprisUpdate::Paused => {
            let _ = player.set_playback_status(PlaybackStatus::Paused).await;
        }
        MprisUpdate::Stopped => {
            let _ = player.set_playback_status(PlaybackStatus::Stopped).await;
        }
    }
}

mod thread {
    pub fn spawn_named<F: FnOnce() + Send + 'static>(
        f: F,
    ) -> std::io::Result<std::thread::JoinHandle<()>> {
        std::thread::Builder::new().name("mpris".into()).spawn(f)
    }
}
