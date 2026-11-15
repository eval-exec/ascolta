use std::sync::Arc;

use crate::Song;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    Playlist,
    File,
}

/// One record from MPD's `lsinfo`.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub kind: EntryKind,
    pub uri: Arc<str>,
    pub name: Arc<str>,
    /// Present for files, carrying tags when MPD supplied them.
    pub song: Option<Song>,
}
