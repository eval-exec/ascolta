//! A small, asynchronous client for the MPD protocol.
//!
//! The protocol is line oriented: commands are sent as one line, responses are
//! `key: value` pairs terminated by `OK`, and failures are reported as
//! `ACK` lines. A few commands stream binary data. This crate keeps the wire
//! handling isolated from the UI so it can be tested against a mock server and
//! against a real daemon.

mod client;
mod error;
mod parse;
mod protocol;

pub use client::{MpdClient, Picture};
pub use error::{MpdError, Result};
pub use parse::Playlist;
pub use protocol::Response;

pub use neo_ncmp_domain as domain;

pub use domain::{
    AudioFormat, DirEntry, EntryKind, Library, MpdOutput, MpdStats, MpdStatus, PlaybackState,
    QueuePos, Song, SongId,
};

/// A subsystem reported by MPD's `idle` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subsystem {
    Database,
    Update,
    StoredPlaylist,
    Playlist,
    Player,
    Mixer,
    Output,
    Options,
    Partition,
    Sticker,
    Subscription,
    Message,
    Unknown,
}

impl Subsystem {
    pub fn parse(value: &str) -> Self {
        match value {
            "database" => Self::Database,
            "update" => Self::Update,
            "stored_playlist" => Self::StoredPlaylist,
            "playlist" => Self::Playlist,
            "player" => Self::Player,
            "mixer" => Self::Mixer,
            "output" => Self::Output,
            "options" => Self::Options,
            "partition" => Self::Partition,
            "sticker" => Self::Sticker,
            "subscription" => Self::Subscription,
            "message" => Self::Message,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Update => "update",
            Self::StoredPlaylist => "stored_playlist",
            Self::Playlist => "playlist",
            Self::Player => "player",
            Self::Mixer => "mixer",
            Self::Output => "output",
            Self::Options => "options",
            Self::Partition => "partition",
            Self::Sticker => "sticker",
            Self::Subscription => "subscription",
            Self::Message => "message",
            Self::Unknown => "",
        }
    }
}
