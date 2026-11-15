//! Tests that require a real MPD instance.
//!
//! Run with:
//!   cargo test -p ascolta-mpd --test live -- --ignored --nocapture
//!
//! Connection settings honour MPD_HOST and MPD_PORT, matching the MPD
//! convention, and default to 127.0.0.1:6600.

use ascolta_mpd::MpdClient;

fn mpd_addr() -> String {
    let host = std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("MPD_PORT").unwrap_or_else(|_| "6600".into());
    format!("{host}:{port}")
}

#[tokio::test]
#[ignore = "requires a running MPD instance"]
async fn reads_live_status_queue_and_library() {
    let mut client = MpdClient::connect(mpd_addr()).await.unwrap();
    println!("connected to MPD {}", client.version());

    let status = client.status().await.unwrap();
    println!(
        "state={:?} volume={:?} queue={} elapsed={}",
        status.state,
        status.volume,
        status.playlist_length,
        status.elapsed_text()
    );

    let queue = client.queue().await.unwrap();
    assert_eq!(queue.len() as u32, status.playlist_length);
    if let Some(song) = queue.first() {
        println!(
            "queue[0] = {} — {} ({})",
            song.display_artist(),
            song.display_title(),
            song.uri
        );
    }

    let stats = client.stats().await.unwrap();
    println!(
        "library: {} artists, {} albums, {} songs",
        stats.artists, stats.albums, stats.songs
    );

    let songs = client.list_all_info().await.unwrap();
    println!("listallinfo returned {} songs", songs.len());

    if let Some(current) = client.current_song().await.unwrap() {
        match client.read_picture(&current.uri).await.unwrap() {
            Some(picture) => println!("cover art: {} bytes, {}", picture.data.len(), picture.mime),
            None => println!("cover art: none embedded"),
        }
    }

    for output in client.outputs().await.unwrap() {
        println!(
            "output {}: {} ({})",
            output.id,
            output.name,
            if output.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
    }

    assert!(!client.version().is_empty());
}
