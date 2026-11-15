//! MPRIS integration.
//!
//! A local `mpris-server` implementation exposes the player, a queue window as
//! the TrackList interface, and MPD playlists. It is `!Send`, so it runs on its
//! own current-thread runtime. The app publishes immutable snapshots through a
//! watch channel; interface methods send intentions to the MPD worker.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use ascolta_domain::{PlaybackState, Song};
use mpris_server::zbus::{Result as ZbusResult, fdo};
use mpris_server::{
    LocalPlayerInterface, LocalPlaylistsInterface, LocalRootInterface, LocalServer,
    LocalTrackListInterface, LoopStatus, Metadata, PlaybackRate, PlaybackStatus, Playlist,
    PlaylistId, PlaylistOrdering, PlaylistsProperty, Property, Time, TrackId, TrackListProperty,
    TrackListSignal, Volume,
};
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
    /// Current queue position, which is more reliable than the song's cached field.
    pub song_pos: Option<u32>,
    /// Queue snapshot, shared cheaply so TrackList can expose a window.
    pub queue: Arc<Vec<Song>>,
    /// Stored playlists as `(name, last_modified)`.
    pub playlists: Arc<Vec<(String, Option<String>)>>,
}

pub fn spawn(mpd: MpdHandle, snapshots: watch::Receiver<MprisSnapshot>) {
    let result = std::thread::Builder::new()
        .name("ascolta-mpris".into())
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

#[derive(Debug, Clone)]
struct MprisTrack {
    position: u32,
    song: Song,
    id: TrackId,
}

#[derive(Debug, Clone)]
struct MprisPlaylist {
    name: String,
    modified: Option<String>,
    id: PlaylistId,
}

struct MprisState {
    mpd: MpdHandle,
    playback_status: Cell<PlaybackStatus>,
    loop_status: Cell<LoopStatus>,
    shuffle: Cell<bool>,
    metadata: RefCell<Metadata>,
    volume: Cell<Volume>,
    position: Cell<Time>,
    tracks: RefCell<Vec<MprisTrack>>,
    playlists: RefCell<Vec<MprisPlaylist>>,
}

impl MprisState {
    fn new(mpd: MpdHandle) -> Self {
        Self {
            mpd,
            playback_status: Cell::new(PlaybackStatus::Stopped),
            loop_status: Cell::new(LoopStatus::None),
            shuffle: Cell::new(false),
            metadata: RefCell::new(Metadata::new()),
            volume: Cell::new(1.0),
            position: Cell::new(Time::ZERO),
            tracks: RefCell::new(Vec::new()),
            playlists: RefCell::new(Vec::new()),
        }
    }
}

impl LocalRootInterface for MprisState {
    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }

    async fn quit(&self) -> fdo::Result<()> {
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> ZbusResult<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok("ascolta".into())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("ascolta".into())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec!["file".into()])
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

impl LocalPlayerInterface for MprisState {
    async fn next(&self) -> fdo::Result<()> {
        self.mpd.send(Command::Next);
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.mpd.send(Command::Previous);
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.mpd.send(Command::Pause(true));
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        if self.playback_status.get() == PlaybackStatus::Playing {
            self.mpd.send(Command::Pause(true));
        } else {
            self.mpd.send(Command::Play);
        }
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.mpd.send(Command::Stop);
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        self.mpd.send(Command::Play);
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.mpd.send(Command::Seek(offset.as_micros() / 1000));
        Ok(())
    }

    async fn set_position(&self, _track_id: TrackId, position: Time) -> fdo::Result<()> {
        self.position.set(position);
        self.mpd
            .send(Command::SeekTo(position.as_micros().max(0) as u64 / 1000));
        Ok(())
    }

    async fn open_uri(&self, uri: String) -> fdo::Result<()> {
        self.mpd.send(Command::AppendAndPlay(vec![uri]));
        Ok(())
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self.playback_status.get())
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(self.loop_status.get())
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> ZbusResult<()> {
        self.loop_status.set(loop_status);
        let (repeat, single) = match loop_status {
            LoopStatus::None => (false, false),
            LoopStatus::Track => (false, true),
            LoopStatus::Playlist => (true, false),
        };
        self.mpd.send(Command::SetLoop { repeat, single });
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn set_rate(&self, _rate: PlaybackRate) -> ZbusResult<()> {
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.shuffle.get())
    }

    async fn set_shuffle(&self, shuffle: bool) -> ZbusResult<()> {
        self.shuffle.set(shuffle);
        self.mpd
            .send(Command::SetOption(PlayerOption::Random, shuffle));
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self.metadata.borrow().clone())
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.volume.get())
    }

    async fn set_volume(&self, volume: Volume) -> ZbusResult<()> {
        self.volume.set(volume);
        let value = (volume * 100.0).round().clamp(0.0, 100.0) as u8;
        self.mpd.send(Command::SetVolume(value));
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        Ok(self.position.get())
    }

    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

impl LocalTrackListInterface for MprisState {
    async fn get_tracks_metadata(&self, track_ids: Vec<TrackId>) -> fdo::Result<Vec<Metadata>> {
        let tracks = self.tracks.borrow();
        Ok(track_ids
            .into_iter()
            .map(|id| {
                let song = position_from_track_id(&id)
                    .and_then(|position| tracks.iter().find(|track| track.position == position))
                    .map(|track| &track.song);
                metadata_for(song, id)
            })
            .collect())
    }

    async fn add_track(
        &self,
        _uri: String,
        _after_track: TrackId,
        _set_as_current: bool,
    ) -> fdo::Result<()> {
        Ok(())
    }

    async fn remove_track(&self, _track_id: TrackId) -> fdo::Result<()> {
        Ok(())
    }

    async fn go_to(&self, track_id: TrackId) -> fdo::Result<()> {
        let Some(position) = position_from_track_id(&track_id) else {
            return Err(fdo::Error::InvalidArgs("unknown track id".into()));
        };
        self.mpd.send(Command::PlayPos(position));
        Ok(())
    }

    async fn tracks(&self) -> fdo::Result<Vec<TrackId>> {
        Ok(self
            .tracks
            .borrow()
            .iter()
            .map(|track| track.id.clone())
            .collect())
    }

    async fn can_edit_tracks(&self) -> fdo::Result<bool> {
        Ok(false)
    }
}

impl LocalPlaylistsInterface for MprisState {
    async fn activate_playlist(&self, playlist_id: PlaylistId) -> fdo::Result<()> {
        let name = self
            .playlists
            .borrow()
            .iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.name.clone())
            .ok_or_else(|| fdo::Error::InvalidArgs("unknown playlist id".into()))?;
        self.mpd.send(Command::LoadPlaylist {
            name,
            replace: true,
        });
        self.mpd.send(Command::Play);
        Ok(())
    }

    async fn get_playlists(
        &self,
        index: u32,
        max_count: u32,
        order: PlaylistOrdering,
        reverse_order: bool,
    ) -> fdo::Result<Vec<Playlist>> {
        let mut playlists = self.playlists.borrow().clone();
        match order {
            PlaylistOrdering::Alphabetical => {
                playlists.sort_by_key(|playlist| playlist.name.to_lowercase());
            }
            PlaylistOrdering::ModifiedDate => playlists.sort_by(|left, right| {
                left.modified
                    .cmp(&right.modified)
                    .then_with(|| left.name.cmp(&right.name))
            }),
            PlaylistOrdering::CreationDate
            | PlaylistOrdering::LastPlayDate
            | PlaylistOrdering::UserDefined => {}
        }
        if reverse_order {
            playlists.reverse();
        }
        let limit = if max_count == 0 {
            usize::MAX
        } else {
            max_count as usize
        };
        Ok(playlists
            .into_iter()
            .skip(index as usize)
            .take(limit)
            .map(|playlist| Playlist {
                id: playlist.id,
                name: playlist.name,
                icon: String::new(),
            })
            .collect())
    }

    async fn playlist_count(&self) -> fdo::Result<u32> {
        Ok(self.playlists.borrow().len() as u32)
    }

    async fn orderings(&self) -> fdo::Result<Vec<PlaylistOrdering>> {
        Ok(vec![
            PlaylistOrdering::Alphabetical,
            PlaylistOrdering::ModifiedDate,
            PlaylistOrdering::UserDefined,
        ])
    }

    async fn active_playlist(&self) -> fdo::Result<Option<Playlist>> {
        Ok(None)
    }
}

struct MprisPlayer {
    server: Rc<LocalServer<MprisState>>,
}

impl MprisPlayer {
    fn state(&self) -> &MprisState {
        self.server.imp()
    }

    async fn update_playback_status(&self, status: PlaybackStatus) {
        self.state().playback_status.set(status);
        let _ = self
            .server
            .properties_changed([Property::PlaybackStatus(status)])
            .await;
    }

    async fn update_loop_status(&self, status: LoopStatus) {
        self.state().loop_status.set(status);
        let _ = self
            .server
            .properties_changed([Property::LoopStatus(status)])
            .await;
    }

    async fn update_shuffle(&self, shuffle: bool) {
        self.state().shuffle.set(shuffle);
        let _ = self
            .server
            .properties_changed([Property::Shuffle(shuffle)])
            .await;
    }

    async fn update_volume(&self, volume: Volume) {
        self.state().volume.set(volume);
        let _ = self
            .server
            .properties_changed([Property::Volume(volume)])
            .await;
    }

    async fn update_metadata(&self, metadata: Metadata) {
        *self.state().metadata.borrow_mut() = metadata.clone();
        let _ = self
            .server
            .properties_changed([Property::Metadata(metadata)])
            .await;
    }

    async fn update_tracks(&self, snapshot: &MprisSnapshot) {
        let tracks = track_window(snapshot.queue.as_ref(), snapshot.song_pos);
        let changed = tracks_changed(&self.state().tracks.borrow(), &tracks);
        if !changed {
            return;
        }
        let ids: Vec<TrackId> = tracks.iter().map(|track| track.id.clone()).collect();
        let current_track = current_track_id(snapshot.song_pos);
        *self.state().tracks.borrow_mut() = tracks;
        let _ = self
            .server
            .track_list_properties_changed([TrackListProperty::Tracks])
            .await;
        let _ = self
            .server
            .track_list_emit(TrackListSignal::TrackListReplaced {
                tracks: ids,
                current_track,
            })
            .await;
    }

    async fn update_playlists(&self, snapshot: &MprisSnapshot) {
        let playlists: Vec<MprisPlaylist> = snapshot
            .playlists
            .iter()
            .map(|(name, modified)| MprisPlaylist {
                name: name.clone(),
                modified: modified.clone(),
                id: playlist_id(name),
            })
            .collect();
        let changed = {
            let current = self.state().playlists.borrow();
            current.len() != playlists.len()
                || current
                    .iter()
                    .zip(playlists.iter())
                    .any(|(left, right)| left.name != right.name)
        };
        if !changed {
            return;
        }
        let count = playlists.len() as u32;
        *self.state().playlists.borrow_mut() = playlists;
        let _ = self
            .server
            .playlists_properties_changed([PlaylistsProperty::PlaylistCount(count)])
            .await;
    }

    async fn apply_snapshot(&self, previous: &MprisSnapshot, snapshot: &MprisSnapshot) {
        if previous.playback != snapshot.playback {
            let status = match snapshot.playback {
                PlaybackState::Playing => PlaybackStatus::Playing,
                PlaybackState::Paused => PlaybackStatus::Paused,
                PlaybackState::Stopped => PlaybackStatus::Stopped,
            };
            self.update_playback_status(status).await;
        }

        let loop_status = if snapshot.single {
            LoopStatus::Track
        } else if snapshot.repeat {
            LoopStatus::Playlist
        } else {
            LoopStatus::None
        };
        if (previous.repeat, previous.single) != (snapshot.repeat, snapshot.single) {
            self.update_loop_status(loop_status).await;
        }

        if previous.random != snapshot.random {
            self.update_shuffle(snapshot.random).await;
        }

        if previous.volume != snapshot.volume {
            let volume = snapshot
                .volume
                .map(|volume| volume as f64 / 100.0)
                .unwrap_or(1.0);
            self.update_volume(volume).await;
        }

        let current_changed = previous.song_pos != snapshot.song_pos
            || match (&previous.current, &snapshot.current) {
                (Some(a), Some(b)) => a.uri != b.uri || a.id != b.id,
                (None, None) => false,
                _ => true,
            };
        if current_changed {
            let metadata = metadata_for(
                snapshot.current.as_ref(),
                current_track_id(snapshot.song_pos),
            );
            self.update_metadata(metadata).await;
        }

        self.update_tracks(snapshot).await;
        self.update_playlists(snapshot).await;

        // `Position` deliberately has no change signal; the app publishes a
        // snapshot on every tick while playing.
        self.state().position.set(Time::from_micros(
            snapshot.position.as_micros().min(i64::MAX as u128) as i64,
        ));
    }
}

async fn run(mpd: MpdHandle, snapshots: watch::Receiver<MprisSnapshot>) {
    tracing::info!("building MPRIS player");
    let state = MprisState::new(mpd);
    let server = match LocalServer::new_with_all("ascolta", state).await {
        Ok(server) => Rc::new(server),
        Err(error) => {
            // No D-Bus session (or no permission): the TUI still works.
            tracing::warn!(%error, "MPRIS server unavailable");
            return;
        }
    };
    tracing::info!("MPRIS player ready");
    let player = MprisPlayer {
        server: server.clone(),
    };
    let run_task = server.run();
    tokio::join!(
        async move {
            tracing::info!("MPRIS server task starting");
            let _ = run_task.await;
            tracing::info!("MPRIS server task ended");
        },
        watch_snapshots(player, snapshots),
    );
}

async fn watch_snapshots(player: MprisPlayer, mut snapshots: watch::Receiver<MprisSnapshot>) {
    let mut previous = MprisSnapshot::default();
    let initial = snapshots.borrow().clone();
    player.apply_snapshot(&previous, &initial).await;
    previous = initial;

    while snapshots.changed().await.is_ok() {
        let snapshot = snapshots.borrow_and_update().clone();
        player.apply_snapshot(&previous, &snapshot).await;
        previous = snapshot;
    }
}

fn track_id(position: u32) -> TrackId {
    TrackId::try_from(format!("/org/ascolta/Track/{position}"))
        .expect("generated track ids are valid object paths")
}

fn position_from_track_id(id: &TrackId) -> Option<u32> {
    id.as_str()
        .strip_prefix("/org/ascolta/Track/")?
        .parse()
        .ok()
}

fn current_track_id(song_pos: Option<u32>) -> TrackId {
    song_pos.map(track_id).unwrap_or(TrackId::NO_TRACK)
}

/// Stable across playlist reordering, derived from the name.
fn playlist_id(name: &str) -> PlaylistId {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    TrackId::try_from(format!("/org/ascolta/Playlist/{hash:016x}"))
        .expect("generated playlist ids are valid object paths")
        .into_inner()
        .into()
}

fn tracks_changed(current: &[MprisTrack], next: &[MprisTrack]) -> bool {
    current.len() != next.len()
        || current
            .iter()
            .zip(next)
            .any(|(left, right)| left.id != right.id || left.song.uri != right.song.uri)
}

/// Exposes at most 21 tracks around the current song, as the MPRIS spec
/// recommends for large queues.
fn track_window(queue: &[Song], current_position: Option<u32>) -> Vec<MprisTrack> {
    let (start, end) = match current_position {
        Some(position) => (
            position.saturating_sub(10) as usize,
            (position as usize + 11).min(queue.len()),
        ),
        None => (0, queue.len().min(21)),
    };
    if start >= end {
        return Vec::new();
    }
    queue[start..end]
        .iter()
        .enumerate()
        .map(|(offset, song)| {
            let position = (start + offset) as u32;
            MprisTrack {
                position,
                song: song.clone(),
                id: track_id(position),
            }
        })
        .collect()
}

fn metadata_for(song: Option<&Song>, track_id: TrackId) -> Metadata {
    let Some(song) = song else {
        return Metadata::builder().trackid(track_id).build();
    };

    let mut builder = Metadata::builder()
        .trackid(track_id)
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

    fn song(position: u32, title: &str) -> Song {
        Song {
            pos: Some(position),
            uri: Arc::from(format!("a/{position}.flac")),
            title: Some(Arc::from(title)),
            artist: Some(Arc::from("Artist")),
            album: Some(Arc::from("Album")),
            ..Song::default()
        }
    }

    #[test]
    fn track_window_follows_the_current_song() {
        let queue: Vec<Song> = (0..100)
            .map(|position| song(position, &format!("Track {position}")))
            .collect();
        let window = track_window(&queue, Some(50));
        assert_eq!(window.len(), 21);
        assert_eq!(window[0].position, 40);
        assert_eq!(window[20].position, 60);
        assert_eq!(position_from_track_id(&window[0].id), Some(40));

        let empty = track_window(&queue, None);
        assert_eq!(empty.len(), 21);
        assert_eq!(empty[0].position, 0);
    }

    #[test]
    fn track_windows_detect_insertions_at_the_same_ids() {
        let before = track_window(&[song(0, "A"), song(1, "B")], None);
        let after = track_window(&[song(0, "A"), song(1, "Inserted"), song(2, "B")], None);
        assert!(tracks_changed(&before, &after));
        assert!(!tracks_changed(&before, &before));
    }

    #[tokio::test]
    async fn playlists_are_sorted_paged_and_reversible() {
        let (mpd, _rx) = MpdHandle::test_channel();
        let state = MprisState::new(mpd);
        *state.playlists.borrow_mut() = vec![
            MprisPlaylist {
                name: "Beta".into(),
                modified: Some("2024".into()),
                id: playlist_id("Beta"),
            },
            MprisPlaylist {
                name: "Alpha".into(),
                modified: Some("2023".into()),
                id: playlist_id("Alpha"),
            },
        ];

        let alphabetical = state
            .get_playlists(0, 10, PlaylistOrdering::Alphabetical, false)
            .await
            .unwrap();
        assert_eq!(
            alphabetical
                .iter()
                .map(|playlist| playlist.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha", "Beta"]
        );

        let reversed = state
            .get_playlists(0, 10, PlaylistOrdering::Alphabetical, true)
            .await
            .unwrap();
        assert_eq!(reversed[0].name, "Beta");

        let paged = state
            .get_playlists(1, 1, PlaylistOrdering::Alphabetical, false)
            .await
            .unwrap();
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].name, "Beta");

        let modified = state
            .get_playlists(0, 10, PlaylistOrdering::ModifiedDate, false)
            .await
            .unwrap();
        assert_eq!(modified[0].name, "Alpha");
        assert_eq!(state.playlist_count().await.unwrap(), 2);
    }

    #[test]
    fn playlist_ids_are_stable_and_name_scoped() {
        assert_eq!(playlist_id("Late Night"), playlist_id("Late Night"));
        assert_ne!(playlist_id("Late Night"), playlist_id("Morning"));
        assert_ne!(
            playlist_id("Late Night"),
            TrackId::try_from("/other/path")
                .unwrap()
                .into_inner()
                .into()
        );
    }

    #[test]
    fn metadata_carries_the_track_id() {
        let track = song(7, "Seventh");
        let id = track_id(7);
        let metadata = metadata_for(Some(&track), id.clone());
        assert_eq!(metadata.trackid(), Some(id));
        assert_eq!(metadata.title(), Some("Seventh"));

        let empty = metadata_for(None, TrackId::NO_TRACK);
        assert_eq!(empty.trackid(), Some(TrackId::NO_TRACK));
    }

    #[tokio::test]
    #[ignore = "requires a D-Bus session"]
    async fn registers_all_mpris_interfaces() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (mpd, _rx) = MpdHandle::test_channel();
                let server = LocalServer::new_with_all("ascolta-probe", MprisState::new(mpd))
                    .await
                    .expect("MPRIS build");
                let run = server.run();
                tokio::select! {
                    _ = run => {}
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
            })
            .await;
    }
}
