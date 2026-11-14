//! Library-wide fuzzy search.
//!
//! A snapshot index is built whenever the library is (re)loaded. Matching uses
//! the same algorithm as fzf via `nucleo-matcher`; the UI only ever renders the
//! visible slice of results, so even very large libraries stay responsive.

use std::sync::Arc;

use neo_ncmp_domain::Library;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use tokio::sync::mpsc;

use crate::mpd_worker::Msg;
use crate::widgets::ListView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Artist,
    Album,
    Song,
}

impl SearchKind {
    /// Single-character marker used by the result list.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Artist => "A",
            Self::Album => "L",
            Self::Song => "S",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchItem {
    pub kind: SearchKind,
    pub title: String,
    pub detail: String,
    /// Normalised text that the fuzzy matcher scores against.
    pub haystack: String,
    /// Everything that should be enqueued if this item is chosen.
    pub uris: Vec<Arc<str>>,
}

#[derive(Debug, Default)]
pub struct SearchIndex {
    pub items: Vec<SearchItem>,
}

impl SearchIndex {
    pub fn build(library: &Library) -> Self {
        let mut items =
            Vec::with_capacity(library.len() + library.album_count() + library.artist_count());

        for artist in library.artists() {
            let tracks = artist.song_count();
            items.push(SearchItem {
                kind: SearchKind::Artist,
                title: artist.name.to_string(),
                detail: format!(
                    "{} album{} · {} track{}",
                    artist.albums.len(),
                    plural(artist.albums.len()),
                    tracks,
                    plural(tracks)
                ),
                haystack: artist.name.to_string(),
                uris: artist
                    .albums
                    .iter()
                    .flat_map(|album| album.songs.iter())
                    .filter_map(|&index| library.song(index))
                    .map(|song| song.uri.clone())
                    .collect(),
            });

            for album in &artist.albums {
                items.push(SearchItem {
                    kind: SearchKind::Album,
                    title: album.name.to_string(),
                    detail: format!(
                        "{} · {} track{}",
                        artist.name,
                        album.songs.len(),
                        plural(album.songs.len())
                    ),
                    haystack: format!(
                        "{} {} {}",
                        album.name,
                        artist.name,
                        album.year.as_deref().unwrap_or_default()
                    ),
                    uris: album
                        .songs
                        .iter()
                        .filter_map(|&index| library.song(index))
                        .map(|song| song.uri.clone())
                        .collect(),
                });
            }
        }

        for song in library.songs() {
            items.push(SearchItem {
                kind: SearchKind::Song,
                title: song.display_title().to_string(),
                detail: format!("{} · {}", song.display_artist(), song.display_album()),
                haystack: format!(
                    "{} {} {} {}",
                    song.display_title(),
                    song.display_artist(),
                    song.display_album(),
                    song.uri
                ),
                uris: vec![song.uri.clone()],
            });
        }

        Self { items }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Scores every item against `query`, best match first.
    ///
    /// Pure and CPU-bound: the UI runs this on a blocking worker so keystrokes
    /// are never delayed, regardless of library size.
    pub fn score(&self, query: &str) -> Vec<usize> {
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buffer = Vec::new();

        let mut scored: Vec<(u32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(position, item)| {
                pattern
                    .score(Utf32Str::new(&item.haystack, &mut buffer), &mut matcher)
                    .map(|score| (score, position))
            })
            .collect();

        // Highest score first; ties keep index order, which is artists,
        // albums, then songs.
        scored.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        scored.into_iter().map(|(_, position)| position).collect()
    }
}

struct SearchRequest {
    generation: u64,
    index: Arc<SearchIndex>,
    query: String,
}

/// Handle to the search worker.
///
/// Requests are coalesced: if several keystrokes arrive while a pass is
/// running, only the newest query is scored, and stale results are discarded
/// by the application via the generation counter.
#[derive(Clone)]
pub struct SearchHandle {
    tx: mpsc::UnboundedSender<SearchRequest>,
}

impl SearchHandle {
    pub fn spawn(out: mpsc::UnboundedSender<Msg>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<SearchRequest>();
        tokio::spawn(async move {
            while let Some(mut request) = rx.recv().await {
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                let SearchRequest {
                    generation,
                    index,
                    query,
                } = request;
                match tokio::task::spawn_blocking(move || index.score(&query)).await {
                    Ok(results) => {
                        if out
                            .send(Msg::SearchResults {
                                generation,
                                results,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(error) => tracing::warn!(%error, "search worker panicked"),
                }
            }
        });
        Self { tx }
    }

    pub fn request(&self, generation: u64, index: Arc<SearchIndex>, query: String) {
        let _ = self.tx.send(SearchRequest {
            generation,
            index,
            query,
        });
    }

    #[cfg(test)]
    pub(crate) fn test_handle() -> Self {
        let (tx, _rx) = mpsc::unbounded_channel();
        Self { tx }
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

#[derive(Debug, Default)]
pub struct SearchState {
    pub active: bool,
    pub pending: bool,
    pub query: String,
    /// Indices into [`SearchIndex::items`], best match first.
    pub results: Vec<usize>,
    pub view: ListView,
}

impl SearchState {
    pub fn open(&mut self) {
        self.active = true;
        self.pending = true;
        self.query.clear();
        self.results.clear();
        self.view.reset();
    }

    pub fn close(&mut self) {
        self.active = false;
        self.pending = false;
    }

    /// Synchronous scoring, used by tests.
    #[cfg(test)]
    pub fn update(&mut self, index: &SearchIndex) {
        self.results = index.score(&self.query);
        self.pending = false;
        self.view.reset();
    }

    /// Applies a result set from the background worker.
    pub fn apply(&mut self, results: Vec<usize>) {
        self.results = results;
        self.pending = false;
        self.view.reset();
    }

    pub fn move_cursor(&mut self, delta: isize) {
        self.view.move_by(delta, self.results.len());
    }

    pub fn selected<'a>(&self, index: &'a SearchIndex) -> Option<&'a SearchItem> {
        self.results
            .get(self.view.cursor)
            .and_then(|&position| index.items.get(position))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_ncmp_domain::Song;

    fn song(artist: &str, album: &str, title: &str) -> Song {
        Song {
            uri: Arc::from(format!("{artist}/{album}/{title}.flac")),
            artist: Some(Arc::from(artist)),
            album: Some(Arc::from(album)),
            title: Some(Arc::from(title)),
            ..Song::default()
        }
    }

    fn index() -> SearchIndex {
        SearchIndex::build(&Library::build(vec![
            song("Portishead", "Dummy", "Mysterons"),
            song("Portishead", "Dummy", "Sour Times"),
            song("Massive Attack", "Mezzanine", "Angel"),
        ]))
    }

    #[test]
    fn indexes_artists_albums_and_songs() {
        let index = index();
        assert_eq!(index.len(), 2 + 2 + 3);
        assert_eq!(
            index
                .items
                .iter()
                .filter(|item| item.kind == SearchKind::Artist)
                .count(),
            2
        );
        assert_eq!(
            index
                .items
                .iter()
                .filter(|item| item.kind == SearchKind::Album)
                .count(),
            2
        );
    }

    #[test]
    fn fuzzy_search_ranks_the_obvious_song_first() {
        let index = index();
        let mut state = SearchState {
            query: "sour".into(),
            ..SearchState::default()
        };
        state.update(&index);
        let selected = state.selected(&index).expect("a match");
        assert_eq!(selected.kind, SearchKind::Song);
        assert_eq!(selected.title, "Sour Times");
    }

    #[test]
    fn search_matches_artist_and_album_fields() {
        let index = index();
        let mut state = SearchState {
            query: "mezzanine".into(),
            ..SearchState::default()
        };
        state.update(&index);
        assert!(
            state
                .results
                .iter()
                .any(|&position| index.items[position].kind == SearchKind::Album)
        );
        assert!(
            state
                .results
                .iter()
                .any(|&position| index.items[position].kind == SearchKind::Song)
        );
    }

    #[test]
    fn empty_query_lists_everything() {
        let index = index();
        let mut state = SearchState::default();
        state.update(&index);
        assert_eq!(state.results.len(), index.len());
    }

    #[test]
    fn cursor_movement_is_bounded() {
        let index = index();
        let mut state = SearchState::default();
        state.update(&index);
        state.move_cursor(-10);
        assert_eq!(state.view.cursor, 0);
        state.move_cursor(10_000);
        assert_eq!(state.view.cursor, index.len() - 1);
    }

    #[test]
    fn pluralises_counts() {
        assert_eq!(plural(0), "s");
        assert_eq!(plural(1), "");
        assert_eq!(plural(2), "s");
    }
}
