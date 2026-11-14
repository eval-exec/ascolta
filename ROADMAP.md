# Roadmap

MPD owns playback; neo-ncmp is a controller, library UI, and desktop citizen.

## Done

### Core

- [x] Workspace split: UI binary, domain model, async MPD client
- [x] Async MPD client: typed commands, `idle`, binary `readpicture`/`albumart`,
      reconnect, password auth, mock and live tests
- [x] Event-driven app: MPD command actor, idle watcher, search worker, no UI blocking
- [x] Viewport-only rendering for large libraries, display-width-aware columns
- [x] TOML config, XDG paths, file logging, panic/signal-safe terminal restore

### Playback and library

- [x] Queue: play, remove, reorder, clear (confirm), shuffle, viewport scrolling
- [x] Transport, seek, volume, repeat/random/single/consume
- [x] Artist → album → track browser, case-insensitive, album-artist grouping
- [x] Stored playlists: browse songs, load/play, append, save queue as, rename, delete,
      remove/reorder songs
- [x] Fuzzy search over artists/albums/songs with a coalescing background worker
- [x] Command palette (`:`) and jump-to-screen
- [x] Play history: SQLite-backed top songs/artists/albums, recent plays, play-from-history
- [x] Session restore: screen, cursors, playlists pane, lyrics offset
- [x] Configurable keymap with global/per-screen overrides and `none` unbinds
- [x] Configurable theme colors (`[theme]`, hex/ANSI names)

### Modern integrations

- [x] Album art: MPD `readpicture`/`albumart`, halfblock/Kitty/Sixel/iTerm2 via
      `ratatui-image`, background fetch, in-memory cache, Now Playing sidebar
- [x] Synced lyrics: local `.lrc` preference + LRCLIB fallback, LRC parser,
      active-line display, offset adjustment, background fetch
- [x] MPRIS server: media keys/playerctl, metadata, PlayPause, seek, position,
      volume, loop/shuffle, D-Bus released on exit
- [x] Desktop notifications on track change
- [x] Remote CLI and JSON status for status bars/keybindings
- [x] ListenBrainz and Last.fm scrobbling with threshold tracking and an offline queue
- [x] Real-time visualizer: MPD FIFO PCM reader or PipeWire monitor capture,
      real FFT, log-spaced bands, peak-hold smoothing
- [x] Tag editor: read/write tags with `lofty`, full-file backup before writes,
      confirmation, MPD rescan
- [x] Directory-first file browser over MPD `lsinfo`: navigation, enqueue, replace+play
- [x] Hot reload of config, theme, and keymap (file watcher)

## Next

### Parity / UI

- [x] Structured filter DSL (`artist:`, `album:`, `year:>=1990`, negated filters)
- [x] Saved smart playlists: named filter queries in the palette; play or enqueue all results
- [ ] Multi-column configurable layout (beyond the built-in adaptive layout)
- [ ] Mouse support (opt-in)
- [ ] Search history and saved searches
- [ ] Track-list and playlist MPRIS interfaces

### Integrations

- [ ] Cover Art Archive/MusicBrainz fallback when embedded art is absent
- [ ] Last.fm browser-auth flow and `lastfm-auth` helper
- [ ] Unix-socket IPC for app-specific remote commands
- [ ] Scrobble editing/queue inspection UI
- [ ] Multi-partition support

### Quality and distribution

- [ ] `insta` snapshot tests for every screen at 80×24, 120×40, 200×60
- [ ] Property tests for keymap/filter parsers; fuzz the MPD protocol parser
- [ ] Benchmarks for search, filtering, and 100k-song rendering
- [ ] CI live-MPD job with a generated test library
- [ ] Packaging polish: Nix flake, AUR, cargo-dist binaries, man page
- [ ] Translations/i18n

## Design notes

**Tag editing is destructive.** Every write first copies the original file to
`~/.local/state/neo-ncmp/tag-backups/`. If the copy fails, the write is aborted.
Batch editing and an undo journal are future work.

**Visualizer.** MPD's `fifo` output is the classic ncmpcpp approach but requires
editing `mpd.conf` (on NixOS, the Nix module). `neo-ncmp` opens the FIFO at
startup so MPD's output does not block. The `pipewire` source is the zero-config
alternative: it resolves the default sink with `pw-metadata` and captures its
monitor via `pw-record`, so other applications on the sink show up too. Set
`[visualizer] target` to pin a specific sink.

**Album art in tmux.** Kitty/Sixel passthrough needs `allow-passthrough` in tmux;
otherwise `ratatui-image` falls back to halfblocks.

**Scrobbling.** Disabled by default. Credentials live in config or environment;
failed scrobbles are appended to `~/.local/state/neo-ncmp/scrobble-queue.jsonl`
and retried on startup.
