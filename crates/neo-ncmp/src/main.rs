mod app;
mod config;
mod coverart;
mod filter;
mod input;
mod logging;
mod lyrics;
mod mpd_worker;
mod mpris;
mod notifications;
mod palette;
mod scrobbler;
mod search;
mod session;
mod stats;
mod tags;
mod ui;
mod visualizer;
mod widgets;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use neo_ncmp_domain::{PlaybackState, format_duration};
use neo_ncmp_mpd::MpdClient;

#[derive(Debug, Parser)]
#[command(
    name = "neo-ncmp",
    version,
    about = "A modern, feature-rich terminal client for MPD"
)]
struct Cli {
    /// MPD host, overriding the configuration file.
    #[arg(long, value_name = "HOST", global = true)]
    host: Option<String>,

    /// MPD port, overriding the configuration file.
    #[arg(long, value_name = "PORT", global = true)]
    port: Option<u16>,

    /// Path to config.toml.
    #[arg(long, value_name = "PATH", global = true)]
    config: Option<PathBuf>,

    /// Connect to MPD, print a summary, and exit without starting the TUI.
    #[arg(long)]
    check: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Connect to MPD, print a summary, and exit.
    Check,

    /// Print the current playback state (machine-readable with --json).
    Status {
        #[arg(long)]
        json: bool,
    },

    /// Control a running MPD instance without starting the TUI.
    Remote {
        #[command(subcommand)]
        action: RemoteAction,
    },
}

#[derive(Debug, Subcommand)]
enum RemoteAction {
    Play,
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    /// Set an absolute volume (0-100) or a relative one (+5 / -5).
    Volume {
        value: String,
    },
    /// Seek to an absolute position in seconds, or a relative one (+5 / -5).
    Seek {
        value: String,
    },
    /// on | off | toggle
    Repeat {
        value: String,
    },
    /// on | off | toggle
    Random {
        value: String,
    },
    /// on | off | toggle
    Single {
        value: String,
    },
    /// on | off | toggle
    Consume {
        value: String,
    },
    Shuffle,
    Clear,
    /// Replace the queue with a stored playlist and play it.
    Playlist {
        name: String,
    },
    /// Enable or disable an output: `output 0 on`.
    Output {
        id: u32,
        state: String,
    },
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Piping CLI output into `head` should exit quietly, not panic on EPIPE.
    #[cfg(unix)]
    // SAFETY: restoring the default SIGPIPE disposition is process-wide and
    // has no memory-safety implications.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    let _log_guard = logging::init()?;

    let config_path = cli.config.clone().or_else(config::default_config_path);
    let mut config = config::Config::load(cli.config.as_deref())?;
    if let Some(host) = cli.host {
        config.mpd.host = host;
    }
    if let Some(port) = cli.port {
        config.mpd.port = port;
    }

    match cli.command {
        Some(Command::Check) => check(&config).await,
        Some(Command::Status { json }) => print_status(&config, json).await,
        Some(Command::Remote { action }) => remote(&config, action).await,
        None if cli.check => check(&config).await,
        None => {
            let mouse = config.ui.mouse;
            let mut terminal = ratatui::init();
            if mouse
                && let Err(error) =
                    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)
            {
                tracing::warn!(%error, "could not enable mouse capture");
            }
            let result = app::App::run(config, config_path, &mut terminal).await;
            if mouse {
                let _ =
                    crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture);
            }
            ratatui::restore();
            result
        }
    }
}

async fn connect(config: &config::Config) -> Result<MpdClient> {
    let mut client = MpdClient::connect(config.mpd_addr()).await?;
    if let Some(password) = &config.mpd.password {
        client.password(password).await?;
    }
    Ok(client)
}

async fn print_status(config: &config::Config, json: bool) -> Result<()> {
    let mut client = connect(config).await?;
    let status = client.status().await?;
    let current = client.current_song().await?;

    if json {
        let state = match status.state {
            PlaybackState::Playing => "playing",
            PlaybackState::Paused => "paused",
            PlaybackState::Stopped => "stopped",
        };
        let value = serde_json::json!({
            "state": state,
            "volume": status.volume,
            "elapsed": status.elapsed.as_secs_f64(),
            "duration": status.duration.map(|duration| duration.as_secs_f64()),
            "queue_length": status.playlist_length,
            "song_id": status.song_id,
            "song_pos": status.song_pos,
            "title": current.as_ref().map(|song| song.display_title()),
            "artist": current.as_ref().map(|song| song.display_artist()),
            "album": current.as_ref().map(|song| song.display_album()),
            "file": current.as_ref().map(|song| song.uri.to_string()),
            "repeat": status.repeat,
            "random": status.random,
            "single": status.single,
            "consume": status.consume,
        });
        println!("{}", serde_json::to_string(&value)?);
        return Ok(());
    }

    let state = match status.state {
        PlaybackState::Playing => "playing",
        PlaybackState::Paused => "paused",
        PlaybackState::Stopped => "stopped",
    };
    let volume = status
        .volume
        .map(|volume| format!("{volume}%"))
        .unwrap_or_else(|| "n/a".into());
    match &current {
        Some(song) => println!(
            "[{state}] {} — {} | {} / {} | vol {volume}",
            song.display_artist(),
            song.display_title(),
            format_duration(status.elapsed),
            status.duration_text(),
        ),
        None => println!("[{state}] idle | vol {volume}"),
    }
    Ok(())
}

async fn remote(config: &config::Config, action: RemoteAction) -> Result<()> {
    if let RemoteAction::Status { json } = action {
        return print_status(config, json).await;
    }

    let mut client = connect(config).await?;
    match action {
        RemoteAction::Play => client.play().await?,
        RemoteAction::Pause => client.pause(true).await?,
        RemoteAction::Toggle => {
            let status = client.status().await?;
            if status.state.is_playing() {
                client.pause(true).await?;
            } else {
                client.play().await?;
            }
        }
        RemoteAction::Stop => client.stop().await?,
        RemoteAction::Next => client.next().await?,
        RemoteAction::Previous => client.previous().await?,
        RemoteAction::Volume { value } => {
            let status = client.status().await?;
            let current = status.volume.unwrap_or(100);
            client.set_volume(parse_volume(&value, current)?).await?;
        }
        RemoteAction::Seek { value } => {
            let trimmed = value.trim();
            let numeric = trimmed.trim_start_matches(['+', '-']);
            if numeric.parse::<f64>().is_err() {
                bail!("seek expects seconds, optionally prefixed with + or -");
            }
            client.seek_current(trimmed).await?;
        }
        RemoteAction::Repeat { value } => {
            let status = client.status().await?;
            client
                .set_repeat(parse_switch(&value, status.repeat)?)
                .await?;
        }
        RemoteAction::Random { value } => {
            let status = client.status().await?;
            client
                .set_random(parse_switch(&value, status.random)?)
                .await?;
        }
        RemoteAction::Single { value } => {
            let status = client.status().await?;
            client
                .set_single(parse_switch(&value, status.single)?)
                .await?;
        }
        RemoteAction::Consume { value } => {
            let status = client.status().await?;
            client
                .set_consume(parse_switch(&value, status.consume)?)
                .await?;
        }
        RemoteAction::Shuffle => client.shuffle().await?,
        RemoteAction::Clear => client.clear().await?,
        RemoteAction::Playlist { name } => {
            client.clear().await?;
            client.load_playlist(&name).await?;
            let _ = client.play().await;
        }
        RemoteAction::Output { id, state } => match state.as_str() {
            "on" | "true" | "1" => client.enable_output(id).await?,
            "off" | "false" | "0" => client.disable_output(id).await?,
            other => bail!("expected on or off, got {other}"),
        },
        RemoteAction::Status { .. } => unreachable!("handled above"),
    }
    Ok(())
}

fn parse_volume(value: &str, current: u8) -> Result<u8> {
    let value = value.trim();
    if let Some(delta) = value.strip_prefix('+') {
        let delta: i16 = delta.parse()?;
        Ok((current as i16 + delta).clamp(0, 100) as u8)
    } else if let Some(delta) = value.strip_prefix('-') {
        let delta: i16 = delta.parse()?;
        Ok((current as i16 - delta).clamp(0, 100) as u8)
    } else {
        Ok(value.parse::<u8>()?.min(100))
    }
}

fn parse_switch(value: &str, current: bool) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" => Ok(true),
        "off" | "false" | "0" => Ok(false),
        "toggle" => Ok(!current),
        other => bail!("expected on, off, or toggle, got {other}"),
    }
}

async fn check(config: &config::Config) -> Result<()> {
    let mut client = connect(config).await?;
    let status = client.status().await?;
    let stats = client.stats().await?;
    let queue = client.queue().await?;
    let songs = client.list_all_info().await?;

    println!(
        "neo-ncmp {} — MPD {}",
        env!("CARGO_PKG_VERSION"),
        client.version()
    );
    println!("address:     {}", config.mpd_addr());
    println!("state:       {:?}", status.state);
    println!(
        "volume:      {}",
        status
            .volume
            .map(|volume| format!("{volume}%"))
            .unwrap_or_else(|| "n/a".into())
    );
    println!("queue:       {} songs", queue.len());
    let library = neo_ncmp_domain::Library::build(songs);
    println!(
        "library:     {} songs ({} artists, {} albums)",
        library.len(),
        library.artist_count(),
        library.album_count()
    );
    println!(
        "search:      {} items",
        search::SearchIndex::build(&library).len()
    );
    println!(
        "MPD stats:   {} artists, {} albums, {} songs",
        stats.artists, stats.albums, stats.songs
    );
    if let Some(song) = client.current_song().await? {
        println!(
            "now playing: {} — {}",
            song.display_artist(),
            song.display_title()
        );
        match client.read_picture(&song.uri).await? {
            Some(picture) => println!(
                "cover art:   {} bytes ({})",
                picture.data.len(),
                picture.mime
            ),
            None => println!("cover art:   none embedded"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_absolute_and_relative_volumes() {
        assert_eq!(parse_volume("80", 50).unwrap(), 80);
        assert_eq!(parse_volume("+5", 60).unwrap(), 65);
        assert_eq!(parse_volume("-10", 60).unwrap(), 50);
        assert_eq!(parse_volume("+500", 60).unwrap(), 100);
        assert_eq!(parse_volume("-500", 60).unwrap(), 0);
        assert!(parse_volume("loud", 60).is_err());
    }

    #[test]
    fn parses_on_off_toggle() {
        assert!(parse_switch("on", false).unwrap());
        assert!(!parse_switch("OFF", true).unwrap());
        assert!(!parse_switch("toggle", true).unwrap());
        assert!(parse_switch("toggle", false).unwrap());
        assert!(parse_switch("maybe", false).is_err());
    }
}
