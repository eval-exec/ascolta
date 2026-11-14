use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use neo_ncmp_domain::{
    Album, Artist, Library, MpdOutput, MpdStats, MpdStatus, PlaybackState, Song,
};
use neo_ncmp_mpd::Subsystem;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::input::{self, Action};
use crate::mpd_worker::{self, Command, MpdHandle, Msg, PlayerOption};
use crate::search::{SearchHandle, SearchIndex, SearchState};
use crate::ui;
use crate::widgets::ListView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Queue,
    Library,
    Outputs,
    Stats,
}

impl Screen {
    pub const ALL: [Self; 4] = [Self::Queue, Self::Library, Self::Outputs, Self::Stats];

    pub fn index(self) -> usize {
        match self {
            Self::Queue => 0,
            Self::Library => 1,
            Self::Outputs => 2,
            Self::Stats => 3,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Queue => "Queue",
            Self::Library => "Library",
            Self::Outputs => "Outputs",
            Self::Stats => "Stats",
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
            "outputs" => Self::Outputs,
            "stats" => Self::Stats,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    ClearQueue,
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

    pub search: SearchState,
    pub search_index: Option<Arc<SearchIndex>>,
    search_handle: SearchHandle,
    search_generation: u64,

    pub connection: ConnectionState,
    pub version: Option<String>,
    pub help: bool,
    pub confirm: Option<Confirm>,
    pub toast: Option<Toast>,
    pub should_quit: bool,

    elapsed_at_sync: Duration,
    elapsed_synced_at: Instant,
    tick: u64,
    library_reload_at: Option<Instant>,
}

impl App {
    pub fn new(config: Config, mpd: MpdHandle, search_handle: SearchHandle) -> Self {
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
            search: SearchState::default(),
            search_index: None,
            search_handle,
            search_generation: 0,
            connection: ConnectionState::Connecting,
            version: None,
            help: false,
            confirm: None,
            toast: None,
            should_quit: false,
            elapsed_at_sync: Duration::ZERO,
            elapsed_synced_at: Instant::now(),
            tick: 0,
            library_reload_at: None,
        }
    }

    pub async fn run(config: Config, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let endpoint = mpd_worker::Endpoint::new(config.mpd_addr(), config.mpd.password.clone());
        let mpd = mpd_worker::spawn(endpoint.clone(), tx.clone());
        mpd_worker::spawn_idle(endpoint, tx.clone());
        let search_handle = SearchHandle::spawn(tx.clone());

        let mut app = App::new(config.clone(), mpd, search_handle);
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
        Ok(())
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

        if self.help {
            self.help = false;
            return;
        }

        if self.search.active {
            self.handle_search_key(key);
            return;
        }

        if let Some(action) = input::action_for(key, self.screen) {
            self.handle_action(action);
        }
    }

    pub fn on_msg(&mut self, message: Msg) {
        match message {
            Msg::Connected { version } => {
                self.connection = ConnectionState::Connected(version.clone());
                self.version = Some(version);
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
                self.elapsed_at_sync = status.elapsed;
                self.elapsed_synced_at = Instant::now();
                self.status = Some(status);
            }
            Msg::CurrentSong(song) => self.current = song,
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
            Action::NextScreen => self.set_screen(self.screen.next()),
            Action::PreviousScreen => self.set_screen(self.screen.previous()),
            Action::GotoScreen(index) => {
                if let Some(screen) = Screen::ALL.get(index) {
                    self.set_screen(*screen);
                }
            }
            Action::Refresh => {
                self.mpd.send(Command::RefreshAll);
                self.toast_info("Refreshing…");
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
            Action::SeekForward => self.mpd.send(Command::Seek(5)),
            Action::SeekBackward => self.mpd.send(Command::Seek(-5)),
            Action::ToggleRepeat => self.toggle_option(PlayerOption::Repeat),
            Action::ToggleRandom => self.toggle_option(PlayerOption::Random),
            Action::ToggleSingle => self.toggle_option(PlayerOption::Single),
            Action::ToggleConsume => self.toggle_option(PlayerOption::Consume),
            Action::PlaySelected => self.play_selected(),
            Action::EnqueueSelected => self.enqueue_selected(),
            Action::ReplaceAndPlay => self.replace_and_play(),
            Action::RemoveSelected => self.remove_selected(),
            Action::MoveSelectionDown => self.move_queue_selection(1),
            Action::MoveSelectionUp => self.move_queue_selection(-1),
            Action::ClearQueue => self.request_clear_queue(),
            Action::ShuffleQueue => {
                self.mpd.send(Command::Shuffle);
                self.toast_info("Queue shuffled");
            }
            Action::PaneLeft => self.change_pane(-1),
            Action::PaneRight => self.change_pane(1),
            Action::ToggleOutput => self.toggle_output(),
        }
    }

    fn open_search(&mut self) {
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
            Screen::Outputs => self.mpd.send(Command::RefreshOutputs),
            Screen::Stats => self.mpd.send(Command::RefreshStats),
            _ => {}
        }
    }

    fn move_selection(&mut self, delta: isize) {
        match self.screen {
            Screen::Queue => self.queue_view.move_by(delta, self.queue.len()),
            Screen::Library => self.move_library(delta),
            Screen::Outputs => self.outputs_view.move_by(delta, self.outputs.len()),
            Screen::Stats => {}
        }
    }

    fn page_selection(&mut self, direction: isize) {
        const PAGE: usize = 10;
        match self.screen {
            Screen::Queue => self.queue_view.page(direction, self.queue.len(), PAGE),
            Screen::Library => self.move_library(direction.saturating_mul(PAGE as isize)),
            Screen::Outputs => self.outputs_view.page(direction, self.outputs.len(), PAGE),
            Screen::Stats => {}
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
            Screen::Outputs => {
                if last {
                    self.outputs_view.last(self.outputs.len());
                } else {
                    self.outputs_view.first();
                }
            }
            Screen::Stats => {}
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
            Screen::Outputs => self.toggle_output(),
            Screen::Stats => {}
        }
    }

    fn enqueue_selected(&mut self) {
        if self.screen != Screen::Library {
            return;
        }
        let uris = self.selected_uris();
        if !uris.is_empty() {
            self.toast_info(format!("Enqueued {} track(s)", uris.len()));
            self.mpd.send(Command::AppendUris(uris));
        }
    }

    fn replace_and_play(&mut self) {
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
        if self.screen != Screen::Queue {
            return;
        }
        if let Some(song) = self.queue.get(self.queue_view.cursor)
            && let Some(id) = song.id
        {
            self.mpd.send(Command::RemoveId(id));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        App::new(
            Config::default(),
            MpdHandle::test_handle(),
            SearchHandle::test_handle(),
        )
    }

    fn app_with_commands() -> (App, mpsc::UnboundedReceiver<Command>) {
        let (mpd, receiver) = MpdHandle::test_channel();
        (
            App::new(Config::default(), mpd, SearchHandle::test_handle()),
            receiver,
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn screens_cycle_both_ways() {
        assert_eq!(Screen::Queue.next(), Screen::Library);
        assert_eq!(Screen::Queue.previous(), Screen::Stats);
        assert_eq!(Screen::Stats.next(), Screen::Queue);
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
