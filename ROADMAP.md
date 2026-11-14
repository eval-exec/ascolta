# Roadmap

This is a living plan, not a promise. MPD already owns playback, so the work is
concentrated on being the best possible controller and library UI.

## Done

- [x] Workspace split: UI binary, domain model, async MPD client
- [x] Async MPD client: typed commands, `idle`, binary `readpicture`/`albumart`,
      reconnect, mock and live tests
- [x] Terminal lifecycle: alternate screen, raw mode, panic restore, resize
- [x] Event-driven app: worker actors, message-based state updates, no UI blocking
- [x] Queue screen with viewport rendering, track/artist/album columns,
      transport, seek, volume, repeat/random/single/consume
- [x] Library browser: artists → albums → tracks, case-insensitive sorting,
      album-artist grouping
- [x] Outputs and server/current-track statistics screens
- [x] Fuzzy search over artists, albums, and songs with `nucleo-matcher`
- [x] Background search worker with request coalescing and stale-result rejection
- [x] TOML config, XDG paths, file logging, MPD password
- [x] Confirmation modal for destructive actions, transient toasts
- [x] Tests: protocol framing, mock server, live MPD, domain, search, key → command

## Next: finish the daily-driver parity

- [ ] Stored playlists screen (list, load, save queue, delete, rename)
- [ ] Playlist editor (append/remove/reorder songs in a stored playlist)
- [ ] File browser (`lsinfo`) for directories and non-tag browsing
- [ ] Structured filter/search DSL (`artist:`, `album:`, `year:>=1990`)
- [ ] Command palette (`:`) and jump-to-anything
- [ ] Mouse support (optional, off by default)
- [ ] Session restore (last screens, panes, search history)
- [ ] Configurable keymap, theme, and multi-pane layout (hot reload)

## Modern integrations

- [ ] Album art: `readpicture`/`albumart` → decode → `ratatui-image` (Kitty,
      Sixel, iTerm2, halfblock fallback), with a disk/URI cache
- [ ] Synced lyrics: local `.lrc` plus LRCLIB, offset adjustment, auto-scroll
- [ ] MPRIS server (`mpris-server`): media keys, `playerctl`, desktop widgets
- [ ] Desktop notifications on track change (`notify-rust`)
- [ ] Remote CLI + JSON status (`neo-ncmp remote next`, `status --json`) for
      status bars and keybindings
- [ ] Last.fm and ListenBrainz scrobbling, with an offline queue
- [ ] Smart playlists and play statistics backed by SQLite, with MPD stickers
      for portable ratings
- [ ] Unix-socket IPC for app-specific commands

## Signature extras

- [ ] Visualizer: FFT spectrum from an MPD FIFO output, or PipeWire monitor
      capture when reconfiguring MPD is undesirable
- [ ] Tag editor: `lofty` read/write, batch album edits, embedded art, undo
      journal, then an MPD rescan
- [ ] Cover art from Cover Art Archive/MusicBrainz when embedded art is missing
- [ ] Multi-partition and multi-instance support
- [ ] Plugin hooks (Lua or Rhai) for user-defined actions

## Hardening

- [ ] `insta` snapshot tests for every screen at 80×24, 120×40, and 200×60
- [ ] Property tests for the filter/keymap/layout parsers
- [ ] Benchmarks for search, filtering, and rendering 100k songs
- [ ] Fuzzing for the MPD protocol parser
- [ ] CI: fmt, clippy `-D warnings`, nextest, cargo-deny/audit
- [ ] Packaging: Nix flake, AUR, cargo-dist binaries, man page

## Design notes

**Visualizer.** MPD's `fifo` output is the classic ncmpcpp approach but requires
editing `mpd.conf` (on NixOS, the Nix module). The alternative is capturing the
PipeWire monitor, which needs no MPD change but is Linux-specific. Make it opt-in
and support both sources.

**Album art in tmux.** Kitty/Sixel passthrough needs `allow-passthrough` in tmux;
otherwise fall back to halfblocks. Detect the protocol at startup and degrade
gracefully.

**Tag editing is destructive.** Write to a temp file and rename, keep an undo
journal, confirm batch edits, and trigger an MPD update/rescan afterwards.
