use std::sync::Arc;

use neo_ncmp_mpd::{MpdClient, MpdError};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const GREETING: &str = "OK MPD 0.24.0\n";
const STATUS: &str = "\
volume: 100
repeat: 1
random: 0
single: 0
consume: 0
partition: default
playlist: 3
playlistlength: 2
state: play
song: 0
songid: 1
elapsed: 12.5
duration: 100.0
bitrate: 320
audio: 44100:16:2
OK
";
const QUEUE: &str = "\
file: a/one.flac
Artist: Portishead
Album: Dummy
Title: Mysterons
Track: 1
duration: 100.0
Pos: 0
Id: 1
file: a/two.flac
Artist: Portishead
Album: Dummy
Title: Sour Times
Track: 2
duration: 120.0
Pos: 1
Id: 2
OK
";
const PICTURE: &str = "size: 3\ntype: image/png\nbinary: 3\nabc\nOK\n";

#[tokio::test]
async fn talks_to_a_mock_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        write_half.write_all(GREETING.as_bytes()).await.unwrap();

        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap() == 0 {
                break;
            }
            let command = line.split_whitespace().next().unwrap_or("");
            let response = match command {
                "status" => STATUS,
                "playlistinfo" => QUEUE,
                "binarylimit" => "OK\n",
                "readpicture" => PICTURE,
                "idle" => "changed: player\nOK\n",
                "ping" => "OK\n",
                _ => "ACK [5@0] {unknown} unknown command\n",
            };
            write_half.write_all(response.as_bytes()).await.unwrap();
            if command == "ping" {
                break;
            }
        }
    });

    let mut client = MpdClient::connect(addr).await.unwrap();
    assert_eq!(client.version(), "0.24.0");

    let status = client.status().await.unwrap();
    assert_eq!(status.volume, Some(100));
    assert!(status.repeat);
    assert_eq!(status.playlist_length, 2);
    assert_eq!(status.song_id, Some(1));

    let queue = client.queue().await.unwrap();
    assert_eq!(queue.len(), 2);
    assert_eq!(queue[0].display_title(), "Mysterons");
    assert_eq!(queue[1].id, Some(2));

    let picture = client.read_picture("a/one.flac").await.unwrap().unwrap();
    assert_eq!(picture.mime.as_ref(), "image/png");
    assert_eq!(picture.data, b"abc");

    let changed = client.idle(&[]).await.unwrap();
    assert_eq!(changed, vec![neo_ncmp_mpd::Subsystem::Player]);

    let error = client.command("bogus", &[]).await.unwrap_err();
    assert!(matches!(error, MpdError::Ack { code: 5, .. }));

    client.ping().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn picture_data_is_shared_as_arc_str_for_mime() {
    // Guards the public Picture shape used by the UI's art cache.
    let picture = neo_ncmp_mpd::Picture {
        mime: Arc::from("image/jpeg"),
        data: vec![0xff, 0xd8],
    };
    assert_eq!(picture.mime.as_ref(), "image/jpeg");
}
