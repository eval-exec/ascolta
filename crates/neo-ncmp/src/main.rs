mod app;
mod config;
mod input;
mod logging;
mod mpd_worker;
mod search;
mod ui;
mod widgets;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use neo_ncmp_mpd::MpdClient;

#[derive(Debug, Parser)]
#[command(
    name = "neo-ncmp",
    version,
    about = "A modern, feature-rich terminal client for MPD"
)]
struct Cli {
    /// MPD host, overriding the configuration file.
    #[arg(long, value_name = "HOST")]
    host: Option<String>,

    /// MPD port, overriding the configuration file.
    #[arg(long, value_name = "PORT")]
    port: Option<u16>,

    /// Path to config.toml.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Connect to MPD, print a summary, and exit without starting the TUI.
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let _log_guard = logging::init()?;

    let mut config = config::Config::load(cli.config.as_deref())?;
    if let Some(host) = cli.host {
        config.mpd.host = host;
    }
    if let Some(port) = cli.port {
        config.mpd.port = port;
    }

    if cli.check {
        return check(&config).await;
    }

    let mut terminal = ratatui::init();
    let result = app::App::run(config, &mut terminal).await;
    ratatui::restore();
    result
}

async fn check(config: &config::Config) -> Result<()> {
    let mut client = MpdClient::connect(config.mpd_addr()).await?;
    if let Some(password) = &config.mpd.password {
        client.password(password).await?;
    }
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
