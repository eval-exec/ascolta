use std::collections::BTreeMap;
use std::sync::Arc;

use crate::song::Song;

/// An in-memory index of the MPD database.
///
/// MPD's own database is the source of truth; this is a read-only snapshot
/// optimised for instant browsing, filtering, and fuzzy search. It is rebuilt
/// whenever MPD reports a database change.
#[derive(Debug, Clone, Default)]
pub struct Library {
    songs: Vec<Song>,
    artists: Vec<Artist>,
    album_count: usize,
}

#[derive(Debug, Clone)]
pub struct Artist {
    pub name: Arc<str>,
    pub albums: Vec<Album>,
}

#[derive(Debug, Clone)]
pub struct Album {
    pub name: Arc<str>,
    pub artist: Arc<str>,
    pub year: Option<Arc<str>>,
    /// Release date from the first tagged track, used for newest-first sorting.
    pub date: Option<Arc<str>>,
    /// Indices into [`Library::songs`], sorted by disc and track number.
    pub songs: Vec<usize>,
}

impl Artist {
    pub fn song_count(&self) -> usize {
        self.albums.iter().map(|album| album.songs.len()).sum()
    }
}

impl Library {
    pub fn build(songs: Vec<Song>) -> Self {
        let mut grouped: BTreeMap<Arc<str>, BTreeMap<Arc<str>, Vec<usize>>> = BTreeMap::new();
        for (index, song) in songs.iter().enumerate() {
            let artist: Arc<str> = Arc::from(song.grouping_artist());
            let album: Arc<str> = Arc::from(song.display_album());
            grouped
                .entry(artist)
                .or_default()
                .entry(album)
                .or_default()
                .push(index);
        }

        let mut artists = Vec::with_capacity(grouped.len());
        let mut album_count = 0;
        for (name, albums) in grouped {
            let mut artist_albums = Vec::with_capacity(albums.len());
            for (album_name, mut indices) in albums {
                indices.sort_by_key(|&index| {
                    let song = &songs[index];
                    (
                        song.disc.unwrap_or(0),
                        song.track.unwrap_or(0),
                        song.display_title().to_lowercase(),
                    )
                });
                let year = indices
                    .iter()
                    .find_map(|&index| songs[index].year().map(Arc::from));
                let date = indices.iter().find_map(|&index| {
                    songs[index]
                        .date
                        .as_deref()
                        .filter(|date| !date.is_empty())
                        .map(Arc::from)
                });
                album_count += 1;
                artist_albums.push(Album {
                    name: album_name,
                    artist: name.clone(),
                    year,
                    date,
                    songs: indices,
                });
            }
            // Newest releases first; undated albums follow alphabetically.
            artist_albums.sort_by(|left, right| {
                let order = match (left.date.as_deref(), right.date.as_deref()) {
                    (Some(left), Some(right)) => right.cmp(left),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                };
                order.then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            });
            artists.push(Artist {
                name,
                albums: artist_albums,
            });
        }

        // Album/artist names are usually presented case-insensitively, the
        // way a music player should, not in ASCII order.
        artists.sort_by_key(|artist| artist.name.to_lowercase());

        Self {
            songs,
            artists,
            album_count,
        }
    }

    pub fn songs(&self) -> &[Song] {
        &self.songs
    }

    pub fn artists(&self) -> &[Artist] {
        &self.artists
    }

    pub fn song(&self, index: usize) -> Option<&Song> {
        self.songs.get(index)
    }

    pub fn len(&self) -> usize {
        self.songs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.songs.is_empty()
    }

    pub fn artist_count(&self) -> usize {
        self.artists.len()
    }

    pub fn album_count(&self) -> usize {
        self.album_count
    }

    /// URIs for every song belonging to an artist, in album order.
    pub fn artist_uris(&self, artist: usize) -> impl Iterator<Item = &str> {
        self.artists
            .get(artist)
            .into_iter()
            .flat_map(|artist| artist.albums.iter())
            .flat_map(|album| album.songs.iter())
            .filter_map(|&index| self.songs.get(index))
            .map(|song| song.uri.as_ref())
    }

    pub fn album_uris(&self, artist: usize, album: usize) -> impl Iterator<Item = &str> {
        self.artists
            .get(artist)
            .and_then(|artist| artist.albums.get(album))
            .into_iter()
            .flat_map(|album| album.songs.iter())
            .filter_map(|&index| self.songs.get(index))
            .map(|song| song.uri.as_ref())
    }

    pub fn album_songs(&self, artist: usize, album: usize) -> impl Iterator<Item = &Song> {
        self.artists
            .get(artist)
            .and_then(|artist| artist.albums.get(album))
            .into_iter()
            .flat_map(|album| album.songs.iter())
            .filter_map(|&index| self.songs.get(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(artist: &str, album: &str, track: u32, title: &str) -> Song {
        Song {
            uri: Arc::from(format!("{artist}/{album}/{track:02} - {title}.flac").as_str()),
            artist: Some(Arc::from(artist)),
            album: Some(Arc::from(album)),
            track: Some(track),
            title: Some(Arc::from(title)),
            ..Song::default()
        }
    }

    #[test]
    fn groups_and_sorts_tracks() {
        let library = Library::build(vec![
            song("Portishead", "Dummy", 2, "Sour Times"),
            song("Portishead", "Dummy", 1, "Mysterons"),
            song("Massive Attack", "Mezzanine", 1, "Angel"),
        ]);

        assert_eq!(library.len(), 3);
        assert_eq!(library.artist_count(), 2);
        assert_eq!(library.album_count(), 2);
        assert_eq!(library.artists()[0].name.as_ref(), "Massive Attack");
        assert_eq!(library.artists()[1].name.as_ref(), "Portishead");

        let dummy = &library.artists()[1].albums[0];
        let titles: Vec<_> = dummy
            .songs
            .iter()
            .map(|&index| library.song(index).unwrap().display_title())
            .collect();
        assert_eq!(titles, ["Mysterons", "Sour Times"]);
    }

    #[test]
    fn groups_compilations_by_album_artist() {
        let mut first = song("Artist A", "Compilation", 1, "One");
        first.album_artist = Some(Arc::from("Various Artists"));
        let mut second = song("Artist B", "Compilation", 2, "Two");
        second.album_artist = Some(Arc::from("Various Artists"));

        let library = Library::build(vec![first, second]);
        assert_eq!(library.artist_count(), 1);
        assert_eq!(library.artists()[0].name.as_ref(), "Various Artists");
        assert_eq!(library.artists()[0].albums[0].songs.len(), 2);
    }

    #[test]
    fn sorts_albums_by_newest_release_date() {
        let mut old = song("Portishead", "Dummy", 1, "Mysterons");
        old.date = Some(Arc::from("1994-08-22"));
        let mut new = song("Portishead", "Third", 1, "Silence");
        new.date = Some(Arc::from("2008-04-28"));
        let undated = song("Portishead", "Rarities", 1, "B-Side");
        let library = Library::build(vec![old, new, undated]);
        let albums: Vec<_> = library.artists()[0]
            .albums
            .iter()
            .map(|album| album.name.as_ref())
            .collect();
        assert_eq!(albums, ["Third", "Dummy", "Rarities"]);
    }

    #[test]
    fn sorts_names_case_insensitively() {
        let library = Library::build(vec![
            song("zz top", "Afterburner", 1, "Sleeping Bag"),
            song("ABBA", "Arrival", 1, "Dancing Queen"),
            song("abba", "The Album", 1, "Take a Chance"),
        ]);
        let names: Vec<_> = library
            .artists()
            .iter()
            .map(|artist| artist.name.as_ref())
            .collect();
        assert_eq!(names, ["ABBA", "abba", "zz top"]);
        let abba_albums: Vec<_> = library.artists()[0]
            .albums
            .iter()
            .map(|album| album.name.as_ref())
            .collect();
        assert_eq!(abba_albums, ["Arrival"]);
    }

    #[test]
    fn unknown_tags_get_placeholders() {
        let mut track = song("", "", 0, "Mystery");
        track.artist = None;
        track.album = None;
        track.title = None;
        let library = Library::build(vec![track]);
        assert_eq!(library.artists()[0].name.as_ref(), "Unknown Artist");
        assert_eq!(
            library.artists()[0].albums[0].name.as_ref(),
            "Unknown Album"
        );
    }
}
