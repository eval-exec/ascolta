//! Domain types shared by the MPD client and the user interface.
//!
//! This crate is intentionally free of I/O and terminal dependencies so that the
//! data model can be tested in isolation.

pub mod library;
pub mod song;
pub mod status;

pub use library::{Album, Artist, Library};
pub use song::{QueuePos, Song, SongId, format_duration};
pub use status::{AudioFormat, MpdOutput, MpdStats, MpdStatus, PlaybackState};
