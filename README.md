# neo-ncmp

A modern, feature-rich terminal client for [MPD](https://www.musicpd.org/), written in Rust.

The goal is ncmpcpp muscle memory with a new asynchronous core: live `idle` updates,
non-blocking library indexing, instant fuzzy search, terminal album art, synced lyrics,
MPRIS, scrobbling, and an interactive play history. MPD remains the playback engine;
neo-ncmp is a first-class controller and desktop citizen.

```
 neo-ncmp  1 Queue (82)  2 Library (5955)  3 Playlists (1)  4 Outputs  5 Stats
           6 Lyrics  7 Visualizer  8 Tags  9 History                    ● MPD 0.24.0
┌ Queue — 82 songs ──────────────────────────────┐┌ Now Playing ──────────────────┐
│     # Title                  Artist      Time  ││ ▀▀▄▄▄▄▄▄▄▄▄▄▀▀▀▀▄▄▄▀         │
│     6 Dissolved Girl         Massive     6:06  ││ ▀▀▀▀▀▀▀▀▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▀▄▀▄▄▀  │
│▶    3 A Thousand Years       Unknown     3:04  ││ ▀▀▀▀▀▀▀▀▀▀▄▀▀▄▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀  │
└────────────────────────────────────────────────┘│ Massive Attack — Dissolved…  │
                                                  └───────────────────────────────┘
                                      0:00 / 3:04
■ Unknown Artist — A Thousand Years
 vol 100% rep● rnd○ sgl○ cns○           ? help  : palette  / search  q quit
```

## Features

**Playback and queue**

- Async MPD client with typed commands, `idle`, reconnect with backoff, password auth
- Queue editor: play, remove, reorder, clear with confirmation, shuffle
- Transport, seek, volume, repeat/random/single/consume
- Interpolated progress between MPD updates

**Library**

- Artist → album → track browser, case-insensitive, grouped by album artist
- Directory-first file browser (`0`) over MPD `lsinfo`; open folders, enqueue, or replace the queue
- Library-wide fuzzy search (`/`) using `nucleo-matcher`, run on a coalescing worker
- Structured search filters: `artist:`, `album:`, `title:`, `genre:`, `format:`, `year:>=1990`, `-genre:rock`
- Saved smart playlists: named filter queries in the palette, with play-all/enqueue-all
- Stored playlists: browse, load/play, append, save queue as, rename, delete, edit songs
- Command palette (`:`) for every global action
- Session restore: last screen, cursors, playlists pane, lyrics offset
- SQLite play history: top songs/artists/albums, recent plays, play a top song with Enter

**Modern extras**

- Album art in the terminal (Kitty/Sixel/iTerm2/halfblocks) with background fetch and cache
- Optional Cover Art Archive fallback for tracks without embedded or folder art
- Synced lyrics from local `.lrc` or LRCLIB, active-line highlight, `[`/`]` offset
- MPRIS server: media keys, `playerctl`, desktop widgets; released cleanly on exit
- Desktop notifications on track change
- ListenBrainz and Last.fm scrobbling with offline retry queue
- Real-time spectrum visualizer from an MPD FIFO output or PipeWire monitor capture
- Tag editor with full-file backups and confirmation before writing
- Remote CLI and machine-readable `status --json` for status bars
- Unix-socket IPC for app-specific control (switch screens, open searches, inspect state)
- MPD partitions: list and switch from the CLI, IPC, or `remote`
- Configurable keymap (global + per-screen overrides) and theme colors
- Config hot reload: edits to `config.toml` apply theme and keybindings immediately
- Opt-in mouse support: wheel scrolling, clickable screen tabs, row selection, double-click to play

## Requirements

- Rust **1.89+** (edition 2024)
- MPD **0.23+** recommended (developed against MPD 0.24)
- Linux for MPRIS/desktop notifications; the TUI and remote CLI are otherwise portable

## Build and run

```sh
cargo build --release
./target/release/neo-ncmp
```

or during development:

```sh
just run
```

### CLI

```sh
neo-ncmp                       # TUI
neo-ncmp --check               # connectivity/library summary, no TUI
neo-ncmp check                 # same as a subcommand
neo-ncmp status                # one-line playback status
neo-ncmp status --json         # JSON for Waybar/polybar/scripts
neo-ncmp remote toggle         # play/pause, no TUI
neo-ncmp remote next
neo-ncmp remote previous
neo-ncmp remote stop
neo-ncmp remote volume +5      # or -5, or an absolute 0-100
neo-ncmp remote seek +10       # seconds, absolute or relative
neo-ncmp remote repeat toggle  # on | off | toggle
neo-ncmp remote random on
neo-ncmp remote single toggle
neo-ncmp remote consume off
neo-ncmp remote shuffle
neo-ncmp remote clear
neo-ncmp remote playlist "Late Night"
neo-ncmp remote output 0 on
neo-ncmp remote partitions         # active partition marked with *
neo-ncmp remote partition "headphones"
neo-ncmp remote status --json
```

While the TUI is running, app-specific commands go through its private Unix
socket (`$XDG_RUNTIME_DIR/neo-ncmp/socket`, mode 0600):

```sh
neo-ncmp ipc ping
neo-ncmp ipc status                 # JSON with the current screen, song, queue length
neo-ncmp ipc screen library         # queue, library, playlists, outputs, stats,
                                    # lyrics, visualizer, tags, history, browser
neo-ncmp ipc action next_track      # any keymap action name
neo-ncmp ipc search "artist:portishead year:>=1990"
neo-ncmp ipc volume 65
neo-ncmp ipc seek 42.5
neo-ncmp ipc playlist "Late Night"  # replace queue and play
neo-ncmp ipc partition "headphones" # switch MPD partition
neo-ncmp ipc notify "hello"
```

Disable the listener with `[ipc] enabled = false`, or choose another path with
`[ipc] path`. Stale sockets from a crashed instance are reclaimed on start.

Global flags (`--host`, `--port`, `--config`) work before or after subcommands.

## Configuration

Default path: `~/.config/neo-ncmp/config.toml`. See [config.example.toml](config.example.toml)
for every option. A minimal setup:

```toml
[mpd]
host = "127.0.0.1"
port = 6600

[library]
music_directory = "/home/you/Music"

[ui]
start_screen = "queue"
```

Config is layered: defaults → file → CLI flags. Environment overrides:
`NEO_NCMP_MPD_HOST`, `NEO_NCMP_MPD_PORT`, `NEO_NCMP_LISTENBRAINZ_TOKEN`,
`NEO_NCMP_LASTFM_API_KEY`, `NEO_NCMP_LASTFM_API_SECRET`,
`NEO_NCMP_LASTFM_SESSION_KEY`.

Logs: `~/.local/state/neo-ncmp/neo-ncmp.log` (daily rotation, never stdout).
Mouse support is off by default; set `[ui] mouse = true` to enable it. Wheel
scrolling and clicking rows always work on the focused list; clicking a header
tab switches screens; double-clicking activates the row (play, open directory,
toggle output, edit tag). Set `RUST_LOG=debug` for detail.

## Key bindings

Defaults match ncmpcpp where practical. Every binding is overridable under
`[keys.<context>]`; use `"none"` to unbind.

| Keys | Action |
| --- | --- |
| `?` | Toggle help (full list) |
| `:` | Command palette |
| `/` | Fuzzy search the library (supports field filters) |
| `q`, `Ctrl-C` | Quit |
| `1`–`9`, `0` | Queue / Library / Playlists / Outputs / Stats / Lyrics / Visualizer / Tags / History / Browser |
| `Tab`, `Shift-Tab` | Next / previous screen |
| `j` `k`, arrows | Move cursor |
| `Ctrl-D` / `Ctrl-U` | Half-page down / up |
| `g` / `G` | First / last |
| `space`, `p` | Play / pause |
| `s` | Stop |
| `>` `.` `n` / `<` `,` `N` | Next / previous track |
| `f` / `b` | Seek +5s / -5s |
| `+` / `-` | Volume |
| `r` `z` `x` `c` | Repeat / random / single / consume |
| `Enter` | Play selection / edit tag field on Tags |
| `a` / `A` (library) | Append / replace queue and play |
| `d` (queue/playlists) | Remove selected |
| `J` / `K` (queue/playlists) | Move selected down / up |
| `C` / `S` (queue) | Clear (confirms) / shuffle |
| `S` `e` `x` (playlists) | Save queue as / rename / delete |
| `h` / `l` (library/playlists) | Previous / next pane |
| `h`, `Backspace` (browser) | Parent directory |
| `w` (tags) | Write tags to disk (backs up first) |
| `o` `[` `]` (lyrics) | Refetch / offset -0.5s / +0.5s |
| `Enter` (history) | Queue the selected most-played song |
| `a` / `A` (browser) | Enqueue / replace queue and play |
| `Enter` (outputs) | Toggle output |
| `Ctrl-A` / `Ctrl-R` (search) | Enqueue all / replace queue with all results |
| Mouse (opt-in) | Wheel scrolls; click tabs/rows; double-click plays or edits |
| `R` | Reload state/library (or History/Browser) |

### Search filters

The search overlay understands field filters alongside fuzzy terms:

```
artist:portishead album:"third" year:>=2008 -genre:rock sour
```

| Filter | Matches |
| --- | --- |
| `title:`, `t:` | Song title (fuzzy terms still rank the result) |
| `artist:`, `a:` | Artist (album artist as fallback) |
| `album:`, `al:` | Album name |
| `albumartist:`, `aa:` | Album-artist tag |
| `genre:`, `g:` | Genre tag |
| `year:`, `date:` | Four-digit year: `1998`, `>=1990`, `<2000`, `1990-1999` |
| `format:`, `f:` | MPD-reported format, such as `FLAC` or `MP3` |
| `uri:`, `path:` | File URI/path |

Prefix any filter with `-` to exclude matches (`-genre:rock`). Quote values that
contain spaces. Unknown fields and unparseable years stay ordinary fuzzy terms.
Inside the search overlay, `Ctrl-A` enqueues every displayed result and `Ctrl-R`
replaces the queue with them and starts playback.

Saved queries live under `[search.saved]` and appear in the command palette (`:`)
as smart playlists:

```toml
[search.saved]
"90s" = "year:1990-1999"
"Portishead" = "artist:portishead"
```

Choose one to open the search overlay pre-filled; then use `Ctrl-R` to play the
whole set or `Ctrl-A` to append it.

Example override:

```toml
[keys.global]
"ctrl+j" = "next_track"
":" = "palette"

[keys.queue]
"d" = "play_selected"
"S" = "none"
```

## Integrations

### MPRIS and media keys

Started automatically. Verify with:

```sh
busctl --user list | grep org.mpris.MediaPlayer2.neo-ncmp
busctl --user call org.mpris.MediaPlayer2.neo-ncmp /org/mpris/MediaPlayer2 \
  org.freedesktop.DBus.Properties Get ss org.mpris.MediaPlayer2.Player PlaybackStatus
```

If no D-Bus session is available the TUI still runs; a warning is logged.

### Notifications

Set `[ui] notifications = true`. Uses the desktop notification service via
`notify-rust`.

### Scrobbling

```toml
[scrobbling]
enabled = true
listenbrainz_token = "…"
# Last.fm requires an API key/secret and a session key obtained out of band:
lastfm_api_key = "…"
lastfm_api_secret = "…"
lastfm_session_key = "…"
```

ListenBrainz `playing_now` is sent when a track starts; a scrobble is sent after
half the track or four minutes, whichever comes first. Failed scrobbles are queued
in `~/.local/state/neo-ncmp/scrobble-queue.jsonl` and retried on the next start.

### Visualizer

Two capture sources are supported.

**PipeWire** (default when `source = "pipewire"`): neo-ncmp asks `pw-metadata` for
the current default sink, then reads its monitor with `pw-record` and
`stream.capture.sink=true`. This needs no MPD changes and picks up every
application on that sink. Set `target` to capture a specific sink:

```toml
[visualizer]
enabled = true
source = "pipewire"
# target = "bluez_output.E4_58_BC_8E_FE_13.1"
sample_rate = 44100
channels = 2
bits = 16
bars = 64
```

**MPD FIFO** (`source = "fifo"`, the default): add a FIFO output to MPD:

```mpd
audio_output {
    type "fifo"
    name "neo-ncmp visualizer"
    path "/tmp/mpd.fifo"
    format "44100:16:2"
}
```

and point `fifo` in config at the same path. On NixOS the MPD config is generated
by the Nix module; edit that instead of the store path. neo-ncmp opens the FIFO at
startup so MPD's FIFO output does not block.

Only 16-bit PCM is supported today.

### Album art

Embedded art is read via MPD's `readpicture`, with `albumart` as a fallback for
folder art. Under tmux, Kitty/Sixel passthrough needs `allow-passthrough on`;
otherwise the halfblock renderer is used automatically.

Set `[artwork] online = true` to fall back to MusicBrainz and the Cover Art
Archive when both MPD sources come up empty. Lookups run in the background,
respect MusicBrainz's request spacing, and cache covers in
`~/.cache/neo-ncmp/covers/`; failed lookups are remembered for a week.

### Tag editing

Set `[library] music_directory`. The editor requires a confirmation and copies the
original file to `~/.local/state/neo-ncmp/tag-backups/` before writing. If the copy
fails, the write is aborted. After a successful write MPD is asked to rescan the
track.

### Lyrics

Local `.lrc` files next to the track are preferred; otherwise LRCLIB is queried.
Network failures are non-fatal. Use `o` to retry and `[`/`]` to adjust sync.

## Design

```
       crossterm events ─┐  MPD idle ─┐  ticks ─┐  palette/search workers ─┐
                        └────────────┴─────────┴──────────────────────────┘
                                       │ Msg
                              ┌────────▼─────────┐
                              │    App state     │
                              └───┬──────────┬───┘
                        Command   │          │ render
                                  ▼          ▼
                         ┌──────────────┐  ┌──────────────┐
                         │  MPD worker  │  │ ratatui      │
                         │ command conn │  │ screens      │
                         └──────┬───────┘  └──────────────┘
                                ▼
                    idle / artwork / lyrics / tag / history tasks
```

- **One writer to MPD.** Commands are serialised through an actor; the UI sends
  intentions and never awaits I/O.
- **Separate `idle` connection**, re-synced after reconnects.
- **Background workers** for fuzzy search, artwork, lyrics, tag writes, scrobble
  queue, history queries, and the visualizer. Nothing slow runs on the render path.
- **Action-based keymap** with global and per-screen layers, parsed from TOML.
- **Viewport rendering** and width-aware text so 100k-song libraries stay responsive.
- **Fail safe.** Panic hook and SIGTERM/SIGINT handling restore the terminal;
  MPD disconnects show a banner and reconnect with backoff.

### Workspace

```
crates/
  neo-ncmp/          binary: event loop, screens, UI, config, workers
  neo-ncmp-domain/   Song/MpdStatus/Library models, no I/O
  neo-ncmp-mpd/      async MPD protocol client, mock-tested, no TUI dependency
```

## Development

```sh
just fmt
just check
just clippy     # -D warnings
just test       # nextest + doctests
just live-test  # against a real MPD
just check-mpd  # --check
```

Tests cover protocol framing (including binary chunks containing newlines),
mock-server integration, live MPD, domain grouping/sorting, fuzzy ranking, LRC
parsing, scrobble payloads and signatures, SQLite aggregation, keymap parsing,
PCM decoding/FFT bands, UI snapshots via `TestBackend`, tag writing with an
`ffmpeg`-generated fixture (ignored), and LRCLIB fetching (ignored).

## Limitations

- The library is grouped by **AlbumArtist**; MPD stats count distinct `Artist`
  tags, so counts differ on compilations.
- The PipeWire visualizer needs `pw-record` and `pw-metadata` from PipeWire; the FIFO mode needs an MPD `fifo` output.
- Last.fm's browser auth flow is not implemented yet.
- Tag editing writes tags only; embedded art editing is not exposed yet.
- MPRIS/notifications are Linux-oriented.

See [ROADMAP.md](ROADMAP.md) for what's next.

## License

MIT.
