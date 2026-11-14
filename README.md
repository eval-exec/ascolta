# neo-ncmp

A modern, feature-rich terminal client for [MPD](https://www.musicpd.org/), written in Rust.

The goal is ncmpcpp muscle memory with a new, asynchronous core: instant fuzzy search,
live `idle` updates, non-blocking library indexing, and a UI designed to stay responsive
on libraries far larger than a few thousand songs. MPD remains the playback engine;
neo-ncmp is a first-class controller.

> Status: early, but already usable as a daily driver for queue management, library
> browsing, and fuzzy search. See [ROADMAP.md](ROADMAP.md) for what is next.

```
 neo-ncmp  1 Queue (82)  2 Library (5955)  3 Outputs  4 Stats              ● MPD 0.24.0
┌ Queue — 82 songs ──────────────────────────────────────────────────────────────────────┐
│     # Title                                  Artist              Album          Time   │
│     6 Dissolved Girl                         Massive Attack      Mezzanine       6:06   │
│▶    3 A Thousand Years                       Unknown Artist      Unknown Album   3:04   │
│     4 Azaleh & Eikona - In Aeternum          Unknown Artist      Unknown Album   4:49   │
└────────────────────────────────────────────────────────────────────────────────────────┘
                                              0:00 / 3:04
■ Unknown Artist — A Thousand Years
 vol 100% rep● rnd○ sgl○ cns○                              ? help  q quit  space play
```

## Features

Current:

- Async MPD protocol client (typed commands, `idle`, binary album art, reconnect with backoff)
- Queue editor: play, remove, reorder, clear (with confirmation), shuffle
- Library browser: artists → albums → tracks, grouped by album artist
- Library-wide fuzzy search (`/`) powered by `nucleo-matcher`, with a coalescing background worker
- Transport, seek, volume, repeat/random/single/consume
- Outputs and server/current-track statistics
- Live updates driven by MPD `idle`, plus interpolated progress between updates
- Viewport-only rendering, so large libraries do not cost a frame per item
- TOML configuration, file logging, optional MPD password

Planned: stored playlists, album art, synced lyrics, MPRIS/media keys, desktop
notifications, Last.fm/ListenBrainz, a visualizer, a tag editor, smart playlists,
configurable keymaps/themes/layouts, and a remote CLI. See [ROADMAP.md](ROADMAP.md).

## Requirements

- Rust **1.88+** (edition 2024)
- MPD **0.23+** recommended (developed and tested against MPD 0.24)

## Build and run

```sh
cargo build --release
./target/release/neo-ncmp
```

Or during development:

```sh
just run
```

A non-interactive connectivity check is available:

```sh
neo-ncmp --check
```

It connects to MPD, prints the server version, library counts, current track, and
whether embedded cover art is present, then exits without entering the TUI.

## Configuration

The default path is `~/.config/neo-ncmp/config.toml`:

```toml
[mpd]
host = "127.0.0.1"
port = 6600
# password = "secret"

[ui]
tick_rate_ms = 250
start_screen = "queue"   # queue | library | outputs | stats
follow_now_playing = false
```

See [config.example.toml](config.example.toml). Command-line overrides:

```sh
neo-ncmp --host music.local --port 6600
neo-ncmp --config ./my-config.toml
```

Environment overrides (useful for one-off runs): `NEO_NCMP_MPD_HOST`,
`NEO_NCMP_MPD_PORT`.

Logs are written to `~/.local/state/neo-ncmp/neo-ncmp.log` (daily rotation; never
stdout, so the TUI is not corrupted). Set `RUST_LOG=debug` for more detail.

## Key bindings

| Keys | Action |
| --- | --- |
| `?` | Toggle help |
| `q`, `Ctrl-C` | Quit |
| `1`–`4` | Queue / Library / Outputs / Stats |
| `Tab`, `Shift-Tab` | Next / previous screen |
| `j` `k`, arrows | Move cursor |
| `Ctrl-D` / `Ctrl-U` | Half-page down / up |
| `g` / `G` | First / last |
| `space`, `p` | Play / pause |
| `s` | Stop |
| `>` `.` `n` / `<` `,` `N` | Next / previous track |
| `f` / `b` | Seek +5s / -5s |
| `+` / `-` | Volume up / down |
| `r` `z` `x` `c` | Repeat / random / single / consume |
| `/` | Fuzzy search the library |
| `Enter` | Play selection |
| `a` / `A` (library) | Append / replace queue and play |
| `d` (queue) | Remove selected |
| `J` / `K` (queue) | Move selected down / up |
| `C` / `S` (queue) | Clear (confirms) / shuffle |
| `h` / `l` (library) | Previous / next pane |
| `Enter` (outputs) | Toggle output |
| `R` | Reload MPD state and library |

In search: type to filter, `Up`/`Down` to select, `Enter` to append and play,
`Ctrl-U` to clear, `Esc` to close. Typing is never blocked by matching; results are
computed on a background worker and stale responses are discarded.

## Design

```
       crossterm events ─┐  MPD idle ─┐  ticks ─┐
                        └────────────┴─────────┘
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
                        idle watcher (separate connection)
```

- **One writer to MPD.** Commands are serialised through an actor; the UI sends
  intentions and never awaits I/O.
- **Separate `idle` connection.** MPD's one-command-in-flight rule is respected;
  changes are pushed into the app as messages.
- **Pure-ish state transitions.** `App::on_msg` and `App::on_key` are testable
  without a terminal; inputs map to an `Action` enum, not to hard-coded keys.
- **Viewport rendering.** Song lists render only the visible rows and handle
  wide/CJK glyphs with display-width-aware padding.
- **Background search.** `SearchIndex::score` is a pure function run on
  `spawn_blocking`; a generation counter discards stale results.
- **Fail safe.** The terminal is restored on panic via `ratatui::init`'s hook;
  MPD disconnect shows a banner and reconnects with backoff.

### Workspace

```
crates/
  neo-ncmp/          binary: event loop, screens, UI, config, search, workers
  neo-ncmp-domain/   Song/MpdStatus/Library/Filter-friendly models, no I/O
  neo-ncmp-mpd/      async MPD protocol client, mock-tested, no TUI dependency
```

## Development

```sh
just fmt        # cargo fmt
just check      # cargo check
just clippy     # clippy with -D warnings
just test       # nextest + doc tests
just live-test  # integration tests against a real MPD on 127.0.0.1:6600
just run
```

Tests include:

- MPD protocol framing, including binary chunks whose payload contains newlines
- Mock-server integration for status, queue, idle, errors, and `readpicture`
- A live, `#[ignore]`d test against a real daemon
- Domain grouping/sorting and fuzzy search ranking
- Keypress → command tests (pause, volume, queue removal, library enqueue)
- List scrolling and width-aware text fitting

Set `MPD_HOST` / `MPD_PORT` to point the live tests at a different server.

## Notes and limitations

- The library is grouped by **AlbumArtist**, then album. MPD's own stats count
  distinct `Artist` tags, so the two counts can differ on compilations.
- Album art, lyrics, the visualizer, MPRIS, and tag editing are not implemented yet.
- The visualizer will need either an MPD FIFO output (a change to the MPD
  configuration) or PipeWire monitor capture.

## License

MIT.
