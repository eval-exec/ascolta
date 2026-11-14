use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use neo_ncmp_domain::{
    Album, Artist, DirEntry, EntryKind, Library, MpdOutput, MpdStats, MpdStatus, PlaybackState,
    Song,
};
use neo_ncmp_mpd::{Picture, Playlist, Subsystem};
use ratatui::DefaultTerminal;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use tokio::sync::{mpsc, watch};

use crate::config::Config;
use crate::coverart;
use crate::input::{Action, Keymap};
use crate::lyrics::LyricsDoc;
use crate::mpd_worker::{self, Command, MpdHandle, Msg, PlayerOption};
use crate::mpris::{self, MprisSnapshot};
use crate::palette::{PaletteAction, PaletteState};
use crate::scrobbler::{
    self, ScrobbleEvent, ScrobbleHandle, ScrobbleKind, ScrobbleTrack, now_unix, scrobble_threshold,
};
use crate::search::{SearchHandle, SearchIndex, SearchState};
use crate::session::SessionState;
use crate::stats::{self, StatsReport};
use crate::tags::TagUpdate;
use crate::ui;
use crate::visualizer;
use crate::widgets::ListView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Queue,
    Library,
    Playlists,
    Outputs,
    Stats,
    Lyrics,
    Visualizer,
    TagEditor,
    History,
    Browser,
}

impl Screen {
    pub const ALL: [Self; 10] = [
        Self::Queue,
        Self::Library,
        Self::Playlists,
        Self::Outputs,
        Self::Stats,
        Self::Lyrics,
        Self::Visualizer,
        Self::TagEditor,
        Self::History,
        Self::Browser,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Queue => 0,
            Self::Library => 1,
            Self::Playlists => 2,
            Self::Outputs => 3,
            Self::Stats => 4,
            Self::Lyrics => 5,
            Self::Visualizer => 6,
            Self::TagEditor => 7,
            Self::History => 8,
            Self::Browser => 9,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Queue => "Queue",
            Self::Library => "Library",
            Self::Playlists => "Playlists",
            Self::Outputs => "Outputs",
            Self::Stats => "Stats",
            Self::Lyrics => "Lyrics",
            Self::Visualizer => "Visualizer",
            Self::TagEditor => "Tags",
            Self::History => "History",
            Self::Browser => "Browser",
        }
    }

    /// The number shown in the header and used by the number keys.
    pub fn key_number(self) -> usize {
        if self == Self::Browser {
            0
        } else {
            self.index() + 1
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Library => "library",
            Self::Playlists => "playlists",
            Self::Outputs => "outputs",
            Self::Stats => "stats",
            Self::Lyrics => "lyrics",
            Self::Visualizer => "visualizer",
            Self::TagEditor => "tags",
            Self::History => "history",
            Self::Browser => "browser",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> Self {
        Self::ALL[(self.index() + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    pub fn from_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "library" => Self::Library,
            "playlists" => Self::Playlists,
            "outputs" => Self::Outputs,
            "stats" => Self::Stats,
            "lyrics" => Self::Lyrics,
            "visualizer" => Self::Visualizer,
            "tags" | "tag-editor" | "tag editor" => Self::TagEditor,
            "history" => Self::History,
            "browser" | "files" | "file browser" => Self::Browser,
            _ => Self::Queue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryPane {
    #[default]
    Artists,
    Albums,
    Songs,
}

#[derive(Debug, Default)]
pub struct LibraryView {
    pub pane: LibraryPane,
    pub artists: ListView,
    pub albums: ListView,
    pub songs: ListView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaylistPane {
    #[default]
    Playlists,
    Songs,
}

#[derive(Debug, Clone)]
pub struct InputModal {
    pub prompt: String,
    pub value: String,
    pub action: InputAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKey {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Date,
    Genre,
    Track,
    Disc,
    Comment,
}

impl TagKey {
    pub const ALL: [Self; 9] = [
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::AlbumArtist,
        Self::Date,
        Self::Genre,
        Self::Track,
        Self::Disc,
        Self::Comment,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::AlbumArtist => "Album Artist",
            Self::Date => "Date",
            Self::Genre => "Genre",
            Self::Track => "Track",
            Self::Disc => "Disc",
            Self::Comment => "Comment",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TagField {
    pub key: TagKey,
    pub value: String,
}

#[derive(Debug, Default)]
pub struct TagEditorState {
    pub uri: Option<Arc<str>>,
    pub fields: Vec<TagField>,
    pub cursor: usize,
    pub dirty: bool,
    pub message: Option<(String, bool)>,
}

impl TagEditorState {
    pub fn from_song(song: &Song) -> Self {
        let mut fields = Vec::with_capacity(TagKey::ALL.len());
        for key in TagKey::ALL {
            let value = match key {
                TagKey::Title => song.title.as_deref().unwrap_or_default().to_string(),
                TagKey::Artist => song.artist.as_deref().unwrap_or_default().to_string(),
                TagKey::Album => song.album.as_deref().unwrap_or_default().to_string(),
                TagKey::AlbumArtist => song.album_artist.as_deref().unwrap_or_default().to_string(),
                TagKey::Date => song.date.as_deref().unwrap_or_default().to_string(),
                TagKey::Genre => song.genre.as_deref().unwrap_or_default().to_string(),
                TagKey::Track => song
                    .track
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                TagKey::Disc => song.disc.map(|value| value.to_string()).unwrap_or_default(),
                TagKey::Comment => song
                    .tags
                    .iter()
                    .find(|(key, _)| key.as_ref() == "Comment")
                    .map(|(_, value)| value.to_string())
                    .unwrap_or_default(),
            };
            fields.push(TagField { key, value });
        }
        Self {
            uri: Some(song.uri.clone()),
            fields,
            cursor: 0,
            dirty: false,
            message: None,
        }
    }

    pub fn value(&self, key: TagKey) -> String {
        self.fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.value.trim().to_string())
            .unwrap_or_default()
    }

    fn number(&self, key: TagKey) -> Result<Option<u32>, String> {
        let value = self.value(key);
        if value.is_empty() {
            Ok(None)
        } else {
            value
                .parse()
                .map(Some)
                .map_err(|_| format!("{} must be a number", key.label()))
        }
    }

    pub fn to_update(&self) -> Result<TagUpdate, String> {
        Ok(TagUpdate {
            title: self.value(TagKey::Title),
            artist: self.value(TagKey::Artist),
            album: self.value(TagKey::Album),
            album_artist: self.value(TagKey::AlbumArtist),
            date: self.value(TagKey::Date),
            genre: self.value(TagKey::Genre),
            comment: self.value(TagKey::Comment),
            track: self.number(TagKey::Track)?,
            disc: self.number(TagKey::Disc)?,
        })
    }
}

#[derive(Debug, Default)]
pub struct BrowserState {
    pub uri: Option<String>,
    pub entries: Vec<DirEntry>,
    pub view: ListView,
    pub loading: bool,
}

#[derive(Debug, Default)]
pub struct LyricsView {
    pub doc: Option<LyricsDoc>,
    pub uri: Option<Arc<str>>,
    pub loading: bool,
    /// Positive shifts lyrics later; negative earlier.
    pub offset_ms: i64,
    pub scroll: usize,
}

impl LyricsView {
    pub fn adjusted_position(&self, position: Duration) -> Duration {
        if self.offset_ms >= 0 {
            position + Duration::from_millis(self.offset_ms as u64)
        } else {
            position.saturating_sub(Duration::from_millis((-self.offset_ms) as u64))
        }
    }
}

#[derive(Debug, Clone)]
pub enum InputAction {
    SaveQueueAsPlaylist,
    RenamePlaylist { from: String },
    EditTagField { index: usize },
}

#[derive(Debug, Clone)]
pub enum ConnectionState {
    Connecting,
    Connected(String),
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub created: Instant,
}

impl Toast {
    fn expired(&self) -> bool {
        self.created.elapsed() > Duration::from_secs(4)
    }
}

#[derive(Debug, Clone)]
pub struct Confirm {
    pub message: String,
    pub action: ConfirmAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmAction {
    ClearQueue,
    DeletePlaylist(String),
    SaveTags,
}

#[derive(Debug, Clone)]
struct PlaybackScrobble {
    uri: Arc<str>,
    track: ScrobbleTrack,
    duration: Option<Duration>,
    elapsed: Duration,
    submitted: bool,
    recorded: bool,
    now_playing_sent: bool,
    timestamp: u64,
}

impl PlaybackScrobble {
    fn new(song: &Song) -> Self {
        Self {
            uri: song.uri.clone(),
            track: ScrobbleTrack::from_song(song),
            duration: song.duration,
            elapsed: Duration::ZERO,
            submitted: false,
            recorded: false,
            now_playing_sent: false,
            timestamp: now_unix(),
        }
    }
}

pub struct AppDeps {
    pub mpd: MpdHandle,
    pub search_handle: SearchHandle,
    pub picker: Picker,
    pub mpris_tx: watch::Sender<MprisSnapshot>,
    pub scrobbler: Option<ScrobbleHandle>,
    pub event_tx: mpsc::UnboundedSender<Msg>,
    pub config_path: Option<PathBuf>,
}

pub struct App {
    pub config: Config,
    pub mpd: MpdHandle,
    pub screen: Screen,

    pub status: Option<MpdStatus>,
    pub current: Option<Song>,
    pub queue: Vec<Song>,
    pub queue_view: ListView,

    pub library: Option<Arc<Library>>,
    pub library_view: LibraryView,

    pub outputs: Vec<MpdOutput>,
    pub outputs_view: ListView,
    pub stats: Option<MpdStats>,

    pub playlists: Vec<Playlist>,
    pub playlists_view: ListView,
    pub playlist_pane: PlaylistPane,
    pub playlist_songs: Vec<Song>,
    pub playlist_songs_view: ListView,
    loaded_playlist: Option<String>,

    pub search: SearchState,
    pub search_index: Option<Arc<SearchIndex>>,
    search_handle: SearchHandle,
    search_generation: u64,

    pub lyrics: LyricsView,
    lyrics_cache: std::collections::HashMap<Arc<str>, LyricsDoc>,

    pub spectrum: Vec<f32>,
    pub visualizer_error: Option<String>,

    pub tag_editor: TagEditorState,
    pub palette: PaletteState,
    keymap: Keymap,

    pub history: Option<StatsReport>,
    pub history_loading: bool,
    pub history_error: Option<String>,
    pub history_cursor: usize,

    pub browser: BrowserState,

    scrobbler: Option<ScrobbleHandle>,
    scrobble: Option<PlaybackScrobble>,
    event_tx: mpsc::UnboundedSender<Msg>,
    config_path: Option<PathBuf>,
    config_reload_at: Option<Instant>,

    mpris_tx: watch::Sender<MprisSnapshot>,

    pub cover_protocol: Option<StatefulProtocol>,
    picker: Picker,
    cover_requested: Option<Arc<str>>,
    cover_cache: std::collections::HashMap<Arc<str>, Option<Arc<Picture>>>,
    remote_cover_inflight: HashSet<Arc<str>>,

    pub connection: ConnectionState,
    pub version: Option<String>,
    pub help: bool,
    pub confirm: Option<Confirm>,
    pub input: Option<InputModal>,
    pub toast: Option<Toast>,
    pub should_quit: bool,

    elapsed_at_sync: Duration,
    elapsed_synced_at: Instant,
    tick: u64,
    library_reload_at: Option<Instant>,
}

impl App {
    pub fn new(config: Config, deps: AppDeps) -> Self {
        let AppDeps {
            mpd,
            search_handle,
            picker,
            mpris_tx,
            scrobbler,
            event_tx,
            config_path,
        } = deps;
        let keymap = Keymap::from_config(&config.keys);
        Self {
            screen: Screen::from_name(&config.ui.start_screen),
            config,
            mpd,
            status: None,
            current: None,
            queue: Vec::new(),
            queue_view: ListView::default(),
            library: None,
            library_view: LibraryView::default(),
            outputs: Vec::new(),
            outputs_view: ListView::default(),
            stats: None,
            playlists: Vec::new(),
            playlists_view: ListView::default(),
            playlist_pane: PlaylistPane::default(),
            playlist_songs: Vec::new(),
            playlist_songs_view: ListView::default(),
            loaded_playlist: None,
            search: SearchState::default(),
            search_index: None,
            search_handle,
            search_generation: 0,
            lyrics: LyricsView::default(),
            lyrics_cache: std::collections::HashMap::new(),
            spectrum: Vec::new(),
            visualizer_error: None,
            tag_editor: TagEditorState::default(),
            palette: PaletteState::default(),
            keymap,
            history: None,
            history_loading: false,
            history_error: None,
            history_cursor: 0,
            browser: BrowserState::default(),
            scrobbler,
            scrobble: None,
            event_tx,
            config_path,
            config_reload_at: None,
            mpris_tx,
            cover_protocol: None,
            picker,
            cover_requested: None,
            cover_cache: std::collections::HashMap::new(),
            remote_cover_inflight: HashSet::new(),
            connection: ConnectionState::Connecting,
            version: None,
            help: false,
            confirm: None,
            input: None,
            toast: None,
            should_quit: false,
            elapsed_at_sync: Duration::ZERO,
            elapsed_synced_at: Instant::now(),
            tick: 0,
            library_reload_at: None,
        }
    }

    pub async fn run(
        config: Config,
        config_path: Option<PathBuf>,
        terminal: &mut DefaultTerminal,
    ) -> anyhow::Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let endpoint = mpd_worker::Endpoint::new(config.mpd_addr(), config.mpd.password.clone());
        let mpd = mpd_worker::spawn(endpoint.clone(), tx.clone());
        mpd_worker::spawn_idle(endpoint, tx.clone());
        let search_handle = SearchHandle::spawn(tx.clone());
        if config.visualizer.enabled {
            visualizer::spawn(
                config.visualizer.fifo.clone(),
                &config.visualizer.source,
                config.visualizer.target.clone(),
                visualizer::AudioSpec {
                    sample_rate: config.visualizer.sample_rate,
                    channels: config.visualizer.channels,
                    bits: config.visualizer.bits,
                },
                config.visualizer.bars,
                tx.clone(),
            );
        }
        let (mpris_tx, mpris_rx) = watch::channel(MprisSnapshot::default());
        mpris::spawn(mpd.clone(), mpris_rx);
        let scrobbler = if config.scrobbling.enabled {
            Some(scrobbler::spawn(config.scrobbling.clone()))
        } else {
            None
        };

        // Must run before EventStream starts consuming stdin.
        let picker = Picker::from_query_stdio().unwrap_or_else(|error| {
            tracing::warn!(%error, "terminal image detection failed; using halfblocks");
            Picker::halfblocks()
        });

        ui::apply_theme(&config.theme);
        if let Some(path) = &config_path {
            spawn_config_watcher(path.clone(), tx.clone());
        }
        let mut app = App::new(
            config.clone(),
            AppDeps {
                mpd,
                search_handle,
                picker,
                mpris_tx,
                scrobbler,
                event_tx: tx.clone(),
                config_path,
            },
        );
        app.restore_session();
        let mut events = EventStream::new();
        let tick_rate = Duration::from_millis(config.ui.tick_rate_ms.max(50));
        let mut ticker = tokio::time::interval(tick_rate);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        #[cfg(unix)]
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        #[cfg(unix)]
        let terminate_signal = async move {
            let _ = terminate.recv().await;
        };
        #[cfg(not(unix))]
        let terminate_signal = std::future::pending::<()>();
        tokio::pin!(terminate_signal);

        loop {
            tokio::select! {
                _ = &mut ctrl_c => break,
                _ = &mut terminate_signal => break,
                event = events.next() => match event {
                    Some(Ok(Event::Key(key))) => app.on_key(key),
                    Some(Ok(Event::Resize(_, _))) => {}
                    Some(Ok(_)) => {}
                    Some(Err(error)) => tracing::warn!(%error, "terminal event error"),
                    None => break,
                },
                message = rx.recv() => {
                    if let Some(message) = message {
                        app.on_msg(message);
                    } else {
                        break;
                    }
                }
                _ = ticker.tick() => app.on_tick(),
            }

            terminal.draw(|frame| ui::draw(&mut app, frame))?;
            if app.should_quit {
                break;
            }
        }
        app.save_session();
        Ok(())
    }

    pub fn restore_session(&mut self) {
        if !self.config.ui.restore_session {
            return;
        }
        let session = SessionState::load();
        if let Some(name) = session.screen.as_deref() {
            self.screen = Screen::from_name(name);
        }
        self.queue_view.cursor = session.queue_cursor;
        self.library_view.pane = match session.library_pane {
            1 => LibraryPane::Albums,
            2 => LibraryPane::Songs,
            _ => LibraryPane::Artists,
        };
        self.library_view.artists.cursor = session.library_artist;
        self.library_view.albums.cursor = session.library_album;
        self.library_view.songs.cursor = session.library_song;
        self.playlists_view.cursor = session.playlist_cursor;
        self.playlist_pane = if session.playlist_pane == 1 {
            PlaylistPane::Songs
        } else {
            PlaylistPane::Playlists
        };
        self.history_cursor = session.history_cursor;
        self.lyrics.offset_ms = session.lyrics_offset_ms.clamp(-10_000, 10_000);
        match self.screen {
            Screen::History => self.load_history(),
            Screen::Lyrics => self.request_lyrics(false),
            Screen::TagEditor => self.load_tag_editor(),
            Screen::Browser => self.load_browser(self.browser.uri.clone()),
            _ => {}
        }
    }

    fn save_session(&self) {
        if !self.config.ui.restore_session {
            return;
        }
        let session = SessionState {
            screen: Some(self.screen.name().to_string()),
            queue_cursor: self.queue_view.cursor,
            library_pane: match self.library_view.pane {
                LibraryPane::Artists => 0,
                LibraryPane::Albums => 1,
                LibraryPane::Songs => 2,
            },
            library_artist: self.library_view.artists.cursor,
            library_album: self.library_view.albums.cursor,
            library_song: self.library_view.songs.cursor,
            playlist_cursor: self.playlists_view.cursor,
            playlist_pane: if self.playlist_pane == PlaylistPane::Songs {
                1
            } else {
                0
            },
            history_cursor: self.history_cursor,
            lyrics_offset_ms: self.lyrics.offset_ms,
        };
        session.save();
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        if self.confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => self.resolve_confirm(),
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => self.confirm = None,
                _ => {}
            }
            return;
        }

        if self.input.is_some() {
            self.handle_input_key(key);
            return;
        }

        if self.help {
            self.help = false;
            return;
        }

        if self.search.active {
            self.handle_search_key(key);
            return;
        }

        if self.palette.active {
            self.handle_palette_key(key);
            return;
        }

        if let Some(action) = self.keymap.action_for(key, self.screen) {
            self.handle_action(action);
        }
    }

    pub fn on_msg(&mut self, message: Msg) {
        match message {
            Msg::Connected { version } => {
                self.connection = ConnectionState::Connected(version.clone());
                self.version = Some(version);
                self.mpd.send(Command::RefreshPlaylists);
                self.toast_success("Connected to MPD");
            }
            Msg::Disconnected { error } => {
                self.connection = ConnectionState::Disconnected;
                self.toast_error(format!("Disconnected: {error}"));
            }
            Msg::Status(status) => {
                let previous_id = self.status.as_ref().and_then(|status| status.song_id);
                if previous_id != status.song_id {
                    self.mpd.send(Command::RefreshCurrent);
                    if self.config.ui.follow_now_playing
                        && let Some(position) = status.song_pos
                    {
                        self.queue_view.cursor = position as usize;
                    }
                }
                if let Some(track) = &mut self.scrobble {
                    if let Some(duration) = status.duration {
                        track.duration = Some(duration);
                    }
                    let current_id = self.current.as_ref().and_then(|song| song.id);
                    if status.song_id.is_none() || status.song_id == current_id {
                        track.elapsed = track.elapsed.max(status.elapsed);
                    }
                }
                if status.state.is_playing() {
                    self.send_now_playing();
                }
                self.maybe_submit_scrobble();
                self.elapsed_at_sync = status.elapsed;
                self.elapsed_synced_at = Instant::now();
                self.status = Some(status);
                self.publish_mpris();
            }
            Msg::CurrentSong(song) => {
                let previous_existed = self.current.is_some();
                let changed = self
                    .current
                    .as_ref()
                    .map(|current| (&current.uri, current.id))
                    != song.as_ref().map(|song| (&song.uri, song.id));
                if changed {
                    if let Some(previous) = self.scrobble.take() {
                        self.submit_finished_scrobble(previous);
                    }
                    self.current = song;
                    if let Some(song) = &self.current {
                        self.scrobble = Some(PlaybackScrobble::new(song));
                    } else {
                        self.scrobble = None;
                    }
                    if previous_existed
                        && self.config.ui.notifications
                        && let Some(song) = &self.current
                    {
                        crate::notifications::track_changed(song);
                    }
                } else {
                    // Reconnect or metadata refresh for the same track.
                    self.current = song;
                }
                if self
                    .status
                    .as_ref()
                    .is_some_and(|status| status.state.is_playing())
                {
                    self.send_now_playing();
                }
                self.request_cover();
                if self.screen == Screen::Lyrics {
                    self.request_lyrics(false);
                }
                if self.screen == Screen::TagEditor {
                    self.load_tag_editor();
                }
                self.publish_mpris();
            }
            Msg::Queue(queue) => {
                self.queue = queue;
                self.queue_view.clamp(self.queue.len());
            }
            Msg::Library(library) => {
                let index = Arc::new(SearchIndex::build(&library));
                self.search_index = Some(index.clone());
                self.library = Some(library);
                self.clamp_library_views();
                if self.search.active {
                    self.request_search(index);
                }
                tracing::info!(
                    artists = self.library_artist_count(),
                    albums = self.library_album_count(),
                    songs = self.library.as_ref().map_or(0, |library| library.len()),
                    "library indexed"
                );
            }
            Msg::Outputs(outputs) => {
                self.outputs = outputs;
                self.outputs_view.clamp(self.outputs.len());
            }
            Msg::Stats(stats) => self.stats = Some(stats),
            Msg::Playlists(mut playlists) => {
                playlists.sort_by_key(|playlist| playlist.name.to_lowercase());
                self.playlists = playlists;
                self.playlists_view.clamp(self.playlists.len());
                self.ensure_playlist_songs();
            }
            Msg::PlaylistSongs { name, songs } => {
                if self.selected_playlist_name().as_deref() == Some(name.as_str()) {
                    self.playlist_songs = songs;
                    self.playlist_songs_view.clamp(self.playlist_songs.len());
                    self.loaded_playlist = Some(name);
                }
            }
            Msg::Cover { uri, picture } => {
                let uri: Arc<str> = Arc::from(uri.as_str());
                let picture = picture.map(Arc::new);
                let missing = picture.is_none();
                if self.cover_cache.len() >= 64 {
                    self.cover_cache.clear();
                }
                self.cover_cache.insert(uri.clone(), picture.clone());
                if self.current.as_ref().is_some_and(|song| song.uri == uri) {
                    self.set_cover(picture);
                }
                if missing {
                    self.request_online_cover();
                }
            }
            Msg::OnlineCover { uri, picture } => {
                let uri: Arc<str> = Arc::from(uri.as_str());
                self.remote_cover_inflight.remove(&uri);
                let picture = picture.map(Arc::new);
                if self.cover_cache.len() >= 64 {
                    self.cover_cache.clear();
                }
                self.cover_cache.insert(uri.clone(), picture.clone());
                if self.current.as_ref().is_some_and(|song| song.uri == uri) {
                    self.set_cover(picture);
                }
            }
            Msg::Visualizer(bars) => {
                self.spectrum = bars;
                self.visualizer_error = None;
            }
            Msg::VisualizerError(error) => {
                self.visualizer_error = error;
                if self.visualizer_error.is_some() {
                    self.spectrum.clear();
                }
            }
            Msg::ConfigFileChanged => {
                self.config_reload_at = Some(Instant::now() + Duration::from_millis(250));
            }
            Msg::Browser { uri, entries } => {
                if self.browser.uri == uri {
                    let mut entries = entries;
                    entries.sort_by_key(|entry| {
                        let rank = match entry.kind {
                            EntryKind::Directory => 0,
                            EntryKind::Playlist => 1,
                            EntryKind::File => 2,
                        };
                        (rank, entry.name.to_lowercase())
                    });
                    self.browser.entries = entries;
                    self.browser.loading = false;
                    self.browser.view.clamp(self.browser.entries.len());
                }
            }
            Msg::History(report) => {
                self.history_cursor = self
                    .history_cursor
                    .min(report.top_songs.len().saturating_sub(1));
                self.history = Some(report);
                self.history_loading = false;
                self.history_error = None;
            }
            Msg::HistoryError(error) => {
                self.history_loading = false;
                self.history_error = Some(error);
            }
            Msg::TagsSaved { uri, backup, error } => {
                if let Some(error) = error {
                    self.tag_editor.message = Some((error.clone(), true));
                    self.toast_error(error);
                } else {
                    let message = match &backup {
                        Some(path) => format!("Saved · backup {}", path.display()),
                        None => "Saved".to_string(),
                    };
                    tracing::info!(%uri, ?backup, "tags saved");
                    self.tag_editor.message = Some((message, false));
                    self.tag_editor.dirty = false;
                    self.toast_success("Tags saved");
                    self.mpd.send(Command::RefreshCurrent);
                    self.mpd.send(Command::RefreshQueue);
                    self.library_reload_at = Some(Instant::now() + Duration::from_millis(700));
                }
            }
            Msg::Lyrics { uri, doc } => {
                let uri: Arc<str> = Arc::from(uri.as_str());
                if self.lyrics.uri.as_deref() == Some(uri.as_ref()) {
                    self.lyrics.loading = false;
                    if let Some(document) = &doc {
                        self.lyrics_cache.insert(uri, document.clone());
                    }
                    self.lyrics.doc = doc;
                }
            }
            Msg::SearchResults {
                generation,
                results,
            } => {
                if generation == self.search_generation && self.search.active {
                    self.search.apply(results);
                }
            }
            Msg::Idle(subsystems) => self.on_idle(subsystems),
            Msg::Error(error) => {
                tracing::warn!(%error, "MPD error");
                self.toast_error(error);
            }
        }
    }

    pub fn on_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);

        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }

        if self
            .config_reload_at
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.config_reload_at = None;
            self.reload_config();
        }

        if self
            .library_reload_at
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.library_reload_at = None;
            self.mpd.send(Command::LoadLibrary);
        }

        let refresh_every = (2000 / self.config.ui.tick_rate_ms.max(50)).max(1);
        if self.tick.is_multiple_of(refresh_every)
            && self
                .status
                .as_ref()
                .is_some_and(|status| status.state.is_playing())
        {
            self.mpd.send(Command::RefreshStatus);
        }
    }

    fn on_idle(&mut self, subsystems: Vec<Subsystem>) {
        let mut refresh_status = false;
        let mut refresh_current = false;
        let mut refresh_queue = false;

        for subsystem in subsystems {
            match subsystem {
                Subsystem::Player => {
                    refresh_status = true;
                    refresh_current = true;
                }
                Subsystem::Playlist => {
                    refresh_status = true;
                    refresh_queue = true;
                }
                Subsystem::Mixer | Subsystem::Options | Subsystem::Partition => {
                    refresh_status = true;
                }
                Subsystem::Output => self.mpd.send(Command::RefreshOutputs),
                Subsystem::StoredPlaylist => {
                    self.mpd.send(Command::RefreshPlaylists);
                    if let Some(name) = self.selected_playlist_name() {
                        self.mpd.send(Command::RefreshPlaylistSongs(name));
                    }
                }
                Subsystem::Database | Subsystem::Update => {
                    // A database update emits many idle events; coalesce them.
                    self.library_reload_at = Some(Instant::now() + Duration::from_millis(400));
                    self.mpd.send(Command::RefreshStats);
                }
                _ => {}
            }
        }

        if refresh_status {
            self.mpd.send(Command::RefreshStatus);
        }
        if refresh_current {
            self.mpd.send(Command::RefreshCurrent);
        }
        if refresh_queue {
            self.mpd.send(Command::RefreshQueue);
        }
    }

    fn handle_action(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::ToggleHelp => self.help = !self.help,
            Action::OpenSearch => self.open_search(),
            Action::OpenPalette => self.open_palette(),
            Action::NextScreen => self.set_screen(self.screen.next()),
            Action::PreviousScreen => self.set_screen(self.screen.previous()),
            Action::GotoScreen(index) => {
                if let Some(screen) = Screen::ALL.get(index) {
                    self.set_screen(*screen);
                }
            }
            Action::Refresh => {
                if self.screen == Screen::History {
                    self.load_history();
                } else if self.screen == Screen::Browser {
                    self.load_browser(self.browser.uri.clone());
                } else {
                    self.mpd.send(Command::RefreshAll);
                    self.toast_info("Refreshing…");
                }
            }
            Action::MoveDown => self.move_selection(1),
            Action::MoveUp => self.move_selection(-1),
            Action::PageDown => self.page_selection(1),
            Action::PageUp => self.page_selection(-1),
            Action::First => self.jump_to_edge(false),
            Action::Last => self.jump_to_edge(true),
            Action::TogglePlay => self.toggle_play(),
            Action::Stop => self.mpd.send(Command::Stop),
            Action::NextTrack => self.mpd.send(Command::Next),
            Action::PreviousTrack => self.mpd.send(Command::Previous),
            Action::VolumeUp => self.change_volume(5),
            Action::VolumeDown => self.change_volume(-5),
            Action::SeekForward => self.mpd.send(Command::Seek(5_000)),
            Action::SeekBackward => self.mpd.send(Command::Seek(-5_000)),
            Action::ToggleRepeat => self.toggle_option(PlayerOption::Repeat),
            Action::ToggleRandom => self.toggle_option(PlayerOption::Random),
            Action::ToggleSingle => self.toggle_option(PlayerOption::Single),
            Action::ToggleConsume => self.toggle_option(PlayerOption::Consume),
            Action::PlaySelected => self.play_selected(),
            Action::EnqueueSelected => self.enqueue_selected(),
            Action::ReplaceAndPlay => self.replace_and_play(),
            Action::RemoveSelected => self.remove_selected(),
            Action::MoveSelectionDown => self.move_selected_item(1),
            Action::MoveSelectionUp => self.move_selected_item(-1),
            Action::ClearQueue => self.request_clear_queue(),
            Action::ShuffleQueue => {
                self.mpd.send(Command::Shuffle);
                self.toast_info("Queue shuffled");
            }
            Action::PaneLeft => self.change_pane(-1),
            Action::PaneRight => self.change_pane(1),
            Action::ToggleOutput => self.toggle_output(),
            Action::BrowserUp => self.browser_up(),
            Action::ReloadLyrics => self.request_lyrics(true),
            Action::LyricsEarlier => self.adjust_lyrics_offset(-500),
            Action::LyricsLater => self.adjust_lyrics_offset(500),
            Action::EditTag => self.open_tag_input(),
            Action::SaveTags => self.request_save_tags(),
            Action::SavePlaylist => self.open_save_playlist(),
            Action::RenamePlaylist => self.open_rename_playlist(),
            Action::DeletePlaylist => self.request_delete_playlist(),
        }
    }

    fn publish_mpris(&self) {
        let snapshot = MprisSnapshot {
            playback: self
                .status
                .as_ref()
                .map_or(PlaybackState::Stopped, |status| status.state),
            volume: self.status.as_ref().and_then(|status| status.volume),
            current: self.current.clone(),
            position: self.effective_elapsed(),
            repeat: self.status.as_ref().is_some_and(|status| status.repeat),
            random: self.status.as_ref().is_some_and(|status| status.random),
            single: self.status.as_ref().is_some_and(|status| status.single),
        };
        let _ = self.mpris_tx.send(snapshot);
    }

    fn load_browser(&mut self, uri: Option<String>) {
        self.browser.uri = uri.clone();
        self.browser.loading = true;
        self.browser.entries.clear();
        self.browser.view.reset();
        self.mpd.send(Command::Browse(uri));
    }

    fn browser_up(&mut self) {
        let Some(current) = self.browser.uri.clone() else {
            return;
        };
        let parent = current
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_string())
            .filter(|parent| !parent.is_empty());
        self.load_browser(parent);
    }

    fn browser_enter(&mut self) {
        let Some(entry) = self.browser.entries.get(self.browser.view.cursor).cloned() else {
            return;
        };
        match entry.kind {
            EntryKind::Directory => self.load_browser(Some(entry.uri.to_string())),
            EntryKind::Playlist => self.mpd.send(Command::LoadPlaylist {
                name: entry.uri.to_string(),
                replace: true,
            }),
            EntryKind::File => self
                .mpd
                .send(Command::AppendAndPlay(vec![entry.uri.to_string()])),
        }
    }

    fn browser_enqueue(&mut self) {
        let Some(entry) = self.browser.entries.get(self.browser.view.cursor) else {
            return;
        };
        match entry.kind {
            EntryKind::Playlist => self
                .mpd
                .send(Command::AppendPlaylist(entry.uri.to_string())),
            EntryKind::Directory | EntryKind::File => {
                self.mpd.send(Command::AppendUri(entry.uri.to_string()))
            }
        }
    }

    fn load_history(&mut self) {
        self.history_loading = true;
        self.history_error = None;
        let tx = self.event_tx.clone();
        std::thread::spawn(move || match stats::load_report() {
            Ok(report) => {
                let _ = tx.send(Msg::History(report));
            }
            Err(error) => {
                let _ = tx.send(Msg::HistoryError(error));
            }
        });
    }

    fn record_play(&self, track: &PlaybackScrobble) {
        if !self.config.stats.enabled {
            return;
        }
        let completed = track
            .duration
            .is_some_and(|duration| track.elapsed + Duration::from_secs(5) >= duration);
        stats::spawn_record(
            track.uri.to_string(),
            track.track.artist.clone(),
            track.track.title.clone(),
            track.track.album.clone(),
            track.timestamp,
            track.elapsed.as_secs(),
            completed,
        );
    }

    fn send_now_playing(&mut self) {
        let Some(track) = &mut self.scrobble else {
            return;
        };
        if track.now_playing_sent {
            return;
        }
        track.now_playing_sent = true;
        let event = ScrobbleEvent {
            kind: ScrobbleKind::NowPlaying,
            track: track.track.clone(),
            timestamp: now_unix(),
        };
        if let Some(scrobbler) = &self.scrobbler {
            scrobbler.send(event);
        }
    }

    fn maybe_submit_scrobble(&mut self) {
        let Some(track) = &self.scrobble else {
            return;
        };
        if track.elapsed < scrobble_threshold(track.duration) {
            return;
        }
        let should_record = !track.recorded;
        let should_submit = !track.submitted;
        let event = should_submit.then(|| ScrobbleEvent {
            kind: ScrobbleKind::Single,
            track: track.track.clone(),
            timestamp: track.timestamp,
        });
        if should_record {
            self.record_play(track);
        }
        if should_record && let Some(track) = &mut self.scrobble {
            track.recorded = true;
        }
        if let Some(event) = event {
            if let Some(scrobbler) = &self.scrobbler {
                scrobbler.send(event);
            }
            if let Some(track) = &mut self.scrobble {
                track.submitted = true;
            }
        }
    }

    fn submit_finished_scrobble(&self, track: PlaybackScrobble) {
        if track.elapsed < scrobble_threshold(track.duration) {
            return;
        }
        if !track.recorded {
            self.record_play(&track);
        }
        if track.submitted {
            return;
        }
        if let Some(scrobbler) = &self.scrobbler {
            scrobbler.send(ScrobbleEvent {
                kind: ScrobbleKind::Single,
                track: track.track,
                timestamp: track.timestamp,
            });
        }
    }

    fn load_tag_editor(&mut self) {
        let Some(song) = &self.current else {
            self.tag_editor = TagEditorState::default();
            return;
        };
        if self.tag_editor.uri.as_deref() == Some(song.uri.as_ref())
            && !self.tag_editor.fields.is_empty()
        {
            return;
        }
        self.tag_editor = TagEditorState::from_song(song);
    }

    fn open_tag_input(&mut self) {
        let Some(field) = self.tag_editor.fields.get(self.tag_editor.cursor) else {
            return;
        };
        self.input = Some(InputModal {
            prompt: format!("Edit {}", field.key.label()),
            value: field.value.clone(),
            action: InputAction::EditTagField {
                index: self.tag_editor.cursor,
            },
        });
    }

    fn request_save_tags(&mut self) {
        if !self.tag_editor.dirty {
            self.toast_info("No tag changes");
            return;
        }
        if self.config.library.music_directory.is_none() {
            self.toast_error("Set [library] music_directory to edit tags");
            return;
        }
        self.confirm = Some(Confirm {
            message: "Write tags to disk? A backup will be created.".into(),
            action: ConfirmAction::SaveTags,
        });
    }

    fn save_tags(&mut self) {
        let Some(directory) = self.config.library.music_directory.clone() else {
            self.toast_error("Set [library] music_directory to edit tags");
            return;
        };
        let Some(uri) = self.tag_editor.uri.clone() else {
            return;
        };
        if uri.contains("://") || uri.starts_with('/') || uri.split('/').any(|part| part == "..") {
            self.toast_error("This track cannot be edited");
            return;
        }
        let update = match self.tag_editor.to_update() {
            Ok(update) => update,
            Err(error) => {
                self.toast_error(error);
                return;
            }
        };
        let path = directory.join(uri.as_ref());
        self.tag_editor.message = Some(("Writing tags…".into(), false));
        self.mpd.send(Command::SaveTags {
            path,
            uri: uri.to_string(),
            update,
        });
    }

    fn request_lyrics(&mut self, force: bool) {
        let Some(song) = self.current.as_ref() else {
            self.lyrics = LyricsView::default();
            return;
        };
        let uri = song.uri.clone();
        if !force && let Some(cached) = self.lyrics_cache.get(uri.as_ref()) {
            self.lyrics.doc = Some(cached.clone());
            self.lyrics.uri = Some(uri);
            self.lyrics.loading = false;
            return;
        }
        self.lyrics.uri = Some(uri.clone());
        self.lyrics.doc = None;
        self.lyrics.loading = true;
        self.lyrics.scroll = 0;
        self.mpd.send(Command::FetchLyrics {
            uri: uri.to_string(),
            artist: song.display_artist().to_owned(),
            title: song.display_title().to_owned(),
            album: song.display_album().to_owned(),
            duration_seconds: song.duration.map(|duration| duration.as_secs()),
            music_directory: self.config.library.music_directory.clone(),
        });
    }

    fn adjust_lyrics_offset(&mut self, delta_ms: i64) {
        self.lyrics.offset_ms = (self.lyrics.offset_ms + delta_ms).clamp(-10_000, 10_000);
    }

    fn scroll_lyrics(&mut self, delta: isize) {
        let Some(doc) = &self.lyrics.doc else {
            return;
        };
        if !doc.synced.is_empty() {
            return;
        }
        let lines = doc.plain.as_deref().map_or(0, |text| text.lines().count());
        self.lyrics.scroll = self
            .lyrics
            .scroll
            .saturating_add_signed(delta)
            .min(lines.saturating_sub(1));
    }

    fn request_cover(&mut self) {
        let Some(uri) = self.current.as_ref().map(|song| song.uri.clone()) else {
            self.cover_protocol = None;
            self.cover_requested = None;
            return;
        };
        if self.cover_requested.as_deref() == Some(uri.as_ref()) {
            return;
        }
        self.cover_requested = Some(uri.clone());
        if let Some(cached) = self.cover_cache.get(uri.as_ref()) {
            let cached = cached.clone();
            let missing = cached.is_none();
            self.set_cover(cached);
            if missing {
                self.request_online_cover();
            }
            return;
        }
        self.mpd.send(Command::FetchCover(uri.to_string()));
    }

    fn request_online_cover(&mut self) {
        if !self.config.artwork.online {
            return;
        }
        let Some(song) = self.current.clone() else {
            return;
        };
        if self.remote_cover_inflight.contains(&song.uri) {
            return;
        }
        let artist = song
            .artist
            .as_deref()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                song.album_artist
                    .as_deref()
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_default()
            .to_string();
        let album = song
            .album
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string();
        let title = song
            .title
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string();
        if artist.is_empty() && album.is_empty() && title.is_empty() {
            return;
        }
        if self.remote_cover_inflight.len() >= 64 {
            self.remote_cover_inflight.clear();
        }
        self.remote_cover_inflight.insert(song.uri.clone());
        coverart::spawn(
            coverart::Request {
                uri: song.uri.to_string(),
                artist,
                album,
                title,
            },
            self.event_tx.clone(),
        );
    }

    fn set_cover(&mut self, picture: Option<Arc<Picture>>) {
        self.cover_protocol = None;
        let Some(picture) = picture else {
            return;
        };
        match image::load_from_memory(&picture.data) {
            Ok(image) => {
                self.cover_protocol = Some(self.picker.new_resize_protocol(image));
            }
            Err(error) => tracing::warn!(%error, "could not decode cover art"),
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.palette.close(),
            KeyCode::Enter => {
                if let Some(item) = self.palette.selected().cloned() {
                    self.palette.close();
                    match item.action {
                        PaletteAction::Action(action) => self.handle_action(action),
                        PaletteAction::Search(query) => self.open_search_with(query),
                    }
                }
            }
            KeyCode::Up => self.palette.move_cursor(-1),
            KeyCode::Down => self.palette.move_cursor(1),
            KeyCode::PageUp => self.palette.move_cursor(-10),
            KeyCode::PageDown => self.palette.move_cursor(10),
            KeyCode::Backspace => {
                self.palette.query.pop();
                self.palette.refresh();
            }
            KeyCode::Char('u') if ctrl => {
                self.palette.query.clear();
                self.palette.refresh();
            }
            KeyCode::Char(character) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.palette.query.push(character);
                self.palette.refresh();
            }
            _ => {}
        }
    }

    fn open_palette(&mut self) {
        let saved = self.config.search.saved.clone();
        self.palette.open(&saved);
    }

    fn open_search(&mut self) {
        self.open_search_with(String::new());
    }

    fn open_search_with(&mut self, query: String) {
        let Some(index) = self.search_index.clone() else {
            self.toast_info("Library is still loading");
            return;
        };
        if index.is_empty() {
            self.toast_info("Library is empty");
            return;
        }
        tracing::debug!(items = index.len(), "opening search");
        self.search.open();
        self.search.query = query;
        self.request_search(index);
    }

    fn request_search(&mut self, index: Arc<SearchIndex>) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search.pending = true;
        self.search_handle
            .request(self.search_generation, index, self.search.query.clone());
    }

    fn refresh_search(&mut self) {
        if let Some(index) = self.search_index.clone() {
            self.request_search(index);
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl {
            match key.code {
                KeyCode::Char('u') => {
                    self.search.query.clear();
                    self.refresh_search();
                }
                KeyCode::Char('n') => self.search.move_cursor(1),
                KeyCode::Char('p') => self.search.move_cursor(-1),
                KeyCode::Char('a') => self.queue_all_search_results(false),
                KeyCode::Char('r') => self.queue_all_search_results(true),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc => self.search.close(),
            KeyCode::Enter => self.play_search_selection(),
            KeyCode::Up => self.search.move_cursor(-1),
            KeyCode::Down => self.search.move_cursor(1),
            KeyCode::PageUp => self.search.move_cursor(-10),
            KeyCode::PageDown => self.search.move_cursor(10),
            KeyCode::Backspace => {
                self.search.query.pop();
                self.refresh_search();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::ALT) => {
                self.search.query.push(character);
                self.refresh_search();
            }
            _ => {}
        }
    }

    /// Enqueues every URI represented by the current result set, deduplicated.
    fn queue_all_search_results(&mut self, replace: bool) {
        let uris = {
            let Some(index) = self.search_index.as_ref() else {
                return;
            };
            let mut seen = HashSet::new();
            let mut uris = Vec::new();
            for &position in &self.search.results {
                let Some(item) = index.items.get(position) else {
                    continue;
                };
                for uri in &item.uris {
                    if seen.insert(uri.clone()) {
                        uris.push(uri.to_string());
                    }
                }
            }
            uris
        };
        if uris.is_empty() {
            return;
        }
        let count = uris.len();
        if replace {
            self.mpd.send(Command::ReplaceAndPlay(uris));
            self.search.close();
            self.toast_info(format!("Playing {count} track(s)"));
        } else {
            self.mpd.send(Command::AppendUris(uris));
            self.toast_info(format!("Enqueued {count} track(s)"));
        }
    }

    fn play_search_selection(&mut self) {
        let uris = {
            let Some(index) = self.search_index.as_ref() else {
                return;
            };
            let Some(item) = self.search.selected(index) else {
                return;
            };
            item.uris
                .iter()
                .map(|uri| uri.to_string())
                .collect::<Vec<_>>()
        };
        if uris.is_empty() {
            return;
        }
        self.mpd.send(Command::AppendAndPlay(uris));
        self.search.close();
        self.toast_info("Playing search selection");
    }

    fn set_screen(&mut self, screen: Screen) {
        if self.screen == screen {
            return;
        }
        self.screen = screen;
        match screen {
            Screen::Playlists => {
                self.mpd.send(Command::RefreshPlaylists);
                self.ensure_playlist_songs();
            }
            Screen::Lyrics => self.request_lyrics(false),
            Screen::TagEditor => self.load_tag_editor(),
            Screen::History => self.load_history(),
            Screen::Browser if self.browser.entries.is_empty() && !self.browser.loading => {
                self.load_browser(self.browser.uri.clone());
            }
            Screen::Browser => {}
            Screen::Outputs => self.mpd.send(Command::RefreshOutputs),
            Screen::Stats => self.mpd.send(Command::RefreshStats),
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        match self.screen {
            Screen::Queue => self.queue_view.move_by(delta, self.queue.len()),
            Screen::Library => self.move_library(delta),
            Screen::Playlists => self.move_playlist_selection(delta),
            Screen::Outputs => self.outputs_view.move_by(delta, self.outputs.len()),
            Screen::Lyrics => self.scroll_lyrics(delta),
            Screen::Stats | Screen::Visualizer => {}
            Screen::History => {
                let len = self
                    .history
                    .as_ref()
                    .map_or(0, |report| report.top_songs.len());
                self.history_cursor = self
                    .history_cursor
                    .saturating_add_signed(delta)
                    .min(len.saturating_sub(1));
            }
            Screen::Browser => self.browser.view.move_by(delta, self.browser.entries.len()),
            Screen::TagEditor => {
                let len = self.tag_editor.fields.len();
                self.tag_editor.cursor = self
                    .tag_editor
                    .cursor
                    .saturating_add_signed(delta)
                    .min(len.saturating_sub(1));
            }
        }
    }

    fn page_selection(&mut self, direction: isize) {
        const PAGE: usize = 10;
        match self.screen {
            Screen::Queue => self.queue_view.page(direction, self.queue.len(), PAGE),
            Screen::Library => self.move_library(direction.saturating_mul(PAGE as isize)),
            Screen::Playlists => {
                self.move_playlist_selection(direction.saturating_mul(PAGE as isize))
            }
            Screen::Outputs => self.outputs_view.page(direction, self.outputs.len(), PAGE),
            Screen::Lyrics => {
                self.scroll_lyrics(direction.saturating_mul(PAGE as isize));
            }
            Screen::Stats | Screen::Visualizer => {}
            Screen::History => {
                let len = self
                    .history
                    .as_ref()
                    .map_or(0, |report| report.top_songs.len());
                self.history_cursor = self
                    .history_cursor
                    .saturating_add_signed(direction.saturating_mul(PAGE as isize))
                    .min(len.saturating_sub(1));
            }
            Screen::Browser => self
                .browser
                .view
                .page(direction, self.browser.entries.len(), PAGE),
            Screen::TagEditor => {
                let len = self.tag_editor.fields.len();
                self.tag_editor.cursor = self
                    .tag_editor
                    .cursor
                    .saturating_add_signed(direction.saturating_mul(PAGE as isize))
                    .min(len.saturating_sub(1));
            }
        }
    }

    fn jump_to_edge(&mut self, last: bool) {
        match self.screen {
            Screen::Queue => {
                if last {
                    self.queue_view.last(self.queue.len());
                } else {
                    self.queue_view.first();
                }
            }
            Screen::Library => match self.library_view.pane {
                LibraryPane::Artists => {
                    if last {
                        self.library_view.artists.last(self.library_artist_count());
                    } else {
                        self.library_view.artists.first();
                    }
                    self.library_view.albums.reset();
                    self.library_view.songs.reset();
                }
                LibraryPane::Albums => {
                    if last {
                        self.library_view.albums.last(self.library_album_count());
                    } else {
                        self.library_view.albums.first();
                    }
                    self.library_view.songs.reset();
                }
                LibraryPane::Songs => {
                    if last {
                        self.library_view.songs.last(self.library_song_count());
                    } else {
                        self.library_view.songs.first();
                    }
                }
            },
            Screen::Playlists => match self.playlist_pane {
                PlaylistPane::Playlists => {
                    if last {
                        self.playlists_view.last(self.playlists.len());
                    } else {
                        self.playlists_view.first();
                    }
                    self.ensure_playlist_songs();
                }
                PlaylistPane::Songs => {
                    if last {
                        self.playlist_songs_view.last(self.playlist_songs.len());
                    } else {
                        self.playlist_songs_view.first();
                    }
                }
            },
            Screen::Outputs => {
                if last {
                    self.outputs_view.last(self.outputs.len());
                } else {
                    self.outputs_view.first();
                }
            }
            Screen::Lyrics => {
                if last {
                    let lines = self
                        .lyrics
                        .doc
                        .as_ref()
                        .and_then(|doc| doc.plain.as_deref())
                        .map_or(0, |text| text.lines().count());
                    self.lyrics.scroll = lines.saturating_sub(1);
                } else {
                    self.lyrics.scroll = 0;
                }
            }
            Screen::Stats | Screen::Visualizer => {}
            Screen::History => {
                let len = self
                    .history
                    .as_ref()
                    .map_or(0, |report| report.top_songs.len());
                self.history_cursor = if last { len.saturating_sub(1) } else { 0 };
            }
            Screen::Browser => {
                if last {
                    self.browser.view.last(self.browser.entries.len());
                } else {
                    self.browser.view.first();
                }
            }
            Screen::TagEditor => {
                self.tag_editor.cursor = if last {
                    self.tag_editor.fields.len().saturating_sub(1)
                } else {
                    0
                };
            }
        }
    }

    fn move_library(&mut self, delta: isize) {
        match self.library_view.pane {
            LibraryPane::Artists => {
                let before = self.library_view.artists.cursor;
                self.library_view
                    .artists
                    .move_by(delta, self.library_artist_count());
                if self.library_view.artists.cursor != before {
                    self.library_view.albums.reset();
                    self.library_view.songs.reset();
                }
            }
            LibraryPane::Albums => {
                let before = self.library_view.albums.cursor;
                self.library_view
                    .albums
                    .move_by(delta, self.library_album_count());
                if self.library_view.albums.cursor != before {
                    self.library_view.songs.reset();
                }
            }
            LibraryPane::Songs => {
                self.library_view
                    .songs
                    .move_by(delta, self.library_song_count());
            }
        }
    }

    fn change_pane(&mut self, delta: isize) {
        if self.screen == Screen::Playlists {
            self.playlist_pane = match self.playlist_pane {
                PlaylistPane::Playlists => PlaylistPane::Songs,
                PlaylistPane::Songs => PlaylistPane::Playlists,
            };
            return;
        }
        let index = (self.library_view.pane as isize + delta).rem_euclid(3);
        self.library_view.pane = match index {
            0 => LibraryPane::Artists,
            1 => LibraryPane::Albums,
            _ => LibraryPane::Songs,
        };
    }

    fn toggle_play(&mut self) {
        let playing = self
            .status
            .as_ref()
            .is_some_and(|status| status.state.is_playing());
        if playing {
            let now = self.effective_elapsed();
            self.elapsed_at_sync = now;
            self.elapsed_synced_at = Instant::now();
            self.mpd.send(Command::Pause(true));
            if let Some(status) = &mut self.status {
                status.state = PlaybackState::Paused;
            }
        } else {
            self.mpd.send(Command::Play);
            if let Some(status) = &mut self.status {
                status.state = PlaybackState::Playing;
            }
        }
        self.publish_mpris();
    }

    fn change_volume(&mut self, delta: i16) {
        let current = self
            .status
            .as_ref()
            .and_then(|status| status.volume)
            .unwrap_or(100) as i16;
        let volume = (current + delta).clamp(0, 100) as u8;
        self.mpd.send(Command::SetVolume(volume));
        if let Some(status) = &mut self.status {
            status.volume = Some(volume);
        }
        self.publish_mpris();
    }

    fn toggle_option(&mut self, option: PlayerOption) {
        let enabled = self.status.as_ref().is_some_and(|status| match option {
            PlayerOption::Repeat => status.repeat,
            PlayerOption::Random => status.random,
            PlayerOption::Single => status.single,
            PlayerOption::Consume => status.consume,
        });
        let enabled = !enabled;
        self.mpd.send(Command::SetOption(option, enabled));
        if let Some(status) = &mut self.status {
            match option {
                PlayerOption::Repeat => status.repeat = enabled,
                PlayerOption::Random => status.random = enabled,
                PlayerOption::Single => status.single = enabled,
                PlayerOption::Consume => status.consume = enabled,
            }
        }
        self.publish_mpris();
    }

    fn play_selected(&mut self) {
        match self.screen {
            Screen::Queue => {
                if let Some(song) = self.queue.get(self.queue_view.cursor)
                    && let Some(position) = song.pos
                {
                    self.mpd.send(Command::PlayPos(position));
                }
            }
            Screen::Library => {
                let uris = self.selected_uris();
                if !uris.is_empty() {
                    self.mpd.send(Command::AppendAndPlay(uris));
                }
            }
            Screen::Playlists => self.play_playlist_selection(),
            Screen::Browser => self.browser_enter(),
            Screen::Outputs => self.toggle_output(),
            Screen::History => {
                if let Some(report) = &self.history
                    && let Some(song) = report.top_songs.get(self.history_cursor)
                {
                    self.mpd
                        .send(Command::AppendAndPlay(vec![song.uri.clone()]));
                }
            }
            Screen::Stats | Screen::Lyrics | Screen::Visualizer | Screen::TagEditor => {}
        }
    }

    fn play_playlist_selection(&mut self) {
        match self.playlist_pane {
            PlaylistPane::Playlists => {
                if let Some(name) = self.selected_playlist_name() {
                    self.mpd.send(Command::LoadPlaylist {
                        name,
                        replace: true,
                    });
                }
            }
            PlaylistPane::Songs => {
                if let Some(song) = self.playlist_songs.get(self.playlist_songs_view.cursor) {
                    self.mpd
                        .send(Command::AppendAndPlay(vec![song.uri.to_string()]));
                }
            }
        }
    }

    fn enqueue_selected(&mut self) {
        if self.screen == Screen::Library {
            let uris = self.selected_uris();
            if !uris.is_empty() {
                self.toast_info(format!("Enqueued {} track(s)", uris.len()));
                self.mpd.send(Command::AppendUris(uris));
            }
        } else if self.screen == Screen::Playlists {
            self.enqueue_playlist_selection();
        } else if self.screen == Screen::Browser {
            self.browser_enqueue();
        }
    }

    fn enqueue_playlist_selection(&mut self) {
        match self.playlist_pane {
            PlaylistPane::Playlists => {
                if let Some(name) = self.selected_playlist_name() {
                    self.toast_info(format!("Enqueued playlist \"{name}\""));
                    self.mpd.send(Command::AppendPlaylist(name));
                }
            }
            PlaylistPane::Songs => {
                if let Some(song) = self.playlist_songs.get(self.playlist_songs_view.cursor) {
                    self.mpd
                        .send(Command::AppendUris(vec![song.uri.to_string()]));
                }
            }
        }
    }

    fn replace_and_play(&mut self) {
        if self.screen == Screen::Browser {
            let Some(entry) = self.browser.entries.get(self.browser.view.cursor) else {
                return;
            };
            match entry.kind {
                EntryKind::Playlist => self.mpd.send(Command::LoadPlaylist {
                    name: entry.uri.to_string(),
                    replace: true,
                }),
                EntryKind::Directory | EntryKind::File => self
                    .mpd
                    .send(Command::ReplaceWithUri(entry.uri.to_string())),
            }
            return;
        }
        if self.screen != Screen::Library {
            return;
        }
        let uris = self.selected_uris();
        if !uris.is_empty() {
            self.mpd.send(Command::ReplaceAndPlay(uris));
        }
    }

    fn selected_uris(&self) -> Vec<String> {
        let Some(library) = &self.library else {
            return Vec::new();
        };
        match self.library_view.pane {
            LibraryPane::Artists => self
                .selected_artist()
                .map(|artist| {
                    artist
                        .albums
                        .iter()
                        .flat_map(|album| album.songs.iter())
                        .filter_map(|&index| library.song(index))
                        .map(|song| song.uri.to_string())
                        .collect()
                })
                .unwrap_or_default(),
            LibraryPane::Albums => self
                .selected_album()
                .map(|album| {
                    album
                        .songs
                        .iter()
                        .filter_map(|&index| library.song(index))
                        .map(|song| song.uri.to_string())
                        .collect()
                })
                .unwrap_or_default(),
            LibraryPane::Songs => self
                .selected_library_song()
                .map(|song| vec![song.uri.to_string()])
                .unwrap_or_default(),
        }
    }

    fn remove_selected(&mut self) {
        if self.screen == Screen::Queue {
            if let Some(song) = self.queue.get(self.queue_view.cursor)
                && let Some(id) = song.id
            {
                self.mpd.send(Command::RemoveId(id));
            }
            return;
        }

        if self.screen == Screen::Playlists
            && self.playlist_pane == PlaylistPane::Songs
            && self.playlist_songs_view.cursor < self.playlist_songs.len()
            && let Some(name) = self.selected_playlist_name()
        {
            self.mpd.send(Command::PlaylistRemoveSong {
                name,
                position: self.playlist_songs_view.cursor as u32,
            });
        }
    }

    fn move_selected_item(&mut self, delta: isize) {
        match self.screen {
            Screen::Queue => self.move_queue_selection(delta),
            Screen::Playlists if self.playlist_pane == PlaylistPane::Songs => {
                self.move_playlist_song(delta);
            }
            _ => {}
        }
    }

    fn move_playlist_song(&mut self, delta: isize) {
        let from = self.playlist_songs_view.cursor;
        let to = from.saturating_add_signed(delta);
        if to == from || to >= self.playlist_songs.len() {
            return;
        }
        let Some(name) = self.selected_playlist_name() else {
            return;
        };
        self.playlist_songs.swap(from, to);
        self.playlist_songs_view.cursor = to;
        self.mpd.send(Command::PlaylistMoveSong {
            name,
            from: from as u32,
            to: to as u32,
        });
    }

    fn move_playlist_selection(&mut self, delta: isize) {
        match self.playlist_pane {
            PlaylistPane::Playlists => {
                let before = self.playlists_view.cursor;
                self.playlists_view.move_by(delta, self.playlists.len());
                if self.playlists_view.cursor != before {
                    self.ensure_playlist_songs();
                }
            }
            PlaylistPane::Songs => {
                self.playlist_songs_view
                    .move_by(delta, self.playlist_songs.len());
            }
        }
    }

    fn move_queue_selection(&mut self, delta: isize) {
        if self.screen != Screen::Queue {
            return;
        }
        let from = self.queue_view.cursor;
        let to = from.saturating_add_signed(delta);
        if to == from || to >= self.queue.len() {
            return;
        }
        let Some(song) = self.queue.get(from) else {
            return;
        };
        let Some(id) = song.id else {
            return;
        };
        self.queue.swap(from, to);
        self.queue_view.cursor = to;
        self.mpd.send(Command::MoveId { id, to: to as u32 });
    }

    pub fn selected_playlist_name(&self) -> Option<String> {
        self.playlists
            .get(self.playlists_view.cursor)
            .map(|playlist| playlist.name.to_string())
    }

    fn ensure_playlist_songs(&mut self) {
        if self.screen != Screen::Playlists {
            return;
        }
        match self.selected_playlist_name() {
            Some(name) => {
                if self.loaded_playlist.as_deref() != Some(name.as_str()) {
                    self.loaded_playlist = Some(name.clone());
                    self.playlist_songs.clear();
                    self.playlist_songs_view.reset();
                    self.mpd.send(Command::RefreshPlaylistSongs(name));
                }
            }
            None => {
                self.loaded_playlist = None;
                self.playlist_songs.clear();
                self.playlist_songs_view.reset();
            }
        }
    }

    fn open_save_playlist(&mut self) {
        if self.queue.is_empty() {
            self.toast_info("Queue is empty");
            return;
        }
        self.input = Some(InputModal {
            prompt: "Save queue as playlist".into(),
            value: String::new(),
            action: InputAction::SaveQueueAsPlaylist,
        });
    }

    fn open_rename_playlist(&mut self) {
        let Some(name) = self.selected_playlist_name() else {
            return;
        };
        self.input = Some(InputModal {
            prompt: "Rename playlist".into(),
            value: name.clone(),
            action: InputAction::RenamePlaylist { from: name },
        });
    }

    fn request_delete_playlist(&mut self) {
        let Some(name) = self.selected_playlist_name() else {
            return;
        };
        self.confirm = Some(Confirm {
            message: format!("Delete playlist \"{name}\"?"),
            action: ConfirmAction::DeletePlaylist(name),
        });
    }

    fn handle_input_key(&mut self, key: KeyEvent) {
        let Some(mut modal) = self.input.take() else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                let value = modal.value.trim().to_string();
                let action = modal.action.clone();
                match action {
                    InputAction::SaveQueueAsPlaylist => {
                        if value.is_empty() {
                            self.input = Some(modal);
                            return;
                        }
                        self.mpd.send(Command::SaveQueueAs(value.clone()));
                        self.toast_info(format!("Saved playlist \"{value}\""));
                    }
                    InputAction::RenamePlaylist { from } => {
                        if value.is_empty() {
                            self.input = Some(modal);
                            return;
                        }
                        if from != value {
                            self.mpd.send(Command::RenamePlaylist {
                                from,
                                to: value.clone(),
                            });
                            self.toast_info(format!("Renamed playlist to \"{value}\""));
                        }
                    }
                    InputAction::EditTagField { index } => {
                        if let Some(field) = self.tag_editor.fields.get_mut(index) {
                            field.value = value;
                            self.tag_editor.dirty = true;
                            self.tag_editor.message = None;
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                modal.value.pop();
                self.input = Some(modal);
            }
            KeyCode::Char('u') if ctrl => {
                modal.value.clear();
                self.input = Some(modal);
            }
            KeyCode::Char(character) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                modal.value.push(character);
                self.input = Some(modal);
            }
            _ => self.input = Some(modal),
        }
    }

    fn request_clear_queue(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        self.confirm = Some(Confirm {
            message: "Clear the current queue?".into(),
            action: ConfirmAction::ClearQueue,
        });
    }

    fn resolve_confirm(&mut self) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        match confirm.action {
            ConfirmAction::ClearQueue => {
                self.mpd.send(Command::Clear);
                self.toast_info("Queue cleared");
            }
            ConfirmAction::DeletePlaylist(name) => {
                self.toast_info(format!("Deleted playlist \"{name}\""));
                self.mpd.send(Command::DeletePlaylist(name));
            }
            ConfirmAction::SaveTags => self.save_tags(),
        }
    }

    fn toggle_output(&mut self) {
        let Some(output) = self.outputs.get(self.outputs_view.cursor) else {
            return;
        };
        let (id, enabled) = (output.id, output.enabled);
        if let Some(output) = self.outputs.get_mut(self.outputs_view.cursor) {
            output.enabled = !enabled;
        }
        self.mpd.send(if enabled {
            Command::DisableOutput(id)
        } else {
            Command::EnableOutput(id)
        });
    }

    pub fn effective_elapsed(&self) -> Duration {
        let Some(status) = &self.status else {
            return Duration::ZERO;
        };
        if status.state.is_playing() {
            let elapsed = self.elapsed_at_sync + self.elapsed_synced_at.elapsed();
            status
                .duration
                .map_or(elapsed, |duration| elapsed.min(duration))
        } else {
            self.elapsed_at_sync
        }
    }

    pub fn is_current(&self, song: &Song) -> bool {
        let Some(current) = &self.current else {
            return false;
        };
        match (song.id, current.id) {
            (Some(id), Some(current_id)) => id == current_id,
            _ => song.uri == current.uri,
        }
    }

    pub fn selected_artist(&self) -> Option<&Artist> {
        self.library
            .as_ref()?
            .artists()
            .get(self.library_view.artists.cursor)
    }

    pub fn selected_album(&self) -> Option<&Album> {
        self.selected_artist()?
            .albums
            .get(self.library_view.albums.cursor)
    }

    pub fn selected_library_song(&self) -> Option<&Song> {
        let library = self.library.as_ref()?;
        let album = self.selected_album()?;
        album
            .songs
            .get(self.library_view.songs.cursor)
            .and_then(|&index| library.song(index))
    }

    pub fn library_artist_count(&self) -> usize {
        self.library
            .as_ref()
            .map_or(0, |library| library.artists().len())
    }

    pub fn library_album_count(&self) -> usize {
        self.selected_artist()
            .map_or(0, |artist| artist.albums.len())
    }

    pub fn library_song_count(&self) -> usize {
        self.selected_album().map_or(0, |album| album.songs.len())
    }

    fn clamp_library_views(&mut self) {
        self.library_view.artists.clamp(self.library_artist_count());
        self.library_view.albums.clamp(self.library_album_count());
        self.library_view.songs.clamp(self.library_song_count());
    }

    fn reload_config(&mut self) {
        let Some(path) = self.config_path.clone() else {
            return;
        };
        match Config::load(Some(&path)) {
            Ok(config) => {
                let connection_changed = config.mpd_addr() != self.config.mpd_addr()
                    || config.mpd.password != self.config.mpd.password;
                ui::apply_theme(&config.theme);
                self.keymap = Keymap::from_config(&config.keys);
                self.config = config;
                if connection_changed {
                    self.toast_info("Config reloaded — restart to change MPD connection");
                } else {
                    self.toast_success("Configuration reloaded");
                }
            }
            Err(error) => self.toast_error(format!("Config reload failed: {error}")),
        }
    }

    fn toast_info(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast {
            message: message.into(),
            level: ToastLevel::Info,
            created: Instant::now(),
        });
    }

    fn toast_success(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast {
            message: message.into(),
            level: ToastLevel::Success,
            created: Instant::now(),
        });
    }

    fn toast_error(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast {
            message: message.into(),
            level: ToastLevel::Error,
            created: Instant::now(),
        });
    }
}

fn spawn_config_watcher(path: PathBuf, tx: mpsc::UnboundedSender<Msg>) {
    std::thread::spawn(move || {
        use notify::{RecursiveMode, Watcher};
        let path = std::path::absolute(&path).unwrap_or(path);
        let Some(directory) = path.parent() else {
            return;
        };
        if let Err(error) = std::fs::create_dir_all(directory) {
            tracing::warn!(%error, "could not create config directory");
            return;
        }
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |result| {
            let _ = event_tx.send(result);
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                tracing::warn!(%error, "could not create config watcher");
                return;
            }
        };
        if let Err(error) = watcher.watch(directory, RecursiveMode::NonRecursive) {
            tracing::warn!(
                %error,
                directory = %directory.display(),
                "could not watch config directory"
            );
            return;
        }
        for result in event_rx {
            let Ok(event) = result else {
                continue;
            };
            if event.paths.iter().any(|candidate| candidate == &path)
                && tx.send(Msg::ConfigFileChanged).is_err()
            {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        let mut config = Config::default();
        config.stats.enabled = false;
        config
    }

    fn app() -> App {
        App::new(
            test_config(),
            AppDeps {
                mpd: MpdHandle::test_handle(),
                search_handle: SearchHandle::test_handle(),
                picker: Picker::halfblocks(),
                mpris_tx: watch::channel(MprisSnapshot::default()).0,
                scrobbler: None,
                event_tx: mpsc::unbounded_channel().0,
                config_path: None,
            },
        )
    }

    fn app_with_commands() -> (App, mpsc::UnboundedReceiver<Command>) {
        let (mpd, receiver) = MpdHandle::test_channel();
        (
            App::new(
                test_config(),
                AppDeps {
                    mpd,
                    search_handle: SearchHandle::test_handle(),
                    picker: Picker::halfblocks(),
                    mpris_tx: watch::channel(MprisSnapshot::default()).0,
                    scrobbler: None,
                    event_tx: mpsc::unbounded_channel().0,
                    config_path: None,
                },
            ),
            receiver,
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn test_index() -> Arc<SearchIndex> {
        let library = Library::build(vec![
            Song {
                uri: Arc::from("a/one.flac"),
                artist: Some(Arc::from("Portishead")),
                album: Some(Arc::from("Dummy")),
                title: Some(Arc::from("Sour Times")),
                ..Song::default()
            },
            Song {
                uri: Arc::from("a/two.flac"),
                artist: Some(Arc::from("Portishead")),
                album: Some(Arc::from("Dummy")),
                title: Some(Arc::from("Mysterons")),
                ..Song::default()
            },
        ]);
        Arc::new(SearchIndex::build(&library))
    }

    #[test]
    fn search_can_enqueue_or_play_all_results() {
        let (mut app, mut commands) = app_with_commands();
        app.search_index = Some(test_index());
        app.search.active = true;
        app.search.query = "artist:portishead".into();
        app.search.update(app.search_index.as_ref().expect("index"));

        app.on_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        match commands.try_recv() {
            Ok(Command::AppendUris(mut uris)) => {
                uris.sort_unstable();
                assert_eq!(uris, vec!["a/one.flac", "a/two.flac"]);
            }
            other => panic!("expected AppendUris, got {other:?}"),
        }
        assert!(app.search.active);

        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        match commands.try_recv() {
            Ok(Command::ReplaceAndPlay(mut uris)) => {
                uris.sort_unstable();
                assert_eq!(uris, vec!["a/one.flac", "a/two.flac"]);
            }
            other => panic!("expected ReplaceAndPlay, got {other:?}"),
        }
        assert!(!app.search.active);
    }

    #[test]
    fn palette_opens_saved_smart_playlists() {
        let (mut app, _commands) = app_with_commands();
        app.search_index = Some(test_index());
        app.config
            .search
            .saved
            .insert("90s".into(), "year:1990-1999".into());

        app.on_key(key(KeyCode::Char(':')));
        assert!(app.palette.active);
        for character in "smart".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));

        assert!(!app.palette.active);
        assert!(app.search.active);
        assert_eq!(app.search.query, "year:1990-1999");
    }

    #[test]
    fn screens_cycle_both_ways() {
        assert_eq!(Screen::Queue.next(), Screen::Library);
        assert_eq!(Screen::Queue.previous(), Screen::Browser);
        assert_eq!(Screen::Stats.next(), Screen::Lyrics);
        assert_eq!(Screen::Lyrics.next(), Screen::Visualizer);
        assert_eq!(Screen::Visualizer.next(), Screen::TagEditor);
        assert_eq!(Screen::TagEditor.next(), Screen::History);
        assert_eq!(Screen::History.next(), Screen::Browser);
        assert_eq!(Screen::Browser.next(), Screen::Queue);
        assert_eq!(Screen::from_name("LIBRARY"), Screen::Library);
        assert_eq!(Screen::from_name("nonsense"), Screen::Queue);
    }

    #[test]
    fn elapsed_interpolates_while_playing() {
        let mut app = app();
        app.status = Some(MpdStatus {
            state: PlaybackState::Playing,
            duration: Some(Duration::from_secs(100)),
            ..MpdStatus::default()
        });
        app.elapsed_at_sync = Duration::from_secs(10);
        app.elapsed_synced_at = Instant::now() - Duration::from_secs(3);
        let elapsed = app.effective_elapsed();
        assert!(elapsed >= Duration::from_secs(13));
        assert!(elapsed <= Duration::from_secs(14));
    }

    #[test]
    fn elapsed_freezes_when_paused() {
        let mut app = app();
        app.status = Some(MpdStatus {
            state: PlaybackState::Paused,
            ..MpdStatus::default()
        });
        app.elapsed_at_sync = Duration::from_secs(42);
        app.elapsed_synced_at = Instant::now() - Duration::from_secs(10);
        assert_eq!(app.effective_elapsed(), Duration::from_secs(42));
    }

    #[test]
    fn space_sends_pause_when_playing() {
        let (mut app, mut commands) = app_with_commands();
        app.status = Some(MpdStatus {
            state: PlaybackState::Playing,
            ..MpdStatus::default()
        });
        app.on_key(key(KeyCode::Char(' ')));
        assert!(matches!(commands.try_recv(), Ok(Command::Pause(true))));
    }

    #[test]
    fn space_sends_play_when_paused() {
        let (mut app, mut commands) = app_with_commands();
        app.status = Some(MpdStatus {
            state: PlaybackState::Paused,
            ..MpdStatus::default()
        });
        app.on_key(key(KeyCode::Char(' ')));
        assert!(matches!(commands.try_recv(), Ok(Command::Play)));
    }

    #[test]
    fn volume_keys_send_the_absolute_new_volume() {
        let (mut app, mut commands) = app_with_commands();
        app.status = Some(MpdStatus {
            volume: Some(60),
            ..MpdStatus::default()
        });
        app.on_key(key(KeyCode::Char('+')));
        assert!(matches!(commands.try_recv(), Ok(Command::SetVolume(65))));
        app.on_key(key(KeyCode::Char('-')));
        assert!(matches!(commands.try_recv(), Ok(Command::SetVolume(60))));
    }

    #[test]
    fn deleting_a_queue_row_sends_its_id() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Queue;
        app.queue = vec![Song {
            id: Some(7),
            uri: Arc::from("a/one.flac"),
            ..Song::default()
        }];
        app.queue_view.cursor = 0;
        app.on_key(key(KeyCode::Char('d')));
        assert!(matches!(commands.try_recv(), Ok(Command::RemoveId(7))));
    }

    #[test]
    fn playing_a_library_artist_enqueues_every_track() {
        let (mut app, mut commands) = app_with_commands();
        let library = Library::build(vec![
            Song {
                uri: Arc::from("a/one.flac"),
                artist: Some(Arc::from("Portishead")),
                album: Some(Arc::from("Dummy")),
                track: Some(1),
                ..Song::default()
            },
            Song {
                uri: Arc::from("a/two.flac"),
                artist: Some(Arc::from("Portishead")),
                album: Some(Arc::from("Dummy")),
                track: Some(2),
                ..Song::default()
            },
        ]);
        app.library = Some(Arc::new(library));
        app.screen = Screen::Library;
        app.on_key(key(KeyCode::Enter));
        match commands.try_recv() {
            Ok(Command::AppendAndPlay(uris)) => {
                assert_eq!(uris, vec!["a/one.flac", "a/two.flac"]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn help_swallows_the_next_key() {
        let (mut app, mut commands) = app_with_commands();
        app.on_key(key(KeyCode::Char('?')));
        assert!(app.help);
        app.on_key(key(KeyCode::Char('p')));
        assert!(!app.help);
        assert!(commands.try_recv().is_err(), "help must not play/pause");
    }

    #[test]
    fn enter_on_a_playlist_loads_and_plays_it() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Playlists;
        app.playlists = vec![Playlist {
            name: Arc::from("Late Night"),
            last_modified: None,
        }];
        app.playlists_view.cursor = 0;
        app.on_key(key(KeyCode::Enter));
        match commands.try_recv() {
            Ok(Command::LoadPlaylist { name, replace }) => {
                assert_eq!(name, "Late Night");
                assert!(replace);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn save_queue_as_playlist_uses_the_input_modal() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Playlists;
        app.queue = vec![Song {
            id: Some(1),
            uri: Arc::from("a/one.flac"),
            ..Song::default()
        }];
        app.on_key(key(KeyCode::Char('S')));
        assert!(app.input.is_some(), "saving must prompt for a name");
        for character in "Chill".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        match commands.try_recv() {
            Ok(Command::SaveQueueAs(name)) => assert_eq!(name, "Chill"),
            other => panic!("unexpected command: {other:?}"),
        }
        assert!(app.input.is_none());
    }

    #[test]
    fn deleting_a_playlist_requires_confirmation() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Playlists;
        app.playlists = vec![Playlist {
            name: Arc::from("Mix"),
            last_modified: None,
        }];
        app.playlists_view.cursor = 0;
        app.on_key(key(KeyCode::Char('x')));
        assert!(app.confirm.is_some());
        assert!(commands.try_recv().is_err());
        app.on_key(key(KeyCode::Char('y')));
        match commands.try_recv() {
            Ok(Command::DeletePlaylist(name)) => assert_eq!(name, "Mix"),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn removing_a_playlist_song_sends_its_position() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Playlists;
        app.playlist_pane = PlaylistPane::Songs;
        app.playlists = vec![Playlist {
            name: Arc::from("Mix"),
            last_modified: None,
        }];
        app.playlists_view.cursor = 0;
        app.playlist_songs = vec![Song {
            uri: Arc::from("a/one.flac"),
            ..Song::default()
        }];
        app.on_key(key(KeyCode::Char('d')));
        match commands.try_recv() {
            Ok(Command::PlaylistRemoveSong { name, position }) => {
                assert_eq!(name, "Mix");
                assert_eq!(position, 0);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn reordering_playlist_songs_updates_locally_and_on_mpd() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Playlists;
        app.playlist_pane = PlaylistPane::Songs;
        app.playlists = vec![Playlist {
            name: Arc::from("Mix"),
            last_modified: None,
        }];
        app.playlists_view.cursor = 0;
        app.playlist_songs = vec![
            Song {
                uri: Arc::from("a/one.flac"),
                ..Song::default()
            },
            Song {
                uri: Arc::from("a/two.flac"),
                ..Song::default()
            },
        ];
        app.on_key(key(KeyCode::Char('J')));
        assert_eq!(app.playlist_songs[0].uri.as_ref(), "a/two.flac");
        assert_eq!(app.playlist_songs_view.cursor, 1);
        match commands.try_recv() {
            Ok(Command::PlaylistMoveSong { name, from, to }) => {
                assert_eq!(name, "Mix");
                assert_eq!((from, to), (0, 1));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn requests_cover_art_for_the_current_song_once() {
        let (mut app, mut commands) = app_with_commands();
        app.current = Some(Song {
            uri: Arc::from("a/one.flac"),
            ..Song::default()
        });
        app.request_cover();
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::FetchCover(uri)) if uri == "a/one.flac"
        ));
        app.request_cover();
        assert!(
            commands.try_recv().is_err(),
            "must not refetch the same song"
        );
    }

    #[test]
    fn decodes_embedded_cover_art_into_a_protocol() {
        let mut app = app();
        let image = image::RgbImage::from_pixel(2, 2, image::Rgb([200, 50, 50]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        app.set_cover(Some(Arc::new(Picture {
            mime: Arc::from("image/png"),
            data: bytes.into_inner(),
        })));
        assert!(app.cover_protocol.is_some());
        app.set_cover(None);
        assert!(app.cover_protocol.is_none());
    }

    #[test]
    fn cover_sidebar_paints_halfblocks() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = app();
        app.current = Some(Song {
            uri: Arc::from("a/one.flac"),
            title: Some(Arc::from("A Track")),
            artist: Some(Arc::from("An Artist")),
            album: Some(Arc::from("An Album")),
            ..Song::default()
        });
        let image = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 50, 50]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        app.set_cover(Some(Arc::new(Picture {
            mime: Arc::from("image/png"),
            data: bytes.into_inner(),
        })));

        let backend = TestBackend::new(150, 42);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(&mut app, frame))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Now Playing"));
        assert!(text.contains('▀'), "halfblock cover art should be painted");
    }

    #[test]
    fn online_cover_replaces_missing_artwork() {
        let mut app = app();
        app.current = Some(Song {
            uri: Arc::from("a/one.flac"),
            title: Some(Arc::from("A Track")),
            artist: Some(Arc::from("An Artist")),
            album: Some(Arc::from("An Album")),
            ..Song::default()
        });
        let image = image::RgbImage::from_pixel(8, 8, image::Rgb([20, 120, 200]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();

        app.remote_cover_inflight.insert(Arc::from("a/one.flac"));
        app.on_msg(Msg::OnlineCover {
            uri: "a/one.flac".into(),
            picture: Some(Picture {
                mime: Arc::from("image/png"),
                data: bytes.into_inner(),
            }),
        });

        assert!(app.remote_cover_inflight.is_empty());
        assert!(
            app.cover_cache
                .get("a/one.flac")
                .is_some_and(Option::is_some)
        );
        assert!(app.cover_protocol.is_some());
    }

    #[test]
    fn requests_and_caches_lyrics() {
        use crate::lyrics::{LyricLine, LyricsSource};

        let (mut app, mut commands) = app_with_commands();
        app.current = Some(Song {
            uri: Arc::from("a/one.flac"),
            artist: Some(Arc::from("Portishead")),
            album: Some(Arc::from("Dummy")),
            title: Some(Arc::from("Mysterons")),
            duration: Some(Duration::from_secs(206)),
            ..Song::default()
        });
        app.request_lyrics(false);
        match commands.try_recv() {
            Ok(Command::FetchLyrics {
                uri,
                artist,
                title,
                album,
                duration_seconds,
                ..
            }) => {
                assert_eq!(uri, "a/one.flac");
                assert_eq!(artist, "Portishead");
                assert_eq!(title, "Mysterons");
                assert_eq!(album, "Dummy");
                assert_eq!(duration_seconds, Some(206));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let doc = LyricsDoc {
            source: Some(LyricsSource::Lrclib),
            synced: vec![LyricLine {
                at: Duration::from_secs(12),
                text: "Hello".into(),
            }],
            plain: None,
        };
        app.on_msg(Msg::Lyrics {
            uri: "a/one.flac".into(),
            doc: Some(doc),
        });
        assert!(app.lyrics.doc.is_some());
        assert!(!app.lyrics.loading);

        app.request_lyrics(false);
        assert!(
            commands.try_recv().is_err(),
            "the second request should hit the cache"
        );
    }

    #[test]
    fn lyrics_offset_scales_with_brackets() {
        let (mut app, mut _commands) = app_with_commands();
        app.screen = Screen::Lyrics;
        app.on_key(key(KeyCode::Char(']')));
        app.on_key(key(KeyCode::Char(']')));
        assert_eq!(app.lyrics.offset_ms, 1000);
        app.on_key(key(KeyCode::Char('[')));
        assert_eq!(app.lyrics.offset_ms, 500);
        assert_eq!(
            app.lyrics.adjusted_position(Duration::from_secs(10)),
            Duration::from_millis(10_500)
        );
    }

    #[test]
    fn lyrics_screen_renders_the_active_line() {
        use crate::lyrics::{LyricLine, LyricsSource};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = app();
        app.screen = Screen::Lyrics;
        app.current = Some(Song {
            uri: Arc::from("a/one.flac"),
            title: Some(Arc::from("Track")),
            artist: Some(Arc::from("Artist")),
            duration: Some(Duration::from_secs(100)),
            ..Song::default()
        });
        app.status = Some(MpdStatus {
            state: PlaybackState::Playing,
            elapsed: Duration::from_secs(25),
            duration: Some(Duration::from_secs(100)),
            ..MpdStatus::default()
        });
        app.elapsed_at_sync = Duration::from_secs(25);
        app.elapsed_synced_at = Instant::now();
        app.lyrics.doc = Some(LyricsDoc {
            source: Some(LyricsSource::Lrclib),
            synced: vec![
                LyricLine {
                    at: Duration::from_secs(10),
                    text: "first line".into(),
                },
                LyricLine {
                    at: Duration::from_secs(20),
                    text: "active line".into(),
                },
                LyricLine {
                    at: Duration::from_secs(30),
                    text: "future line".into(),
                },
            ],
            plain: None,
        });

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(&mut app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Lyrics"));
        assert!(text.contains("active line"));
        assert!(text.contains("future line"));
    }

    #[test]
    fn visualizer_screen_paints_bars() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = app();
        app.screen = Screen::Visualizer;
        app.config.visualizer.enabled = true;
        app.spectrum = vec![0.5; 64];
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(&mut app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains('█'), "spectrum bars should be painted");
    }

    #[test]
    fn tag_editor_loads_and_parses_fields() {
        let song = Song {
            uri: Arc::from("a/one.flac"),
            title: Some(Arc::from("Title")),
            artist: Some(Arc::from("Artist")),
            album: Some(Arc::from("Album")),
            album_artist: Some(Arc::from("Album Artist")),
            track: Some(3),
            disc: Some(1),
            date: Some(Arc::from("2026")),
            genre: Some(Arc::from("Test")),
            tags: vec![(Arc::from("Comment"), Arc::from("hello"))],
            ..Song::default()
        };
        let editor = TagEditorState::from_song(&song);
        assert_eq!(editor.value(TagKey::Title), "Title");
        assert_eq!(editor.value(TagKey::AlbumArtist), "Album Artist");
        assert_eq!(editor.value(TagKey::Comment), "hello");

        let update = editor.to_update().unwrap();
        assert_eq!(update.track, Some(3));
        assert_eq!(update.disc, Some(1));
        assert_eq!(update.date, "2026");

        let mut invalid = editor;
        invalid.fields[6].value = "abc".into();
        assert!(invalid.to_update().is_err());
    }

    #[test]
    fn writing_tags_requires_confirmation_and_sends_the_file_path() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::TagEditor;
        app.config.library.music_directory = Some(std::path::PathBuf::from("/music"));
        app.current = Some(Song {
            uri: Arc::from("a/one.flac"),
            title: Some(Arc::from("Old")),
            ..Song::default()
        });
        app.load_tag_editor();
        app.tag_editor.fields[0].value = "New".into();
        app.tag_editor.dirty = true;

        app.on_key(key(KeyCode::Char('w')));
        assert!(app.confirm.is_some());
        assert!(commands.try_recv().is_err());

        app.on_key(key(KeyCode::Char('y')));
        match commands.try_recv() {
            Ok(Command::SaveTags { path, uri, update }) => {
                assert_eq!(path, std::path::PathBuf::from("/music/a/one.flac"));
                assert_eq!(uri, "a/one.flac");
                assert_eq!(update.title, "New");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn saved_tags_clear_the_dirty_flag() {
        let mut app = app();
        app.tag_editor.dirty = true;
        app.on_msg(Msg::TagsSaved {
            uri: "a/one.flac".into(),
            backup: Some(std::path::PathBuf::from("/backup")),
            error: None,
        });
        assert!(!app.tag_editor.dirty);
    }

    #[test]
    fn scrobbles_now_playing_and_after_the_threshold() {
        let (scrobbler, mut events) = ScrobbleHandle::test_channel();
        let (mpd, _commands) = MpdHandle::test_channel();
        let mut app = App::new(
            test_config(),
            AppDeps {
                mpd,
                search_handle: SearchHandle::test_handle(),
                picker: Picker::halfblocks(),
                mpris_tx: watch::channel(MprisSnapshot::default()).0,
                scrobbler: Some(scrobbler),
                event_tx: mpsc::unbounded_channel().0,
                config_path: None,
            },
        );
        app.status = Some(MpdStatus {
            state: PlaybackState::Playing,
            ..MpdStatus::default()
        });
        app.on_msg(Msg::CurrentSong(Some(Song {
            id: Some(1),
            uri: Arc::from("a/one.flac"),
            artist: Some(Arc::from("Artist")),
            title: Some(Arc::from("Title")),
            album: Some(Arc::from("Album")),
            duration: Some(Duration::from_secs(100)),
            ..Song::default()
        })));
        match events.try_recv() {
            Ok(event) => {
                assert_eq!(event.kind, ScrobbleKind::NowPlaying);
                assert_eq!(event.track.title, "Title");
            }
            Err(error) => panic!("missing now-playing event: {error}"),
        }

        app.on_msg(Msg::Status(MpdStatus {
            state: PlaybackState::Playing,
            song_id: Some(1),
            elapsed: Duration::from_secs(60),
            duration: Some(Duration::from_secs(100)),
            ..MpdStatus::default()
        }));
        match events.try_recv() {
            Ok(event) => assert_eq!(event.kind, ScrobbleKind::Single),
            Err(error) => panic!("missing scrobble event: {error}"),
        }
        assert!(events.try_recv().is_err(), "must not scrobble twice");
    }

    #[test]
    fn history_enter_queues_the_top_song() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::History;
        app.on_msg(Msg::History(crate::stats::StatsReport {
            top_songs: vec![crate::stats::CountedSong {
                uri: "a/one.flac".into(),
                artist: "Artist".into(),
                title: "Title".into(),
                plays: 9,
            }],
            ..Default::default()
        }));
        app.on_key(key(KeyCode::Enter));
        match commands.try_recv() {
            Ok(Command::AppendAndPlay(uris)) => assert_eq!(uris, vec!["a/one.flac"]),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn history_screen_renders_reports() {
        use crate::stats::{Counted, CountedSong, RecentPlay, StatsReport};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = app();
        app.screen = Screen::History;
        app.on_msg(Msg::History(StatsReport {
            total_plays: 12,
            total_seconds: 3_600,
            top_songs: vec![CountedSong {
                uri: "a/one.flac".into(),
                artist: "Portishead".into(),
                title: "Sour Times".into(),
                plays: 7,
            }],
            top_artists: vec![Counted {
                name: "Portishead".into(),
                plays: 7,
            }],
            top_albums: vec![Counted {
                name: "Dummy".into(),
                plays: 7,
            }],
            recent: vec![RecentPlay {
                artist: "Portishead".into(),
                title: "Sour Times".into(),
                played_at: 1,
            }],
        }));
        let backend = TestBackend::new(140, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(&mut app, frame))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Top Songs"));
        assert!(text.contains("Sour Times"));
        assert!(text.contains("12 plays"));
        assert!(text.contains("Recent"));
    }

    #[test]
    fn browser_sorts_directories_first_and_navigates() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Browser;
        app.on_msg(Msg::Browser {
            uri: None,
            entries: vec![
                DirEntry {
                    kind: EntryKind::File,
                    uri: Arc::from("song.flac"),
                    name: Arc::from("song.flac"),
                    song: None,
                },
                DirEntry {
                    kind: EntryKind::Directory,
                    uri: Arc::from("Albums"),
                    name: Arc::from("Albums"),
                    song: None,
                },
            ],
        });
        assert_eq!(app.browser.entries[0].kind, EntryKind::Directory);
        app.on_key(key(KeyCode::Enter));
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Browse(Some(uri))) if uri == "Albums"
        ));
    }

    #[test]
    fn browser_backspace_goes_up_and_enqueue_keys() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Browser;
        app.browser.uri = Some("Albums/One".into());
        app.browser.entries = vec![DirEntry {
            kind: EntryKind::Directory,
            uri: Arc::from("Albums/One/Live"),
            name: Arc::from("Live"),
            song: None,
        }];
        app.on_key(key(KeyCode::Backspace));
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::Browse(Some(uri))) if uri == "Albums"
        ));

        app.browser.uri = Some("Albums/One".into());
        app.browser.entries = vec![DirEntry {
            kind: EntryKind::Directory,
            uri: Arc::from("Albums/One/Live"),
            name: Arc::from("Live"),
            song: None,
        }];
        app.on_key(key(KeyCode::Char('a')));
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::AppendUri(uri)) if uri == "Albums/One/Live"
        ));

        app.on_key(key(KeyCode::Char('A')));
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::ReplaceWithUri(uri)) if uri == "Albums/One/Live"
        ));
    }

    #[test]
    fn header_keeps_the_browser_tab_visible_at_150_columns() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = app();
        let backend = TestBackend::new(150, 42);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(&mut app, frame))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            text.contains("0 Browser"),
            "last screen tab must stay visible"
        );

        let backend = TestBackend::new(200, 42);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(&mut app, frame))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Queue (0)"), "wide terminals show counts");
    }

    #[test]
    fn reloads_config_theme_and_keymap() {
        let directory =
            std::env::temp_dir().join(format!("neo-ncmp-reload-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        std::fs::write(
            &path,
            "[ui]\nnotifications = true\n\n[keys.queue]\n\"d\" = \"play_selected\"\n",
        )
        .unwrap();

        let mut app = app();
        app.config_path = Some(path);
        app.reload_config();
        assert!(app.config.ui.notifications);
        assert_eq!(
            app.keymap
                .action_for(key(KeyCode::Char('d')), Screen::Queue),
            Some(Action::PlaySelected)
        );

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn clear_queue_requires_confirmation() {
        let (mut app, mut commands) = app_with_commands();
        app.screen = Screen::Queue;
        app.queue = vec![Song {
            id: Some(1),
            uri: Arc::from("a/one.flac"),
            ..Song::default()
        }];
        app.on_key(key(KeyCode::Char('C')));
        assert!(app.confirm.is_some());
        assert!(
            commands.try_recv().is_err(),
            "must not clear before confirmation"
        );

        app.on_key(key(KeyCode::Char('y')));
        assert!(matches!(commands.try_recv(), Ok(Command::Clear)));
        assert!(app.confirm.is_none());
    }
}
