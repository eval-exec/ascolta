<div align="center">

# Ascolta

**A fast, keyboard-first music client for MPD — built for the terminal.**

ncmpcpp muscle memory, a modern asynchronous core, and the details that make a
music player feel finished: inline artwork, synced lyrics, a real visualizer,
MPRIS, scrobbling, and instant fuzzy search.

[![CI](https://github.com/eval-exec/ascolta/actions/workflows/ci.yml/badge.svg)](https://github.com/eval-exec/ascolta/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust 1.90+](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org/)
[![MPD 0.23+](https://img.shields.io/badge/MPD-0.23%2B-green.svg)](https://www.musicpd.org/)

</div>

<p align="center">
  <img src="https://github.com/eval-exec/ascolta/releases/download/screenshot/ascolta.png" alt="Ascolta browsing a library with inline artist artwork and the Now Playing card" width="900">
</p>

<p align="center"><em>Library browsing with online artist portraits, inline album art, and the Now Playing card.</em></p>

## Why Ascolta?

Most MPD clients make you choose between the speed of a terminal and the comfort
of a modern desktop player. Ascolta refuses that tradeoff.

- **Instant, even on huge libraries.** The UI never waits on MPD or the network.
  Commands are serialized through an actor, `idle` updates arrive live, and
  search runs on a coalescing worker.
- **Built for keyboard people.** Ten screens, one coherent keymap, a command
  palette, fuzzy search, and per-screen overrides. Mouse support is there when
  you want it, off by default.
- **A player that looks alive.** Album art in lists and in the Now Playing card,
  online artist portraits, synced lyrics, a real FFT spectrum, and a UI that
  adapts from 80×24 terminals to ultrawide layouts.
- **A good desktop citizen.** MPRIS with TrackList and Playlists, media-key
  support, notifications, ListenBrainz/Last.fm scrobbling, and a CLI/IPC surface
  for status bars and scripts.

MPD remains the playback engine. Ascolta is the controller, library, and glue.

## Feature tour

### Playback and queue

- Async MPD client with reconnects, password auth, and live `idle` updates
- Play, pause, stop, next/previous, seek, volume, crossfade
- Repeat, random, single, consume; shuffle and queue editing
- Disc/track-aware album ordering and a current-track window for MPRIS

### Library, browsing, and search

- Artist → album → song browser grouped by album artist
- **Newest albums first**, then alphabetical for undated releases
- Directory-first file browser over MPD `lsinfo`
- Stored playlists: play, append, save, rename, delete, edit
- Fuzzy search (`/`) with structured filters:
  `artist:`, `album:`, `title:`, `genre:`, `format:`, `year:>=1990`, `-genre:rock`
- Saved smart playlists in the command palette, plus persisted search history
  (`Ctrl-P` / `Ctrl-N`)
- SQLite play history: top songs, artists, albums, and recent plays

### Artwork, lyrics, and motion

- Album art through Kitty, Sixel, iTerm2, or halfblocks
- Large cover in the Now Playing card and **inline thumbnails in list rows**
- Keyless online artist portraits from Deezer, cached on disk
- Optional MusicBrainz/Cover Art Archive fallback for missing covers
- Synced lyrics from local `.lrc` files or LRCLIB, with the active line shown in
  the Now Playing card
- Real-time spectrum from PipeWire monitor capture or an MPD FIFO output
- Synced playback position between MPD updates

### Desktop and automation

- MPRIS: media keys, `playerctl`, desktop widgets, TrackList, and Playlists
- Desktop notifications on track change
- ListenBrainz and Last.fm scrobbling with an offline queue, manual retry, and
  entry removal
- Last.fm browser auth helper (`ascolta lastfm-auth`)
- Unix-socket IPC for screen switching, searches, actions, status, volume,
  seeking, playlists, partitions, and notifications
- MPD partitions: list and switch from the CLI, IPC, or `remote`
- Machine-readable `status --json`, shell completions, remote CLI

### Make it yours

- Config hot reload: theme and keybindings apply while the app is running
- Configurable global and per-screen keymap
- TOML theme with hex or ANSI colors
- Configurable pane proportions and Now Playing card behavior
- Opt-in mouse: wheel, tabs, row selection, double-click actions
- Session restore for screen, cursors, playlists pane, lyrics offset, and
  search history

## Install

### From source

```sh
git clone https://github.com/eval-exec/ascolta
cd ascolta
cargo install --path crates/ascolta
```

Or build a release binary:

```sh
cargo build --release
./target/release/ascolta
```

### cargo-binstall

Prebuilt archives are published for **x86_64 Linux (glibc)** and **aarch64
Linux (glibc)** as GitHub release assets. `cargo-binstall` picks the matching
archive, verifies it, and installs the binary without compiling:

```sh
cargo binstall ascolta
```

If no archive matches the current target, binstall can fall back to building
from source. musl/Alpine archives are not published yet; use a glibc target or
build from source.

### Nix

```sh
nix run github:eval-exec/ascolta
```

The flake also installs the man page, example config, and shell completions.

### Arch Linux

A `PKGBUILD` is included in the repository:

```sh
git clone https://github.com/eval-exec/ascolta
cd ascolta
makepkg -si
```

### Docker

Multi-arch images (`linux/amd64`, `linux/arm64`) are published to GitHub
Container Registry:

```sh
mkdir -p ~/.config/ascolta ~/.local/state/ascolta ~/.cache/ascolta

docker run --rm -it --init --network host \
  -e TERM=xterm-256color \
  -v ~/.config/ascolta:/config/ascolta \
  -v ~/.local/state/ascolta:/state/ascolta \
  -v ~/.cache/ascolta:/cache/ascolta \
  -v ~/Music:/music:ro \
  ghcr.io/eval-exec/ascolta:latest
```

The container runs as UID 1000 and stores everything under `/config`, `/state`,
and `/cache` (the XDG directories are redirected there). Create the bind-mount
directories first so they are owned by your user. Images are tagged
`latest`, `X.Y.Z`, and `X.Y`.

`--network host` lets Ascolta reach MPD and the library at `127.0.0.1`. Use this
config inside the container:

```toml
[mpd]
host = "127.0.0.1"
port = 6600

[library]
music_directory = "/music"
```

MPD itself stays on the host; the container only runs the TUI client. On Docker
Desktop (macOS/Windows), host networking is unavailable: use `-p`/`host.docker.internal`
and set `ASCOLTA_MPD_HOST=host.docker.internal` instead. Drop `:ro` from the
music mount if you want in-app tag editing to write files.

A Compose example is available at
[docker-compose.example.yml](docker-compose.example.yml).

The image is intentionally minimal. MPRIS and desktop notifications need a
session D-Bus socket passed into the container, and the PipeWire visualizer needs
`pw-record` plus the PipeWire socket; those integrations are easier to use with
a native install.

### Requirements

- Rust **1.90+** for building (edition 2024)
- MPD **0.23+** (developed against 0.24)
- Linux for MPRIS and desktop notifications; the TUI and remote CLI are portable
- PipeWire (`pw-record`, `pw-metadata`) only if you use the PipeWire visualizer

## Quick start

Create `~/.config/ascolta/config.toml`:

```toml
[mpd]
host = "127.0.0.1"
port = 6600

[library]
music_directory = "/home/you/Music"

[ui]
start_screen = "queue"
```

Then run:

```sh
ascolta
```

If you are coming from ncmpcpp, the first five keys are the same:

| Key | Action |
| --- | --- |
| `1`–`9`, `0` | Queue, Library, Playlists, Outputs, Stats, Lyrics, Visualizer, Tags, History, Browser |
| `?` | Full help |
| `:` | Command palette |
| `/` | Library search |
| `:` then type `smart` | Saved smart playlists |

The full key table lives in the [man page](docs/ascolta.1) and in `?`.

## CLI and IPC

The TUI is optional. The same MPD instance can be controlled from a shell:

```sh
ascolta status --json              # status bars / scripts
ascolta remote toggle              # play/pause
ascolta remote next
ascolta remote volume +5
ascolta remote crossfade 5
ascolta remote playlist "Late Night"
ascolta remote partitions
ascolta remote partition "headphones"
ascolta scrobble queue
ascolta lastfm-auth
```

While the TUI is running, app-specific commands use a private Unix socket
(`$XDG_RUNTIME_DIR/ascolta/socket`, mode 0600):

```sh
ascolta ipc status
ascolta ipc screen library
ascolta ipc search "artist:portishead year:>=1990"
ascolta ipc action next_track
ascolta ipc volume 65
ascolta ipc notify "hello"
```

Disable the listener with `[ipc] enabled = false`; stale sockets left by a crash
are reclaimed on the next start.

## Search

Search blends fuzzy matching with field filters. Bare words are fuzzy terms;
fields narrow the candidates before ranking:

```text
artist:portishead album:"third" year:>=2008 -genre:rock sour
```

`Ctrl-A` enqueues every result, `Ctrl-R` replaces the queue and plays, and
`Ctrl-P`/`Ctrl-N` walk closed-query history. Named queries can become smart
playlists:

```toml
[search.saved]
"90s" = "year:1990-1999"
"Portishead" = "artist:portishead"
```

## Configuration

The complete reference is [config.example.toml](config.example.toml). Main
sections:

| Section | What it controls |
| --- | --- |
| `[mpd]` | Host, port, optional password |
| `[ui]` | Start screen, tick rate, session restore, notifications, mouse, inline art, lyrics in the card |
| `[library]` | Music directory for tag editing |
| `[layout]` | Library and playlist pane proportions |
| `[artwork]` | Cover Art Archive fallback and Deezer artist portraits |
| `[visualizer]` | PipeWire or FIFO capture, sample format, bar count |
| `[scrobbling]` | ListenBrainz/Last.fm credentials and offline queue |
| `[search.saved]` | Named smart playlists |
| `[keys.*]` | Global and per-screen key overrides |
| `[theme]` | Colors for every UI element |
| `[ipc]` | Socket enablement and path |

Environment overrides are available for deployments and scripts:

```text
ASCOLTA_MPD_HOST, ASCOLTA_MPD_PORT
ASCOLTA_LISTENBRAINZ_TOKEN
ASCOLTA_LASTFM_API_KEY, ASCOLTA_LASTFM_API_SECRET, ASCOLTA_LASTFM_SESSION_KEY
```

Legacy `NEO_NCMP_*` variables are still accepted.

### Files

```text
~/.config/ascolta/config.toml          configuration
~/.local/state/ascolta/ascolta.db      play history
~/.local/state/ascolta/session.json    restored UI state and search history
~/.local/state/ascolta/tag-backups/    pre-write tag backups
~/.local/state/ascolta/scrobble-queue.jsonl
~/.local/state/ascolta/ascolta.log     logs (daily rotation)
~/.cache/ascolta/covers/               Cover Art Archive cache
~/.cache/ascolta/artists/              Deezer artist portrait cache
```

Existing `neo-ncmp` config, state, and cache directories are migrated to the
`ascolta` paths automatically on first launch. In the Docker image these paths
are remapped to `/config/ascolta`, `/state/ascolta`, and `/cache/ascolta`.

## Integrations

### MPRIS

Started automatically. Verify:

```sh
busctl --user list | grep org.mpris.MediaPlayer2.ascolta
busctl --user get-property org.mpris.MediaPlayer2.ascolta \
  /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player PlaybackStatus
```

TrackList exposes a 21-track window around the current song; Playlists exposes
stored MPD playlists with ordering, paging, and activation.

### Scrobbling

Enable ListenBrainz and/or Last.fm in `[scrobbling]`. Failed scrobbles go to the
offline queue and retry on the next start, from the palette, or with
`ascolta scrobble retry`. `ascolta lastfm-auth` walks through browser
authorization and prints the session key.

### Visualizer

Use PipeWire capture with no MPD changes:

```toml
[visualizer]
enabled = true
source = "pipewire"
```

Or use the classic MPD FIFO approach:

```mpd
audio_output {
    type "fifo"
    name "ascolta visualizer"
    path "/tmp/mpd.fifo"
    format "44100:16:2"
}
```

```toml
[visualizer]
enabled = true
source = "fifo"
fifo = "/tmp/mpd.fifo"
```

### Artwork

Embedded and folder art come from MPD. With `[artwork] online = true`, missing
covers fall back to MusicBrainz and the Cover Art Archive. Artist rows fetch
keyless portraits from Deezer. Both caches live under `~/.cache/ascolta/`.

## Architecture

```text
       crossterm events ─┐  MPD idle ─┐  ticks ─┐  palette/search workers ─┐
                        └────────────┴─────────┴──────────────────────────┘
                                       │ Msg
                              ┌────────▼─────────┐
                              │    App state     │
                              └───┬──────────┬───┘
                        Command   │          │ render
                                  ▼          ▼
                         ┌──────────────┐  ┌──────────────┐
                         │  MPD worker  │  │   ratatui    │
                         │ command conn │  │   screens    │
                         └──────┬───────┘  └──────────────┘
                                ▼
                    idle / artwork / lyrics / tag / history tasks
```

- **One writer to MPD.** Commands are serialized through an actor; the UI sends
  intentions and never awaits I/O.
- **A separate `idle` connection**, resynced after reconnects.
- **Background workers** for search, artwork, lyrics, tag writes, scrobbles,
  history, and the visualizer. Nothing slow runs on the render path.
- **Viewport-only rendering** and width-aware text keep 100k-song libraries
  responsive.
- **Action-based keymap** with global and per-screen layers, parsed from TOML.
- **Fail safe.** Panic hooks and signal handling restore the terminal; MPD
  disconnects show a banner and reconnect with backoff.

### Workspace

```text
crates/
  ascolta/          binary: event loop, screens, UI, config, workers
  ascolta-domain/   Song/MpdStatus/Library models, no I/O
  ascolta-mpd/      async MPD protocol client, mock-tested, no TUI dependency
```

## Development

```sh
just fmt
just check
just clippy       # -D warnings
just test         # nextest + doctests
just live-test    # against a real MPD
just check-mpd    # --check
```

The suite covers protocol framing (including binary chunks with newlines),
mock-server integration, live MPD, library grouping/sorting, fuzzy ranking, LRC
parsing, scrobble payloads/signatures, SQLite aggregation, keymap parsing, PCM
decoding/FFT bands, every screen at multiple terminal sizes, property-style
filter/cell-width tests, tag writing with an `ffmpeg` fixture, and LRCLIB.

Local benchmarks on a synthetic library: ~122k search items indexed in ~170 ms,
with fuzzy/filtered queries in the 1.5–31 ms range; rendering a 100k-song queue
at 200×60 stays under ~2 ms per frame.

## Limitations

- Library grouping uses **AlbumArtist**; MPD stats count distinct `Artist` tags,
  so counts differ on compilations.
- Albums are ordered newest release first; tracks inside an album stay in
  disc/track order.
- Artist portraits come from Deezer and require network access; there is no
  offline artist-photo source.
- Inline thumbnails are tiny by design. Kitty/Sixel/iTerm2 render them sharply;
  halfblock terminals show a small color swatch.
- The PipeWire visualizer needs `pw-record` and `pw-metadata`. The FIFO mode
  needs an MPD `fifo` output.
- Tag editing writes tags only; embedded art editing is not exposed yet.
- MPRIS and desktop notifications are Linux-oriented. In Docker they also need
  the session D-Bus socket; the default image does not include PipeWire tools.
- The UI is English-only for now.

See [ROADMAP.md](ROADMAP.md) for what is next.

## Contributing

Issues and pull requests are welcome. If you are changing behavior, include a
test and keep the render path allocation-light. Run `just fmt`, `just clippy`,
and `just test` before opening a PR.

## License

MIT. See [LICENSE](LICENSE).
