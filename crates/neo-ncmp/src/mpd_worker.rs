//! The MPD actor.
//!
//! All requests to MPD are serialised through one task, which owns the command
//! connection and reconnects it with backoff. A second, independent task owns
//! the `idle` connection and reports changes. The UI never awaits MPD.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use neo_ncmp_domain::{Library, MpdOutput, MpdStats, MpdStatus, Song};
use neo_ncmp_mpd::{DirEntry, MpdClient, MpdError, Picture, Playlist, Subsystem};

use crate::lyrics::{self, LyricsDoc};
use crate::stats::StatsReport;
use crate::tags::TagUpdate;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct MpdHandle {
    tx: mpsc::UnboundedSender<Command>,
}

impl MpdHandle {
    pub fn send(&self, command: Command) {
        if self.tx.send(command).is_err() {
            tracing::debug!("MPD worker has stopped; dropping command");
        }
    }

    #[cfg(test)]
    pub(crate) fn test_handle() -> Self {
        let (tx, _receiver) = mpsc::unbounded_channel();
        Self { tx }
    }

    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, mpsc::UnboundedReceiver<Command>) {
        let (tx, receiver) = mpsc::unbounded_channel();
        (Self { tx }, receiver)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerOption {
    Repeat,
    Random,
    Single,
    Consume,
}

#[derive(Debug)]
pub enum Command {
    RefreshAll,
    RefreshStatus,
    RefreshCurrent,
    RefreshQueue,
    RefreshOutputs,
    RefreshStats,
    LoadLibrary,

    Play,
    Pause(bool),
    Stop,
    Next,
    Previous,
    Seek(i64),
    SeekTo(u64),
    SetLoop {
        repeat: bool,
        single: bool,
    },
    SetVolume(u8),
    SetOption(PlayerOption, bool),
    PlayPos(u32),

    AppendUris(Vec<String>),
    AppendAndPlay(Vec<String>),
    ReplaceAndPlay(Vec<String>),
    RemoveId(u32),
    MoveId {
        id: u32,
        to: u32,
    },
    Clear,
    Shuffle,

    EnableOutput(u32),
    DisableOutput(u32),

    RefreshPlaylists,
    RefreshPlaylistSongs(String),
    LoadPlaylist {
        name: String,
        replace: bool,
    },
    AppendPlaylist(String),
    SaveQueueAs(String),
    DeletePlaylist(String),
    RenamePlaylist {
        from: String,
        to: String,
    },
    PlaylistRemoveSong {
        name: String,
        position: u32,
    },
    PlaylistMoveSong {
        name: String,
        from: u32,
        to: u32,
    },

    FetchCover(String),
    Browse(Option<String>),
    AppendUri(String),
    ReplaceWithUri(String),
    SaveTags {
        path: PathBuf,
        uri: String,
        update: TagUpdate,
    },
    FetchLyrics {
        uri: String,
        artist: String,
        title: String,
        album: String,
        duration_seconds: Option<u64>,
        music_directory: Option<PathBuf>,
    },
}

#[derive(Debug)]
pub enum Msg {
    Connected {
        version: String,
    },
    Disconnected {
        error: String,
    },
    Status(MpdStatus),
    CurrentSong(Option<Song>),
    Queue(Vec<Song>),
    Library(Arc<Library>),
    Outputs(Vec<MpdOutput>),
    Stats(MpdStats),
    Idle(Vec<Subsystem>),
    SearchResults {
        generation: u64,
        results: Vec<usize>,
    },
    Playlists(Vec<Playlist>),
    PlaylistSongs {
        name: String,
        songs: Vec<Song>,
    },
    Cover {
        uri: String,
        picture: Option<Picture>,
    },
    OnlineCover {
        uri: String,
        picture: Option<Picture>,
    },
    Ipc {
        line: String,
        reply: tokio::sync::oneshot::Sender<String>,
    },
    Lyrics {
        uri: String,
        doc: Option<LyricsDoc>,
    },
    Visualizer(Vec<f32>),
    VisualizerError(Option<String>),
    TagsSaved {
        uri: String,
        backup: Option<PathBuf>,
        error: Option<String>,
    },
    History(StatsReport),
    HistoryError(String),
    Browser {
        uri: Option<String>,
        entries: Vec<DirEntry>,
    },
    ConfigFileChanged,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub addr: String,
    pub password: Option<String>,
}

impl Endpoint {
    pub fn new(addr: String, password: Option<String>) -> Self {
        Self { addr, password }
    }
}

async fn connect(endpoint: &Endpoint) -> Result<MpdClient, MpdError> {
    let mut client = MpdClient::connect(&endpoint.addr).await?;
    if let Some(password) = &endpoint.password {
        client.password(password).await?;
    }
    Ok(client)
}

pub fn spawn(endpoint: Endpoint, tx: mpsc::UnboundedSender<Msg>) -> MpdHandle {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    tokio::spawn(worker(endpoint, tx, command_rx));
    MpdHandle { tx: command_tx }
}

/// Starts the independent `idle` connection.
pub fn spawn_idle(endpoint: Endpoint, tx: mpsc::UnboundedSender<Msg>) {
    tokio::spawn(async move {
        loop {
            match connect(&endpoint).await {
                Ok(mut client) => {
                    // Changes may have happened while this connection was
                    // down; ask the UI to resynchronise.
                    let _ = tx.send(Msg::Idle(vec![
                        Subsystem::Player,
                        Subsystem::Playlist,
                        Subsystem::Mixer,
                        Subsystem::Options,
                        Subsystem::Output,
                    ]));
                    loop {
                        match client.idle(&[]).await {
                            Ok(subsystems) => {
                                if tx.send(Msg::Idle(subsystems)).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                tracing::debug!(%error, "idle connection ended");
                                break;
                            }
                        }
                    }
                }
                Err(error) => tracing::debug!(%error, "could not open idle connection"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
}

async fn worker(
    endpoint: Endpoint,
    tx: mpsc::UnboundedSender<Msg>,
    mut commands: mpsc::UnboundedReceiver<Command>,
) {
    let mut client: Option<MpdClient> = None;

    loop {
        if client.is_none() {
            match connect(&endpoint).await {
                Ok(mut new_client) => {
                    tracing::info!(addr = %endpoint.addr, version = new_client.version(), "connected to MPD");
                    send(
                        &tx,
                        Msg::Connected {
                            version: new_client.version().to_owned(),
                        },
                    );
                    if let Err(error) = refresh_initial(&mut new_client, &tx, &endpoint).await {
                        send(&tx, Msg::Error(error.to_string()));
                        if error.is_connection_lost() {
                            send(
                                &tx,
                                Msg::Disconnected {
                                    error: error.to_string(),
                                },
                            );
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    }
                    client = Some(new_client);
                }
                Err(error) => {
                    tracing::warn!(%error, "could not connect to MPD");
                    send(
                        &tx,
                        Msg::Disconnected {
                            error: error.to_string(),
                        },
                    );
                    tokio::select! {
                        command = commands.recv() => {
                            if command.is_none() {
                                return;
                            }
                        }
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                    continue;
                }
            }
        }

        let Some(command) = commands.recv().await else {
            return;
        };
        let Some(active_client) = client.as_mut() else {
            continue;
        };

        match execute(active_client, command, &tx, &endpoint).await {
            Ok(()) => {}
            Err(error) if error.is_connection_lost() => {
                tracing::warn!(%error, "lost connection to MPD");
                client = None;
                send(
                    &tx,
                    Msg::Disconnected {
                        error: error.to_string(),
                    },
                );
            }
            Err(error) => {
                tracing::warn!(%error, "MPD command failed");
                send(&tx, Msg::Error(error.to_string()));
            }
        }
    }
}

async fn refresh_initial(
    client: &mut MpdClient,
    tx: &mpsc::UnboundedSender<Msg>,
    endpoint: &Endpoint,
) -> Result<(), MpdError> {
    let status = client.status().await?;
    send(tx, Msg::Status(status));
    let current = client.current_song().await?;
    send(tx, Msg::CurrentSong(current));
    let queue = client.queue().await?;
    send(tx, Msg::Queue(queue));
    let outputs = client.outputs().await?;
    send(tx, Msg::Outputs(outputs));
    let stats = client.stats().await?;
    send(tx, Msg::Stats(stats));
    spawn_library_load(endpoint.clone(), tx.clone());
    Ok(())
}

async fn execute(
    client: &mut MpdClient,
    command: Command,
    tx: &mpsc::UnboundedSender<Msg>,
    endpoint: &Endpoint,
) -> Result<(), MpdError> {
    use Command::*;

    match command {
        RefreshAll => refresh_initial(client, tx, endpoint).await?,
        RefreshStatus => refresh_status(client, tx).await?,
        RefreshCurrent => {
            let song = client.current_song().await?;
            send(tx, Msg::CurrentSong(song));
        }
        RefreshQueue => refresh_queue(client, tx).await?,
        RefreshOutputs => {
            let outputs = client.outputs().await?;
            send(tx, Msg::Outputs(outputs));
        }
        RefreshStats => {
            let stats = client.stats().await?;
            send(tx, Msg::Stats(stats));
        }
        LoadLibrary => spawn_library_load(endpoint.clone(), tx.clone()),

        Play => {
            client.play().await?;
            refresh_status_and_current(client, tx).await?;
        }
        Pause(paused) => {
            client.pause(paused).await?;
            refresh_status(client, tx).await?;
        }
        Stop => {
            client.stop().await?;
            refresh_status(client, tx).await?;
        }
        Next => {
            client.next().await?;
            refresh_status_and_current(client, tx).await?;
        }
        Previous => {
            client.previous().await?;
            refresh_status_and_current(client, tx).await?;
        }
        Seek(delta_ms) => {
            client
                .seek_current(&format!("{:+.3}", delta_ms as f64 / 1000.0))
                .await?;
            refresh_status(client, tx).await?;
        }
        SeekTo(position_ms) => {
            client
                .seek_current(&format!("{:.3}", position_ms as f64 / 1000.0))
                .await?;
            refresh_status(client, tx).await?;
        }
        SetLoop { repeat, single } => {
            client.set_repeat(repeat).await?;
            client.set_single(single).await?;
            refresh_status(client, tx).await?;
        }
        SetVolume(volume) => {
            client.set_volume(volume).await?;
            refresh_status(client, tx).await?;
        }
        SetOption(option, enabled) => {
            match option {
                PlayerOption::Repeat => client.set_repeat(enabled).await?,
                PlayerOption::Random => client.set_random(enabled).await?,
                PlayerOption::Single => client.set_single(enabled).await?,
                PlayerOption::Consume => client.set_consume(enabled).await?,
            }
            refresh_status(client, tx).await?;
        }
        PlayPos(position) => {
            client.play_pos(position).await?;
            refresh_status_and_current(client, tx).await?;
        }

        AppendUris(uris) => {
            for uri in uris {
                client.add(&uri).await?;
            }
            refresh_queue(client, tx).await?;
            refresh_status(client, tx).await?;
        }
        AppendAndPlay(uris) => {
            let Some(first_id) = add_all(client, uris).await? else {
                return Ok(());
            };
            client.play_id(first_id).await?;
            refresh_queue(client, tx).await?;
            refresh_status_and_current(client, tx).await?;
        }
        ReplaceAndPlay(uris) => {
            client.clear().await?;
            let Some(first_id) = add_all(client, uris).await? else {
                refresh_queue(client, tx).await?;
                return Ok(());
            };
            client.play_id(first_id).await?;
            refresh_queue(client, tx).await?;
            refresh_status_and_current(client, tx).await?;
        }
        RemoveId(id) => {
            client.delete_id(id).await?;
            refresh_queue(client, tx).await?;
            refresh_status(client, tx).await?;
        }
        MoveId { id, to } => {
            client.move_id(id, to).await?;
            refresh_queue(client, tx).await?;
        }
        Clear => {
            client.clear().await?;
            refresh_queue(client, tx).await?;
            refresh_status(client, tx).await?;
        }
        Shuffle => {
            client.shuffle().await?;
            refresh_queue(client, tx).await?;
        }

        EnableOutput(id) => {
            client.enable_output(id).await?;
            let outputs = client.outputs().await?;
            send(tx, Msg::Outputs(outputs));
        }
        DisableOutput(id) => {
            client.disable_output(id).await?;
            let outputs = client.outputs().await?;
            send(tx, Msg::Outputs(outputs));
        }

        RefreshPlaylists => {
            let playlists = client.playlists().await?;
            send(tx, Msg::Playlists(playlists));
        }
        RefreshPlaylistSongs(name) => {
            match client.playlist_info(&name).await {
                Ok(songs) => send(tx, Msg::PlaylistSongs { name, songs }),
                // The playlist was deleted while this request was in flight.
                Err(MpdError::Ack { code: 50, .. }) => {
                    send(
                        tx,
                        Msg::PlaylistSongs {
                            name,
                            songs: Vec::new(),
                        },
                    );
                }
                Err(error) => return Err(error),
            }
        }
        LoadPlaylist { name, replace } => {
            if replace {
                client.clear().await?;
            }
            client.load_playlist(&name).await?;
            if replace {
                let _ = client.play().await;
            }
            refresh_queue(client, tx).await?;
            refresh_status_and_current(client, tx).await?;
        }
        AppendPlaylist(name) => {
            client.load_playlist(&name).await?;
            refresh_queue(client, tx).await?;
            refresh_status(client, tx).await?;
        }
        SaveQueueAs(name) => {
            client.save_playlist(&name).await?;
            let playlists = client.playlists().await?;
            send(tx, Msg::Playlists(playlists));
        }
        DeletePlaylist(name) => {
            client.remove_playlist(&name).await?;
            let playlists = client.playlists().await?;
            send(tx, Msg::Playlists(playlists));
        }
        RenamePlaylist { from, to } => {
            client.rename_playlist(&from, &to).await?;
            let playlists = client.playlists().await?;
            send(tx, Msg::Playlists(playlists));
        }
        PlaylistRemoveSong { name, position } => {
            client.playlist_delete(&name, position).await?;
            let songs = client.playlist_info(&name).await?;
            send(tx, Msg::PlaylistSongs { name, songs });
        }
        PlaylistMoveSong { name, from, to } => {
            client.playlist_move(&name, from, to).await?;
            let songs = client.playlist_info(&name).await?;
            send(tx, Msg::PlaylistSongs { name, songs });
        }

        SaveTags { path, uri, update } => {
            let endpoint = endpoint.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let result =
                    tokio::task::spawn_blocking(move || crate::tags::save(&path, &update)).await;
                match result {
                    Ok(Ok(backup)) => {
                        if let Ok(mut client) = connect(&endpoint).await {
                            let _ = client.update(Some(&uri)).await;
                        }
                        send(
                            &tx,
                            Msg::TagsSaved {
                                uri,
                                backup: Some(backup),
                                error: None,
                            },
                        );
                    }
                    Ok(Err(error)) => send(
                        &tx,
                        Msg::TagsSaved {
                            uri,
                            backup: None,
                            error: Some(error),
                        },
                    ),
                    Err(error) => send(
                        &tx,
                        Msg::TagsSaved {
                            uri,
                            backup: None,
                            error: Some(error.to_string()),
                        },
                    ),
                }
            });
        }

        FetchLyrics {
            uri,
            artist,
            title,
            album,
            duration_seconds,
            music_directory,
        } => {
            let tx = tx.clone();
            tokio::spawn(async move {
                let doc = lyrics::fetch(lyrics::LyricsRequest {
                    uri: &uri,
                    artist: &artist,
                    title: &title,
                    album: &album,
                    duration_seconds,
                    music_directory: music_directory.as_deref(),
                })
                .await;
                send(&tx, Msg::Lyrics { uri, doc });
            });
        }

        Browse(uri) => {
            let entries = client.list_entries(uri.as_deref()).await?;
            send(tx, Msg::Browser { uri, entries });
        }
        AppendUri(uri) => {
            client.add(&uri).await?;
            refresh_queue(client, tx).await?;
            refresh_status(client, tx).await?;
        }
        ReplaceWithUri(uri) => {
            client.clear().await?;
            client.add(&uri).await?;
            let _ = client.play().await;
            refresh_queue(client, tx).await?;
            refresh_status_and_current(client, tx).await?;
        }

        FetchCover(uri) => {
            // Artwork is fetched on its own connection so a slow read can
            // never delay transport commands.
            let endpoint = endpoint.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut client = match connect(&endpoint).await {
                    Ok(client) => client,
                    Err(error) => {
                        tracing::debug!(%error, "cover connection failed");
                        send(&tx, Msg::Cover { uri, picture: None });
                        return;
                    }
                };
                let picture = match client.read_picture(&uri).await {
                    Ok(Some(picture)) => Some(picture),
                    Ok(None) => match client.album_art(&uri).await {
                        Ok(picture) => picture,
                        Err(error) => {
                            tracing::debug!(%error, "albumart failed");
                            None
                        }
                    },
                    Err(error) => {
                        tracing::debug!(%error, "readpicture failed");
                        None
                    }
                };
                send(&tx, Msg::Cover { uri, picture });
            });
        }
    }
    Ok(())
}

async fn add_all(client: &mut MpdClient, uris: Vec<String>) -> Result<Option<u32>, MpdError> {
    let mut first_id = None;
    for uri in uris {
        let id = client.add_id(&uri, None).await?;
        first_id.get_or_insert(id);
    }
    Ok(first_id)
}

async fn refresh_status(
    client: &mut MpdClient,
    tx: &mpsc::UnboundedSender<Msg>,
) -> Result<(), MpdError> {
    let status = client.status().await?;
    send(tx, Msg::Status(status));
    Ok(())
}

async fn refresh_queue(
    client: &mut MpdClient,
    tx: &mpsc::UnboundedSender<Msg>,
) -> Result<(), MpdError> {
    let queue = client.queue().await?;
    send(tx, Msg::Queue(queue));
    Ok(())
}

async fn refresh_status_and_current(
    client: &mut MpdClient,
    tx: &mpsc::UnboundedSender<Msg>,
) -> Result<(), MpdError> {
    refresh_status(client, tx).await?;
    let song = client.current_song().await?;
    send(tx, Msg::CurrentSong(song));
    Ok(())
}

fn spawn_library_load(endpoint: Endpoint, tx: mpsc::UnboundedSender<Msg>) {
    tokio::spawn(async move {
        let mut client = match connect(&endpoint).await {
            Ok(client) => client,
            Err(error) => {
                send(&tx, Msg::Error(format!("library: {error}")));
                return;
            }
        };

        let songs = match client.list_all_info().await {
            Ok(songs) => songs,
            Err(error) => {
                send(&tx, Msg::Error(format!("library: {error}")));
                return;
            }
        };

        match tokio::task::spawn_blocking(move || Library::build(songs)).await {
            Ok(library) => {
                send(&tx, Msg::Library(Arc::new(library)));
            }
            Err(error) => send(&tx, Msg::Error(format!("library index: {error}"))),
        }
    });
}

fn send(tx: &mpsc::UnboundedSender<Msg>, message: Msg) {
    let _ = tx.send(message);
}
