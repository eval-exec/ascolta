use std::sync::Arc;

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use neo_ncmp_domain::{DirEntry, MpdOutput, MpdStats, MpdStatus, Song};

use crate::Subsystem;
use crate::error::{MpdError, Result};
use crate::parse::{self, Playlist};
use crate::protocol::{self, Response};

/// Album art larger than this is rejected rather than buffered.
const MAX_PICTURE_BYTES: usize = 16 * 1024 * 1024;

/// Maximum payload per `binary:` chunk requested from MPD.
const PICTURE_CHUNK_BYTES: u32 = 256 * 1024;

/// Image data returned by `readpicture` / `albumart`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub mime: Arc<str>,
    pub data: Vec<u8>,
}

/// A single connection to MPD.
///
/// MPD connections are stateful and strictly request/response: a client may
/// only have one command in flight. For concurrent work open another client
/// (the application uses one for commands and one for `idle`).
pub struct MpdClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    version: String,
}

impl std::fmt::Debug for MpdClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MpdClient")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl MpdClient {
    pub async fn connect(addr: impl tokio::net::ToSocketAddrs) -> Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        Self::from_stream(stream).await
    }

    pub async fn from_stream(stream: TcpStream) -> Result<Self> {
        stream.set_nodelay(true)?;
        let (read_half, write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let version = protocol::read_greeting(&mut reader).await?;
        Ok(Self {
            reader,
            writer: write_half,
            version,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    /// Sends a command and reads its response.
    pub async fn command(&mut self, command: &str, args: &[&str]) -> Result<Response> {
        let line = protocol::format_command(command, args)?;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        protocol::read_response(&mut self.reader).await
    }

    /// Sends a command and discards its (non-binary) response.
    pub async fn command_ok(&mut self, command: &str, args: &[&str]) -> Result<()> {
        self.command(command, args).await.map(|_| ())
    }

    pub async fn ping(&mut self) -> Result<()> {
        self.command_ok("ping", &[]).await
    }

    /// Authenticates this connection. Must be the first command if the server
    /// requires a password.
    pub async fn password(&mut self, password: &str) -> Result<()> {
        self.command_ok("password", &[password]).await
    }

    // ---------------------------------------------------------------- state

    pub async fn status(&mut self) -> Result<MpdStatus> {
        let response = self.command("status", &[]).await?;
        Ok(parse::status(&response))
    }

    pub async fn current_song(&mut self) -> Result<Option<Song>> {
        let response = self.command("currentsong", &[]).await?;
        Ok(parse::songs(&response).into_iter().next())
    }

    /// Reads the whole queue.
    pub async fn queue(&mut self) -> Result<Vec<Song>> {
        let response = self.command("playlistinfo", &[]).await?;
        Ok(parse::songs(&response))
    }

    /// Reads the whole database. MPD streams one `file:` block per song, so
    /// this is a single request even for large libraries.
    pub async fn list_all_info(&mut self) -> Result<Vec<Song>> {
        let response = self.command("listallinfo", &[]).await?;
        Ok(parse::songs(&response))
    }

    /// Lists the contents of one database directory.
    pub async fn list_info(&mut self, uri: &str) -> Result<Vec<Song>> {
        let response = self.command("lsinfo", &[uri]).await?;
        Ok(parse::songs(&response))
    }

    /// Lists a directory (or the database root) including subdirectories,
    /// stored playlists, and tagged files.
    pub async fn list_entries(&mut self, uri: Option<&str>) -> Result<Vec<DirEntry>> {
        let response = match uri {
            Some(uri) => self.command("lsinfo", &[uri]).await?,
            None => self.command("lsinfo", &[]).await?,
        };
        Ok(parse::entries(&response))
    }

    pub async fn outputs(&mut self) -> Result<Vec<MpdOutput>> {
        let response = self.command("outputs", &[]).await?;
        Ok(parse::outputs(&response))
    }

    pub async fn stats(&mut self) -> Result<MpdStats> {
        let response = self.command("stats", &[]).await?;
        Ok(parse::stats(&response))
    }

    pub async fn playlists(&mut self) -> Result<Vec<Playlist>> {
        let response = self.command("listplaylists", &[]).await?;
        Ok(parse::playlists(&response))
    }

    /// Returns the songs in a stored playlist.
    pub async fn playlist_info(&mut self, name: &str) -> Result<Vec<Song>> {
        let response = self.command("listplaylistinfo", &[name]).await?;
        Ok(parse::songs(&response))
    }

    /// Exact-match database search. Use `tag`/`value` pairs, e.g.
    /// `[("Artist", "Portishead")]`.
    pub async fn find(&mut self, filters: &[(&str, &str)]) -> Result<Vec<Song>> {
        let args = flatten_filters(filters);
        let response = self.command("find", &args).await?;
        Ok(parse::songs(&response))
    }

    /// Substring database search. Use `tag`/`value` pairs.
    pub async fn search(&mut self, filters: &[(&str, &str)]) -> Result<Vec<Song>> {
        let args = flatten_filters(filters);
        let response = self.command("search", &args).await?;
        Ok(parse::songs(&response))
    }

    // ------------------------------------------------------------ transport

    pub async fn play(&mut self) -> Result<()> {
        self.command_ok("play", &[]).await
    }

    pub async fn play_pos(&mut self, pos: u32) -> Result<()> {
        self.command_ok("play", &[&pos.to_string()]).await
    }

    pub async fn play_id(&mut self, id: u32) -> Result<()> {
        self.command_ok("playid", &[&id.to_string()]).await
    }

    pub async fn pause(&mut self, paused: bool) -> Result<()> {
        self.command_ok("pause", &[if paused { "1" } else { "0" }])
            .await
    }

    /// Toggles pause; MPD 0.23 and newer accept `pause` without an argument.
    pub async fn toggle_pause(&mut self) -> Result<()> {
        self.command_ok("pause", &[]).await
    }

    pub async fn stop(&mut self) -> Result<()> {
        self.command_ok("stop", &[]).await
    }

    pub async fn next(&mut self) -> Result<()> {
        self.command_ok("next", &[]).await
    }

    pub async fn previous(&mut self) -> Result<()> {
        self.command_ok("previous", &[]).await
    }

    /// Seeks relatively: `+5`, `-5`, or absolute `30`.
    pub async fn seek_current(&mut self, position: &str) -> Result<()> {
        self.command_ok("seekcur", &[position]).await
    }

    pub async fn set_volume(&mut self, volume: u8) -> Result<()> {
        let volume = volume.min(100).to_string();
        self.command_ok("setvol", &[&volume]).await
    }

    pub async fn set_repeat(&mut self, enabled: bool) -> Result<()> {
        self.command_ok("repeat", &[bool_arg(enabled)]).await
    }

    pub async fn set_random(&mut self, enabled: bool) -> Result<()> {
        self.command_ok("random", &[bool_arg(enabled)]).await
    }

    pub async fn set_single(&mut self, enabled: bool) -> Result<()> {
        self.command_ok("single", &[bool_arg(enabled)]).await
    }

    pub async fn set_consume(&mut self, enabled: bool) -> Result<()> {
        self.command_ok("consume", &[bool_arg(enabled)]).await
    }

    // ---------------------------------------------------------------- queue

    pub async fn add(&mut self, uri: &str) -> Result<()> {
        self.command_ok("add", &[uri]).await
    }

    /// Adds a song and returns its new queue id.
    pub async fn add_id(&mut self, uri: &str, position: Option<u32>) -> Result<u32> {
        let position = position.map(|pos| pos.to_string());
        let mut args = vec![uri];
        if let Some(position) = position.as_deref() {
            args.push(position);
        }
        let response = self.command("addid", &args).await?;
        response
            .get("Id")
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| MpdError::Protocol("addid returned no Id".into()))
    }

    pub async fn delete_id(&mut self, id: u32) -> Result<()> {
        self.command_ok("deleteid", &[&id.to_string()]).await
    }

    pub async fn move_id(&mut self, id: u32, to: u32) -> Result<()> {
        self.command_ok("moveid", &[&id.to_string(), &to.to_string()])
            .await
    }

    pub async fn clear(&mut self) -> Result<()> {
        self.command_ok("clear", &[]).await
    }

    pub async fn shuffle(&mut self) -> Result<()> {
        self.command_ok("shuffle", &[]).await
    }

    pub async fn update(&mut self, uri: Option<&str>) -> Result<Option<u32>> {
        let response = match uri {
            Some(uri) => self.command("update", &[uri]).await?,
            None => self.command("update", &[]).await?,
        };
        Ok(response.get("updating_db").and_then(|id| id.parse().ok()))
    }

    // -------------------------------------------------------------- outputs

    pub async fn enable_output(&mut self, id: u32) -> Result<()> {
        self.command_ok("enableoutput", &[&id.to_string()]).await
    }

    pub async fn disable_output(&mut self, id: u32) -> Result<()> {
        self.command_ok("disableoutput", &[&id.to_string()]).await
    }

    // ------------------------------------------------------------ playlists

    pub async fn save_playlist(&mut self, name: &str) -> Result<()> {
        self.command_ok("save", &[name]).await
    }

    pub async fn load_playlist(&mut self, name: &str) -> Result<()> {
        self.command_ok("load", &[name]).await
    }

    pub async fn remove_playlist(&mut self, name: &str) -> Result<()> {
        self.command_ok("rm", &[name]).await
    }

    pub async fn rename_playlist(&mut self, from: &str, to: &str) -> Result<()> {
        self.command_ok("rename", &[from, to]).await
    }

    pub async fn playlist_append(&mut self, name: &str, uri: &str) -> Result<()> {
        self.command_ok("playlistadd", &[name, uri]).await
    }

    pub async fn playlist_delete(&mut self, name: &str, position: u32) -> Result<()> {
        self.command_ok("playlistdelete", &[name, &position.to_string()])
            .await
    }

    pub async fn playlist_move(&mut self, name: &str, from: u32, to: u32) -> Result<()> {
        self.command_ok("playlistmove", &[name, &from.to_string(), &to.to_string()])
            .await
    }

    pub async fn playlist_clear(&mut self, name: &str) -> Result<()> {
        self.command_ok("playlistclear", &[name]).await
    }

    // -------------------------------------------------------------- stickers

    pub async fn sticker_set(&mut self, uri: &str, key: &str, value: &str) -> Result<()> {
        self.command_ok("sticker", &["set", "song", uri, key, value])
            .await
    }

    pub async fn sticker_get(&mut self, uri: &str, key: &str) -> Result<Option<String>> {
        let response = self.command("sticker", &["get", "song", uri, key]).await?;
        Ok(response
            .get("sticker")
            .and_then(|sticker| sticker.split_once('='))
            .map(|(_, value)| value.to_owned()))
    }

    pub async fn sticker_delete(&mut self, uri: &str, key: &str) -> Result<()> {
        self.command_ok("sticker", &["delete", "song", uri, key])
            .await
    }

    // ------------------------------------------------------------- pictures

    /// Reads embedded cover art for a song via MPD's `readpicture`.
    pub async fn read_picture(&mut self, uri: &str) -> Result<Option<Picture>> {
        self.picture_request("readpicture", uri).await
    }

    /// Reads cover art associated with a song or album directory via
    /// MPD's `albumart`.
    pub async fn album_art(&mut self, uri: &str) -> Result<Option<Picture>> {
        self.picture_request("albumart", uri).await
    }

    async fn picture_request(&mut self, command: &str, uri: &str) -> Result<Option<Picture>> {
        match self
            .command_ok("binarylimit", &[&PICTURE_CHUNK_BYTES.to_string()])
            .await
        {
            Ok(()) | Err(MpdError::Ack { .. }) => {}
            Err(error) => return Err(error),
        }

        let mut data = Vec::new();
        let mut mime: Arc<str> = Arc::from("application/octet-stream");
        let mut total_size = None;

        while data.len() < MAX_PICTURE_BYTES {
            let offset = data.len().to_string();
            let response = match self.command(command, &[uri, &offset]).await {
                Ok(response) => response,
                // A song without a picture (or an unknown URI) is not an error.
                Err(MpdError::Ack { code: 50, .. }) => return Ok(None),
                Err(MpdError::Ack { message, .. })
                    if message.to_ascii_lowercase().contains("no such") =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };

            if let Some(size) = response.get("size") {
                total_size = size.parse::<usize>().ok();
            }
            if let Some(kind) = response.get("type") {
                mime = Arc::from(kind);
            }
            if total_size == Some(0) {
                return Ok(None);
            }

            let chunk = response.binary();
            if chunk.is_empty() {
                break;
            }
            data.extend_from_slice(chunk);
            if total_size.is_some_and(|size| data.len() >= size) {
                break;
            }
        }

        if data.is_empty() {
            return Ok(None);
        }
        if data.len() > MAX_PICTURE_BYTES {
            return Err(MpdError::TooLarge(MAX_PICTURE_BYTES));
        }
        Ok(Some(Picture { mime, data }))
    }

    // ----------------------------------------------------------------- idle

    /// Blocks until MPD reports a change in one of `subsystems` (empty means
    /// all subsystems). Returns the set of changed subsystems.
    pub async fn idle(&mut self, subsystems: &[Subsystem]) -> Result<Vec<Subsystem>> {
        let args: Vec<&str> = subsystems
            .iter()
            .filter(|subsystem| **subsystem != Subsystem::Unknown)
            .map(|subsystem| subsystem.as_str())
            .collect();
        let response = self.command("idle", &args).await?;
        Ok(response
            .pairs()
            .iter()
            .filter(|(key, _)| key == "changed")
            .map(|(_, value)| Subsystem::parse(value))
            .collect())
    }
}

fn bool_arg(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn flatten_filters<'a>(filters: &'a [(&'a str, &'a str)]) -> Vec<&'a str> {
    let mut args = Vec::with_capacity(filters.len() * 2);
    for (tag, value) in filters {
        args.push(*tag);
        args.push(*value);
    }
    args
}
