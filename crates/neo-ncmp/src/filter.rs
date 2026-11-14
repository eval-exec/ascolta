//! Structured filters for library search.
//!
//! A query is a mix of fuzzy terms and field filters:
//!
//! ```text
//! artist:portishead album:"third" year:>=2008 -genre:rock remix
//! ```
//!
//! Bare words stay fuzzy terms; `field:value` narrows the candidate set first.
//! Prefix a filter with `-` to exclude matches. Values may be quoted.

/// A song or library entity field that a filter can address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Year,
    Format,
    Uri,
}

impl Field {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "title" | "t" => Some(Self::Title),
            "artist" | "a" => Some(Self::Artist),
            "album" | "al" => Some(Self::Album),
            "albumartist" | "album-artist" | "aa" => Some(Self::AlbumArtist),
            "genre" | "g" => Some(Self::Genre),
            "year" | "date" | "y" => Some(Self::Year),
            "format" | "f" => Some(Self::Format),
            "uri" | "path" => Some(Self::Uri),
            _ => None,
        }
    }
}

/// A numeric comparison on the year extracted from the `Date` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Year {
    Eq(u32),
    Gte(u32),
    Lte(u32),
    Gt(u32),
    Lt(u32),
    Between(u32, u32),
}

impl Year {
    fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        for (prefix, make) in [
            (">=", Self::Gte as fn(u32) -> Self),
            ("<=", Self::Lte),
            (">", Self::Gt),
            ("<", Self::Lt),
        ] {
            if let Some(rest) = value.strip_prefix(prefix) {
                return rest.trim().parse().ok().map(make);
            }
        }
        if let Some((start, end)) = value.split_once('-')
            && let (Ok(start), Ok(end)) = (start.trim().parse::<u32>(), end.trim().parse::<u32>())
        {
            return Some(Self::Between(start.min(end), start.max(end)));
        }
        value.parse().ok().map(Self::Eq)
    }

    fn matches(self, year: u32) -> bool {
        match self {
            Self::Eq(value) => year == value,
            Self::Gte(value) => year >= value,
            Self::Lte(value) => year <= value,
            Self::Gt(value) => year > value,
            Self::Lt(value) => year < value,
            Self::Between(start, end) => (start..=end).contains(&year),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterValue {
    /// Lowercased substring, matched case-insensitively.
    Text(String),
    Year(Year),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub field: Field,
    pub negated: bool,
    pub value: FilterValue,
}

impl Filter {
    pub fn matches(&self, values: &Values) -> bool {
        let matched = match &self.value {
            FilterValue::Text(needle) => values.text(self.field).contains(needle),
            FilterValue::Year(year) => values.year.is_some_and(|value| year.matches(value)),
        };
        matched != self.negated
    }
}

/// Lowercased, per-field text used by [`Filter::matches`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Values {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub format: String,
    pub uri: String,
    pub year: Option<u32>,
}

impl Values {
    fn text(&self, field: Field) -> &str {
        match field {
            Field::Title => &self.title,
            Field::Artist => &self.artist,
            Field::Album => &self.album,
            Field::AlbumArtist => &self.album_artist,
            Field::Genre => &self.genre,
            Field::Year => "", // Years are numeric; see `Filter::matches`.
            Field::Format => &self.format,
            Field::Uri => &self.uri,
        }
    }
}

/// Parsed search input: fuzzy terms and structured field filters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub terms: Vec<String>,
    pub filters: Vec<Filter>,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let mut query = Self::default();
        for token in tokenize(input) {
            let (negated, token) = match token.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest),
                _ => (false, token.as_str()),
            };

            if let Some((name, value)) = token.split_once(':')
                && !value.is_empty()
                && let Some(field) = Field::parse(&name.to_ascii_lowercase())
            {
                let value = if field == Field::Year {
                    Year::parse(value).map(FilterValue::Year)
                } else {
                    Some(FilterValue::Text(value.to_lowercase()))
                };
                if let Some(value) = value {
                    query.filters.push(Filter {
                        field,
                        negated,
                        value,
                    });
                    continue;
                }
            }

            if negated {
                query.terms.push(format!("-{token}"));
            } else {
                query.terms.push(token.to_string());
            }
        }
        query
    }

    pub fn matches(&self, values: &Values) -> bool {
        self.filters.iter().all(|filter| filter.matches(values))
    }

    /// The fuzzy part of the query, reconstructed for `nucleo-matcher`.
    pub fn fuzzy_terms(&self) -> String {
        self.terms.join(" ")
    }
}

/// Splits on whitespace while keeping quoted values together.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = input.chars();

    while let Some(ch) = chars.next() {
        match quote {
            Some(end) if ch == end => quote = None,
            Some(_) if ch == '\\' => {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(artist: &str, album: &str, title: &str) -> Values {
        Values {
            title: title.to_lowercase(),
            artist: artist.to_lowercase(),
            album: album.to_lowercase(),
            ..Values::default()
        }
    }

    #[test]
    fn parses_terms_and_filters() {
        let query = Query::parse(r#"artist:portishead album:"third" -genre:rock sour"#);
        assert_eq!(query.terms, vec!["sour"]);
        assert_eq!(query.filters.len(), 3);
        assert_eq!(query.filters[0].field, Field::Artist);
        assert_eq!(
            query.filters[0].value,
            FilterValue::Text("portishead".into())
        );
        assert_eq!(query.filters[1].value, FilterValue::Text("third".into()));
        assert!(query.filters[2].negated);
        assert_eq!(query.fuzzy_terms(), "sour");
    }

    #[test]
    fn quoted_values_keep_spaces() {
        let query = Query::parse(r#"artist:'massive attack' angel"#);
        assert_eq!(query.terms, vec!["angel"]);
        assert_eq!(
            query.filters[0].value,
            FilterValue::Text("massive attack".into())
        );
    }

    #[test]
    fn unknown_fields_remain_fuzzy_terms() {
        let query = Query::parse("http://radio.example/stream");
        assert!(query.filters.is_empty());
        assert_eq!(query.terms, vec!["http://radio.example/stream"]);
    }

    #[test]
    fn year_operators_and_ranges() {
        assert_eq!(Year::parse("1998"), Some(Year::Eq(1998)));
        assert_eq!(Year::parse(">=1990"), Some(Year::Gte(1990)));
        assert_eq!(Year::parse("<2000"), Some(Year::Lt(2000)));
        assert_eq!(Year::parse("1990-1999"), Some(Year::Between(1990, 1999)));
        assert_eq!(Year::parse("1999-1990"), Some(Year::Between(1990, 1999)));
        assert_eq!(Year::parse("nineties"), None);
    }

    #[test]
    fn filters_match_and_negate() {
        let values = values("Portishead", "Dummy", "Sour Times");
        assert!(
            Filter {
                field: Field::Artist,
                negated: false,
                value: FilterValue::Text("portis".into()),
            }
            .matches(&values)
        );
        assert!(
            !Filter {
                field: Field::Album,
                negated: true,
                value: FilterValue::Text("dummy".into()),
            }
            .matches(&values)
        );
        assert!(
            Filter {
                field: Field::Title,
                negated: false,
                value: FilterValue::Text("sour".into()),
            }
            .matches(&values)
        );
    }

    #[test]
    fn a_bad_year_filter_falls_back_to_a_term() {
        let query = Query::parse("year:nineties");
        assert!(query.filters.is_empty());
        assert_eq!(query.terms, vec!["year:nineties"]);
    }
}
