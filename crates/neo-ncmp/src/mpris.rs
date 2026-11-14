//! MPRIS integration.
//!
//! The `mpris-server` ready-made player is `!Send`, so it runs on its own
//! current-thread runtime. The app publishes immutable snapshots through a
//! watch channel; callbacks send intentions to the regular MPD worker.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time};
use neo_ncmp_domain::{PlaybackState, Song};
use tokio::sync::watch;

use crate::mpd_worker::{Command, MpdHandle, PlayerOption};

#[derive(Debug, Clone, Default)]
pub struct MprisSnapshot {
    pub playback: PlaybackState,
    pub volume: Option<u8>,
    pub current: Option<Song>,
    pub position: Duration,
    pub repeat: bool,
    pub random: bool,
    pub single: bool,
}

pub fn spawn(mpd: MpdHandle, snapshots: watch::Receiver<MprisSnapshot>) {
    let result = std::thread::Builder::new()
        .name("neo-ncmp-mpris".into())
        .spawn(move || {
            tracing::info!("starting MPRIS thread");
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::warn!(%error, "could not create MPRIS runtime");
                    return;
                }
            };
            runtime.block_on(async {
                let local = tokio::task::LocalSet::new();
                local.run_until(run(mpd, snapshots)).await;
            });
        });
    if let Err(error) = result {
        tracing::warn!(%error, "could not start MPRIS thread");
    }
}

async fn run(mpd: MpdHandle, snapshots: watch::Receiver<MprisSnapshot>) {
    tracing::info!("building MPRIS player");
    let player = match Player::builder("neo-ncmp")
        .identity("neo-ncmp")
        .desktop_entry("neo-ncmp")
        .can_play(true)
        .can_pause(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_seek(true)
        .can_control(true)
        .build()
        .await
    {
        Ok(player) => {
            tracing::info!("MPRIS player ready");
            player
        }
        Err(error) => {
            // No D-Bus session (or no permission): the TUI still works.
            tracing::warn!(%error, "MPRIS server unavailable");
            return;
        }
    };

    let playing = Arc::new(AtomicBool::new(false));

    let handle = mpd.clone();
    player.connect_play(move |_| handle.send(Command::Play));
    let handle = mpd.clone();
    player.connect_pause(move |_| handle.send(Command::Pause(true)));
    let handle = mpd.clone();
    let is_playing = playing.clone();
    player.connect_play_pause(move |_| {
        if is_playing.load(Ordering::Relaxed) {
            handle.send(Command::Pause(true));
        } else {
            handle.send(Command::Play);
        }
    });
    let handle = mpd.clone();
    player.connect_stop(move |_| handle.send(Command::Stop));
    let handle = mpd.clone();
    player.connect_next(move |_| handle.send(Command::Next));
    let handle = mpd.clone();
    player.connect_previous(move |_| handle.send(Command::Previous));
    let handle = mpd.clone();
    player.connect_seek(move |_, offset| {
        handle.send(Command::Seek(offset.as_micros() / 1000));
    });
    let handle = mpd.clone();
    player.connect_set_position(move |_, _, position| {
        handle.send(Command::SeekTo(position.as_micros().max(0) as u64 / 1000));
    });
    let handle = mpd.clone();
    player.connect_set_volume(move |_, volume| {
        let volume = (volume * 100.0).round().clamp(0.0, 100.0) as u8;
        handle.send(Command::SetVolume(volume));
    });
    let handle = mpd.clone();
    player.connect_set_loop_status(move |_, status| {
        let (repeat, single) = match status {
            LoopStatus::None => (false, false),
            LoopStatus::Track => (false, true),
            LoopStatus::Playlist => (true, false),
        };
        handle.send(Command::SetLoop { repeat, single });
    });
    let handle = mpd.clone();
    player.connect_set_shuffle(move |_, shuffle| {
        handle.send(Command::SetOption(PlayerOption::Random, shuffle));
    });
    let handle = mpd.clone();
    player.connect_open_uri(move |_, uri| {
        handle.send(Command::AppendAndPlay(vec![uri.to_owned()]));
    });

    let playing_for_watch = playing.clone();
    let player = std::rc::Rc::new(player);
    let server_player = player.clone();
    let watch_player = player.clone();
    tokio::join!(
        async move {
            tracing::info!("MPRIS server task starting");
            let _ = server_player.run().await;
            tracing::info!("MPRIS server task ended");
        },
        watch_snapshots(watch_player, snapshots, playing_for_watch),
    );
}

async fn watch_snapshots(
    player: std::rc::Rc<Player>,
    mut snapshots: watch::Receiver<MprisSnapshot>,
    playing: Arc<AtomicBool>,
) {
    let mut previous = MprisSnapshot::default();
    let initial = snapshots.borrow().clone();
    playing.store(initial.playback.is_playing(), Ordering::Relaxed);
    apply_snapshot(&player, &previous, &initial).await;
    previous = initial;

    while snapshots.changed().await.is_ok() {
        let snapshot = snapshots.borrow_and_update().clone();
        playing.store(snapshot.playback.is_playing(), Ordering::Relaxed);
        apply_snapshot(&player, &previous, &snapshot).await;
        previous = snapshot;
    }
}

async fn apply_snapshot(player: &Player, previous: &MprisSnapshot, snapshot: &MprisSnapshot) {
    if previous.playback != snapshot.playback {
        let status = match snapshot.playback {
            PlaybackState::Playing => PlaybackStatus::Playing,
            PlaybackState::Paused => PlaybackStatus::Paused,
            PlaybackState::Stopped => PlaybackStatus::Stopped,
        };
        let _ = player.set_playback_status(status).await;
    }

    let loop_status = if snapshot.single {
        LoopStatus::Track
    } else if snapshot.repeat {
        LoopStatus::Playlist
    } else {
        LoopStatus::None
    };
    if (previous.repeat, previous.single) != (snapshot.repeat, snapshot.single) {
        let _ = player.set_loop_status(loop_status).await;
    }

    if previous.random != snapshot.random {
        let _ = player.set_shuffle(snapshot.random).await;
    }

    if previous.volume != snapshot.volume {
        let volume = snapshot
            .volume
            .map(|volume| volume as f64 / 100.0)
            .unwrap_or(1.0);
        let _ = player.set_volume(volume).await;
    }

    let current_changed = match (&previous.current, &snapshot.current) {
        (Some(a), Some(b)) => a.uri != b.uri || a.id != b.id,
        (None, None) => false,
        _ => true,
    };
    if current_changed {
        let metadata = metadata_for(snapshot.current.as_ref());
        let _ = player.set_metadata(metadata).await;
    }

    // `set_position` is deliberately synchronous and does not emit a property
    // change; the app publishes a snapshot on every tick while playing.
    player.set_position(Time::from_micros(
        snapshot.position.as_micros().min(i64::MAX as u128) as i64,
    ));
}

fn metadata_for(song: Option<&Song>) -> Metadata {
    let Some(song) = song else {
        return Metadata::new();
    };

    let mut builder = Metadata::builder()
        .title(song.display_title())
        .artist([song.display_artist()])
        .album(song.display_album());
    if let Some(album_artist) = &song.album_artist {
        builder = builder.album_artist([album_artist.to_string()]);
    }
    if let Some(duration) = song.duration {
        builder = builder.length(Time::from_micros(
            duration.as_micros().min(i64::MAX as u128) as i64,
        ));
    }
    if let Some(track) = song.track.and_then(|track| i32::try_from(track).ok()) {
        builder = builder.track_number(track);
    }
    if let Some(disc) = song.disc.and_then(|disc| i32::try_from(disc).ok()) {
        builder = builder.disc_number(disc);
    }
    if let Some(genre) = &song.genre {
        builder = builder.genre([genre.to_string()]);
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a D-Bus session"]
    async fn registers_an_mpris_name() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let player = Player::builder("neo-ncmp-probe")
                    .identity("neo-ncmp-probe")
                    .build()
                    .await
                    .expect("MPRIS build");
                player
                    .set_metadata(Metadata::builder().title("probe").build())
                    .await
                    .expect("metadata");
                let run = player.run();
                tokio::select! {
                    _ = run => {}
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
            })
            .await;
    }
}
