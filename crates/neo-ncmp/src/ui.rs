//! Rendering only: every function takes application state and paints a frame.
//!
//! Large lists are rendered viewport-first. `Table`/`List` would walk every
//! item, which is fine for a 6,000-song library but not for the 100,000-song
//! libraries this client is meant to handle.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use neo_ncmp_domain::{EntryKind, PlaybackState, Song, format_duration};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Gauge, Paragraph, Wrap};
use ratatui_image::{Resize, StatefulImage};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    App, Confirm, ConnectionState, HitArea, InputModal, LibraryPane, PlaylistPane, Screen, Toast,
    ToastLevel,
};
use crate::search::SearchKind;
use crate::widgets::{ListView, fit, fit_right};

#[derive(Debug, Clone, Copy)]
struct Theme {
    bg: Color,
    fg: Color,
    dim: Color,
    accent: Color,
    green: Color,
    red: Color,
    yellow: Color,
    purple: Color,
    cyan: Color,
    border: Color,
    selected_bg: Color,
}

impl Theme {
    const DEFAULT: Self = Self {
        bg: Color::Rgb(26, 27, 38),
        fg: Color::Rgb(192, 202, 245),
        dim: Color::Rgb(86, 95, 137),
        accent: Color::Rgb(122, 162, 247),
        green: Color::Rgb(158, 206, 106),
        red: Color::Rgb(247, 118, 142),
        yellow: Color::Rgb(224, 175, 104),
        purple: Color::Rgb(187, 154, 247),
        cyan: Color::Rgb(125, 207, 255),
        border: Color::Rgb(59, 66, 103),
        selected_bg: Color::Rgb(40, 52, 87),
    };

    fn from_config(config: &crate::config::ThemeConfig) -> Self {
        let default = Self::DEFAULT;
        Self {
            bg: parse_color(&config.bg, default.bg),
            fg: parse_color(&config.fg, default.fg),
            dim: parse_color(&config.dim, default.dim),
            accent: parse_color(&config.accent, default.accent),
            green: parse_color(&config.green, default.green),
            red: parse_color(&config.red, default.red),
            yellow: parse_color(&config.yellow, default.yellow),
            purple: parse_color(&config.purple, default.purple),
            cyan: parse_color(&config.cyan, default.cyan),
            border: parse_color(&config.border, default.border),
            selected_bg: parse_color(&config.selected_bg, default.selected_bg),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::DEFAULT
    }
}

static THEME: std::sync::RwLock<Theme> = std::sync::RwLock::new(Theme::DEFAULT);

fn theme() -> Theme {
    *THEME
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Applies `[theme]`; safe to call again for live reload.
pub fn apply_theme(config: &crate::config::ThemeConfig) {
    *THEME
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Theme::from_config(config);
}

fn parse_color(value: &str, fallback: Color) -> Color {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#')
        && hex.len() == 6
        && let (Ok(red), Ok(green), Ok(blue)) = (
            u8::from_str_radix(&hex[0..2], 16),
            u8::from_str_radix(&hex[2..4], 16),
            u8::from_str_radix(&hex[4..6], 16),
        )
    {
        return Color::Rgb(red, green, blue);
    }
    match value.to_ascii_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "white" => Color::White,
        "reset" => Color::Reset,
        _ => fallback,
    }
}

fn base_style() -> Style {
    Style::default().fg(theme().fg).bg(theme().bg)
}

fn dim_style() -> Style {
    Style::default().fg(theme().dim)
}

pub fn draw(app: &mut App, frame: &mut Frame) {
    let area = frame.area();
    app.mouse = crate::app::MouseLayout::default();
    frame.render_widget(Block::default().style(base_style()), area);

    if area.height >= 12 {
        let [header, main, progress, now, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        draw_header(app, frame, header);
        draw_main(app, frame, main);
        draw_progress(app, frame, progress);
        draw_now_playing(app, frame, now);
        draw_footer(app, frame, footer);
    } else {
        let [header, main, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(area);
        draw_header(app, frame, header);
        draw_main(app, frame, main);
        draw_footer(app, frame, footer);
    }

    if let Some(toast) = &app.toast {
        draw_toast(frame, area, toast);
    }
    if app.search.active {
        draw_search(app, frame, area);
    }
    if app.palette.active {
        draw_palette(app, frame, area);
    }
    if let Some(input) = &app.input {
        draw_input(frame, area, input);
    }
    if let Some(confirm) = &app.confirm {
        draw_confirm(frame, area, confirm);
    }
    if app.help {
        draw_help(frame, area);
    }
}

fn draw_header(app: &mut App, frame: &mut Frame, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let right_width = if area.width >= 64 { 28 } else { 0 };
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(right_width)]).areas(area);

    // Counts are useful but push the last tab off narrower terminals.
    let show_counts = area.width >= 170;
    let mut spans = vec![Span::styled(
        " neo-ncmp ",
        Style::default()
            .fg(theme().bg)
            .bg(theme().accent)
            .add_modifier(Modifier::BOLD),
    )];
    // Tabs start after the " neo-ncmp " badge.
    let mut tab_x = left.x + UnicodeWidthStr::width(" neo-ncmp ") as u16;
    for screen in Screen::ALL.iter() {
        let number = screen.key_number();
        let label = match screen {
            Screen::Queue if show_counts => {
                format!(" {number} Queue ({}) ", app.queue.len())
            }
            Screen::Library if show_counts => format!(
                " {number} Library ({}) ",
                app.library.as_ref().map_or(0, |library| library.len())
            ),
            Screen::Playlists if show_counts => {
                format!(" {number} Playlists ({}) ", app.playlists.len())
            }
            _ => format!(" {number} {} ", screen.title()),
        };
        let style = if *screen == app.screen {
            Style::default()
                .fg(theme().bg)
                .bg(theme().accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme().dim)
        };
        let width = UnicodeWidthStr::width(label.as_str()) as u16;
        let visible = width.min(left.right().saturating_sub(tab_x));
        if visible > 0 {
            app.mouse.tabs.push((
                Rect {
                    x: tab_x,
                    y: area.y,
                    width: visible,
                    height: 1,
                },
                *screen,
            ));
        }
        tab_x = tab_x.saturating_add(width);
        spans.push(Span::styled(label, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), left);

    if right_width > 0 {
        let (label, style) = match &app.connection {
            ConnectionState::Connecting => (
                "○ connecting".to_string(),
                Style::default().fg(theme().yellow),
            ),
            ConnectionState::Connected(version) => (
                format!("● MPD {version}"),
                Style::default().fg(theme().green),
            ),
            ConnectionState::Disconnected => (
                "● disconnected".to_string(),
                Style::default().fg(theme().red),
            ),
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(label, style))).alignment(Alignment::Right),
            right,
        );
    }
}

fn draw_main(app: &mut App, frame: &mut Frame, area: Rect) {
    match app.screen {
        Screen::Queue => draw_queue(app, frame, area),
        Screen::Library => draw_library(app, frame, area),
        Screen::Playlists => draw_playlists(app, frame, area),
        Screen::Outputs => draw_outputs(app, frame, area),
        Screen::Stats => draw_stats(app, frame, area),
        Screen::Lyrics => draw_lyrics(app, frame, area),
        Screen::Visualizer => draw_visualizer(app, frame, area),
        Screen::TagEditor => draw_tags(app, frame, area),
        Screen::History => draw_history(app, frame, area),
        Screen::Browser => draw_browser(app, frame, area),
    }
}

fn pane_block(title: &str, active: bool) -> Block<'static> {
    let border = if active {
        theme().accent
    } else {
        theme().border
    };
    let title_style = if active {
        Style::default()
            .fg(theme().accent)
            .add_modifier(Modifier::BOLD)
    } else {
        dim_style()
    };
    Block::bordered()
        .border_style(Style::default().fg(border))
        .title(Span::styled(format!(" {title} "), title_style))
}

fn draw_queue(app: &mut App, frame: &mut Frame, area: Rect) {
    let layout = &app.config.layout;
    let with_cover = layout.cover_sidebar && area.width >= layout.cover_width.saturating_add(40);
    let (queue_area, cover_area) = if with_cover {
        let [queue_area, cover_area] =
            Layout::horizontal([Constraint::Min(40), Constraint::Length(layout.cover_width)])
                .areas(area);
        (queue_area, Some(cover_area))
    } else {
        (area, None)
    };

    let block = pane_block(
        &format!(
            "Queue — {} song{}",
            app.queue.len(),
            if app.queue.len() == 1 { "" } else { "s" }
        ),
        true,
    );
    let inner = block.inner(queue_area);
    frame.render_widget(block, queue_area);
    if let Some(cover_area) = cover_area {
        draw_cover(app, frame, cover_area);
    }

    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if app.queue.is_empty() {
        frame.render_widget(
            Paragraph::new("Queue is empty — press 2 to browse the library")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let header_area = Rect { height: 1, ..inner };
    let list_area = Rect {
        y: inner.y + 1,
        height: inner.height - 1,
        ..inner
    };
    render_song_header(frame, header_area);

    let viewport = list_area.height as usize;
    app.queue_view.ensure_visible(app.queue.len(), viewport);
    let cursor = app.queue_view.selected(app.queue.len()).unwrap_or(0);
    let start = app.queue_view.offset;
    let end = (start + viewport).min(app.queue.len());
    app.mouse.queue = Some(HitArea {
        rect: list_area,
        offset: start,
    });

    for (row, index) in (start..end).enumerate() {
        let song = &app.queue[index];
        let current = app.is_current(song);
        let selected = index == cursor;
        let style = row_style(selected, current);
        let line = song_line(song, inner.width as usize, current, false);
        frame.render_widget(
            Paragraph::new(line).style(style),
            Rect {
                x: list_area.x,
                y: list_area.y + row as u16,
                width: list_area.width,
                height: 1,
            },
        );
    }
}

fn draw_cover(app: &mut App, frame: &mut Frame, area: Rect) {
    let block = pane_block("Now Playing", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 4 || inner.width < 4 {
        return;
    }

    let info_height = 6.min(inner.height.saturating_sub(1));
    let art_height = inner.height.saturating_sub(info_height);
    if art_height > 0 {
        let art_rect = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: art_height,
        };
        if let Some(protocol) = app.cover_protocol.as_mut() {
            frame.render_stateful_widget(
                StatefulImage::new().resize(Resize::Fit(None)),
                art_rect,
                protocol,
            );
        } else {
            frame.render_widget(
                Paragraph::new("♪\n\nno cover art")
                    .alignment(Alignment::Center)
                    .style(dim_style()),
                art_rect,
            );
        }
    }

    let info_rect = Rect {
        x: inner.x,
        y: inner.y + art_height,
        width: inner.width,
        height: inner.height.saturating_sub(art_height),
    };
    let mut lines = Vec::new();
    if let Some(song) = &app.current {
        lines.push(Line::from(Span::styled(
            fit(song.display_title(), inner.width as usize),
            Style::default()
                .fg(theme().green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            fit(song.display_artist(), inner.width as usize),
            Style::default().fg(theme().fg),
        )));
        lines.push(Line::from(Span::styled(
            fit(song.display_album(), inner.width as usize),
            dim_style(),
        )));
    } else {
        lines.push(Line::from(Span::styled("Nothing playing", dim_style())));
    }
    if let Some(status) = &app.status
        && let Some(audio) = status.audio
    {
        let detail = match status.bitrate {
            Some(bitrate) => format!("{audio} · {bitrate} kbps"),
            None => audio.to_string(),
        };
        lines.push(Line::from(Span::styled(
            fit(&detail, inner.width as usize),
            dim_style(),
        )));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(base_style()),
        info_rect,
    );
}

fn draw_library(app: &mut App, frame: &mut Frame, area: Rect) {
    if app.library.is_none() {
        frame.render_widget(
            Paragraph::new("Indexing library…")
                .style(dim_style())
                .alignment(Alignment::Center)
                .block(pane_block("Library", true)),
            area,
        );
        return;
    }

    let layout = &app.config.layout;
    let [artists_area, albums_area, songs_area] = Layout::horizontal([
        Constraint::Percentage(layout.artists_width),
        Constraint::Percentage(layout.albums_width),
        Constraint::Percentage(layout.songs_width),
    ])
    .areas(area);

    let artist_cursor = app.library_view.artists.cursor;
    let album_cursor = app.library_view.albums.cursor;
    let current_uri = app.current.as_ref().map(|song| song.uri.clone());
    let (artist_count, album_count, song_count) = (
        app.library_artist_count(),
        app.library_album_count(),
        app.library_song_count(),
    );
    let pane = app.library_view.pane;

    let artists_hit = {
        let library = app.library.as_ref().expect("checked above");
        let view = &mut app.library_view.artists;
        render_list_pane(
            frame,
            artists_area,
            "Artists",
            artist_count,
            view,
            pane == LibraryPane::Artists,
            |index| {
                let artist = &library.artists()[index];
                Line::from(vec![
                    Span::raw(artist.name.to_string()),
                    Span::styled(format!("  ({})", artist.song_count()), dim_style()),
                ])
            },
        )
    };
    app.mouse.library_artists = artists_hit;

    let albums_hit = {
        let library = app.library.as_ref().expect("checked above");
        let artist = library.artists().get(artist_cursor);
        let view = &mut app.library_view.albums;
        render_list_pane(
            frame,
            albums_area,
            "Albums",
            album_count,
            view,
            pane == LibraryPane::Albums,
            |index| {
                let album = &artist.expect("non-empty pane").albums[index];
                let mut spans = vec![Span::raw(album.name.to_string())];
                if let Some(year) = &album.year {
                    spans.push(Span::styled(format!(" {year}"), dim_style()));
                }
                spans.push(Span::styled(
                    format!("  ({})", album.songs.len()),
                    dim_style(),
                ));
                Line::from(spans)
            },
        )
    };
    app.mouse.library_albums = albums_hit;

    let songs_hit = {
        let library = app.library.as_ref().expect("checked above");
        let album = library
            .artists()
            .get(artist_cursor)
            .and_then(|artist| artist.albums.get(album_cursor));
        let view = &mut app.library_view.songs;
        let width = songs_area.width.saturating_sub(2) as usize;
        render_list_pane(
            frame,
            songs_area,
            "Songs",
            song_count,
            view,
            pane == LibraryPane::Songs,
            |index| {
                let Some(song) = album
                    .and_then(|album| album.songs.get(index))
                    .and_then(|&song_index| library.song(song_index))
                else {
                    return Line::default();
                };
                let current = current_uri.as_ref().is_some_and(|uri| song.uri == *uri);
                song_line(song, width, current, true)
            },
        )
    };
    app.mouse.library_songs = songs_hit;
}

fn draw_playlists(app: &mut App, frame: &mut Frame, area: Rect) {
    let playlists_width = app.config.layout.playlists_width.min(90);
    let [playlists_area, songs_area] = Layout::horizontal([
        Constraint::Percentage(playlists_width),
        Constraint::Percentage(100u16.saturating_sub(playlists_width)),
    ])
    .areas(area);
    let (playlist_count, song_count) = (app.playlists.len(), app.playlist_songs.len());
    let pane = app.playlist_pane;
    let selected_name = app.selected_playlist_name();
    let current_uri = app.current.as_ref().map(|song| song.uri.clone());

    let playlists_hit = {
        let playlists = &app.playlists;
        let view = &mut app.playlists_view;
        render_list_pane(
            frame,
            playlists_area,
            "Playlists",
            playlist_count,
            view,
            pane == PlaylistPane::Playlists,
            |index| {
                let playlist = &playlists[index];
                let mut spans = vec![Span::raw(playlist.name.to_string())];
                if let Some(modified) = &playlist.last_modified {
                    spans.push(Span::styled(
                        format!("  {}", short_date(modified)),
                        dim_style(),
                    ));
                }
                Line::from(spans)
            },
        )
    };
    app.mouse.playlists = playlists_hit;

    let songs_hit = {
        let songs = &app.playlist_songs;
        let view = &mut app.playlist_songs_view;
        let title = selected_name
            .as_deref()
            .map(|name| format!("Songs — {name}"))
            .unwrap_or_else(|| "Songs".into());
        let width = songs_area.width.saturating_sub(2) as usize;
        render_list_pane(
            frame,
            songs_area,
            &title,
            song_count,
            view,
            pane == PlaylistPane::Songs,
            |index| {
                let song = &songs[index];
                let current = current_uri.as_ref().is_some_and(|uri| song.uri == *uri);
                song_line(song, width, current, false)
            },
        )
    };
    app.mouse.playlist_songs = songs_hit;
}

fn draw_lyrics(app: &App, frame: &mut Frame, area: Rect) {
    let source = if app.lyrics.loading {
        "searching…".to_string()
    } else {
        app.lyrics
            .doc
            .as_ref()
            .and_then(|doc| doc.source)
            .map(|source| source.label().to_string())
            .unwrap_or_default()
    };
    let offset = app.lyrics.offset_ms;
    let offset_text = if offset == 0 {
        String::new()
    } else {
        format!(" · offset {:+0.1}s", offset as f64 / 1000.0)
    };
    let title = if source.is_empty() {
        format!("Lyrics{offset_text}")
    } else {
        format!("Lyrics — {source}{offset_text}")
    };
    let block = pane_block(&title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if app.lyrics.loading {
        frame.render_widget(
            Paragraph::new("Looking for lyrics…")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }
    let Some(doc) = &app.lyrics.doc else {
        frame.render_widget(
            Paragraph::new("No lyrics found for this track\n\nPress o to retry")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    };
    if doc.is_empty() {
        frame.render_widget(
            Paragraph::new("Lyrics are empty\n\nPress o to retry")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    if !doc.synced.is_empty() {
        let position = app.lyrics.adjusted_position(app.effective_elapsed());
        let active = doc.active_index(position);
        let start = active
            .unwrap_or(0)
            .saturating_sub(inner.height as usize / 2);
        let lines: Vec<Line<'static>> = doc
            .synced
            .iter()
            .enumerate()
            .map(|(index, line)| {
                let style = if Some(index) == active {
                    Style::default()
                        .fg(theme().accent)
                        .add_modifier(Modifier::BOLD)
                } else if Some(index) < active {
                    dim_style()
                } else {
                    Style::default().fg(theme().fg)
                };
                Line::from(Span::styled(line.text.clone(), style))
            })
            .collect();
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false })
                .scroll((start as u16, 0))
                .style(base_style()),
            inner,
        );
    } else if let Some(plain) = &doc.plain {
        frame.render_widget(
            Paragraph::new(plain.as_str())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false })
                .scroll((app.lyrics.scroll as u16, 0))
                .style(base_style()),
            inner,
        );
    }
}

fn draw_visualizer(app: &mut App, frame: &mut Frame, area: Rect) {
    let settings = &app.config.visualizer;
    let block = pane_block(
        &format!(
            "Visualizer — {} Hz · {} bars",
            settings.sample_rate, settings.bars
        ),
        true,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if !settings.enabled {
        frame.render_widget(
            Paragraph::new("Visualizer disabled\n\nSet [visualizer] enabled = true in config.toml")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }
    if let Some(error) = &app.visualizer_error {
        let hint = if settings.source.starts_with("pipewire") || settings.source == "pw" {
            "PipeWire capture needs pw-record and a usable default sink; set [visualizer] target to override it"
        } else {
            "Add an MPD fifo audio output pointed at this path"
        };
        frame.render_widget(
            Paragraph::new(format!("{error}\n\n{hint}"))
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }
    if app.spectrum.is_empty() {
        frame.render_widget(
            Paragraph::new("Waiting for audio…")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let bars = &app.spectrum;
    let width = inner.width as usize;
    let height = inner.height as usize;
    let columns = bars.len().min(width).max(1);
    let buffer = frame.buffer_mut();
    for x in 0..width {
        let index = x * columns / width;
        let value = bars.get(index).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let bar_height = (value * height as f32).round() as usize;
        for y in 0..bar_height {
            if let Some(cell) =
                buffer.cell_mut((inner.x + x as u16, inner.y + inner.height - 1 - y as u16))
            {
                cell.set_char('█')
                    .set_fg(bar_color(y as f32 / height as f32));
            }
        }
    }
}

fn bar_color(fraction: f32) -> Color {
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction < 0.5 {
        lerp_color(theme().green, theme().yellow, fraction * 2.0)
    } else {
        lerp_color(theme().yellow, theme().red, (fraction - 0.5) * 2.0)
    }
}

fn lerp_color(from: Color, to: Color, t: f32) -> Color {
    let (from_r, from_g, from_b) = rgb(from);
    let (to_r, to_g, to_b) = rgb(to);
    Color::Rgb(
        lerp_u8(from_r, to_r, t),
        lerp_u8(from_g, to_g, t),
        lerp_u8(from_b, to_b, t),
    )
}

fn rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(red, green, blue) => (red, green, blue),
        _ => (255, 255, 255),
    }
}

fn lerp_u8(from: u8, to: u8, t: f32) -> u8 {
    (from as f32 + (to as f32 - from as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn draw_tags(app: &mut App, frame: &mut Frame, area: Rect) {
    let title = if app.tag_editor.dirty {
        "Tags — unsaved changes"
    } else {
        "Tags"
    };
    let block = pane_block(title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let Some(song) = &app.current else {
        frame.render_widget(
            Paragraph::new("Nothing playing")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    };

    let [path_area, list_area, message_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .areas(inner);

    let editable = app.config.library.music_directory.is_some() && !song.uri.contains("://");
    let path_text = match &app.config.library.music_directory {
        Some(directory) => format!("File: {}", directory.join(song.uri.as_ref()).display()),
        None => "Set [library] music_directory to enable tag editing".to_string(),
    };
    frame.render_widget(
        Paragraph::new(fit(&path_text, path_area.width as usize)).style(if editable {
            dim_style()
        } else {
            Style::default().fg(theme().red)
        }),
        path_area,
    );

    let len = app.tag_editor.fields.len();
    if len > 0 && list_area.height > 0 {
        app.tag_editor.cursor = app.tag_editor.cursor.min(len - 1);
        let height = list_area.height as usize;
        let start = app
            .tag_editor
            .cursor
            .saturating_sub(height.saturating_sub(1))
            .min(len.saturating_sub(height));
        let end = (start + height).min(len);
        app.mouse.tags = Some(HitArea {
            rect: list_area,
            offset: start,
        });
        for (row, index) in (start..end).enumerate() {
            let field = &app.tag_editor.fields[index];
            let selected = index == app.tag_editor.cursor;
            let line = Line::from(vec![
                Span::styled(format!("{:<14}", field.key.label()), dim_style()),
                Span::raw(field.value.clone()),
            ]);
            let style = if selected {
                Style::default()
                    .bg(theme().selected_bg)
                    .fg(theme().fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme().fg)
            };
            frame.render_widget(
                Paragraph::new(line).style(style),
                Rect {
                    x: list_area.x,
                    y: list_area.y + row as u16,
                    width: list_area.width,
                    height: 1,
                },
            );
        }
    }

    let (message, style) = match &app.tag_editor.message {
        Some((message, true)) => (message.as_str(), Style::default().fg(theme().red)),
        Some((message, false)) => (message.as_str(), Style::default().fg(theme().green)),
        None => ("Enter edit · w write · j/k move", dim_style()),
    };
    frame.render_widget(
        Paragraph::new(fit(message, message_area.width as usize)).style(style),
        message_area,
    );
}

fn draw_browser(app: &mut App, frame: &mut Frame, area: Rect) {
    let location = app
        .browser
        .uri
        .as_deref()
        .map(|uri| format!("/{uri}"))
        .unwrap_or_else(|| "/".into());
    let block = pane_block(&format!("Browser — {location}"), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if app.browser.loading {
        frame.render_widget(
            Paragraph::new("Loading directory…")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }
    if app.browser.entries.is_empty() {
        frame.render_widget(
            Paragraph::new("Empty directory — Backspace to go up")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let height = inner.height as usize;
    app.browser
        .view
        .ensure_visible(app.browser.entries.len(), height);
    let cursor = app.browser.view.cursor;
    let start = app.browser.view.offset;
    let end = (start + height).min(app.browser.entries.len());
    app.mouse.browser = Some(HitArea {
        rect: inner,
        offset: start,
    });
    let width = inner.width as usize;
    let name_width = (width * 2 / 5).max(10);

    for (row, index) in (start..end).enumerate() {
        let entry = &app.browser.entries[index];
        let (marker, color) = match entry.kind {
            EntryKind::Directory => ("[D]", theme().accent),
            EntryKind::Playlist => ("[P]", theme().purple),
            EntryKind::File => ("[F]", theme().green),
        };
        let detail = match entry.kind {
            EntryKind::Directory => "directory".to_string(),
            EntryKind::Playlist => "playlist".to_string(),
            EntryKind::File => entry
                .song
                .as_ref()
                .map(|song| format!("{} — {}", song.display_artist(), song.display_album()))
                .unwrap_or_default(),
        };
        let line = Line::from(vec![
            Span::styled(
                marker,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::raw(fit(&entry.name, name_width)),
            Span::raw(" "),
            Span::styled(
                fit(&detail, width.saturating_sub(3 + 1 + name_width + 1)),
                dim_style(),
            ),
        ]);
        let selected = index == cursor;
        let style = if selected {
            Style::default()
                .bg(theme().selected_bg)
                .fg(theme().fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme().fg)
        };
        frame.render_widget(
            Paragraph::new(line).style(style),
            Rect {
                x: inner.x,
                y: inner.y + row as u16,
                width: inner.width,
                height: 1,
            },
        );
    }
}

fn draw_history(app: &mut App, frame: &mut Frame, area: Rect) {
    let block = pane_block("History", true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if app.history_loading {
        frame.render_widget(
            Paragraph::new("Loading play history…")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }
    if let Some(error) = &app.history_error {
        frame.render_widget(
            Paragraph::new(error.as_str())
                .style(Style::default().fg(theme().red))
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }
    let Some(report) = &app.history else {
        frame.render_widget(
            Paragraph::new("No play history yet")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    };
    if report.total_plays == 0 {
        frame.render_widget(
            Paragraph::new("No play history yet\n\nPlays are recorded after half a track")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let [totals_area, columns_area, recent_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(10),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "{} plays · {} listened",
                report.total_plays,
                format_long_duration(Duration::from_secs(report.total_seconds))
            ),
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        ))),
        totals_area,
    );

    let [songs_area, artists_area, albums_area] = Layout::horizontal([
        Constraint::Percentage(45),
        Constraint::Percentage(25),
        Constraint::Percentage(30),
    ])
    .areas(columns_area);
    render_history_songs(frame, songs_area, report, app.history_cursor);
    app.mouse.history = Some(HitArea {
        rect: pane_block("Top Songs", true).inner(songs_area),
        offset: 0,
    });
    render_history_column(
        frame,
        artists_area,
        "Top Artists",
        report
            .top_artists
            .iter()
            .map(|artist| (artist.plays, artist.name.clone())),
    );
    render_history_column(
        frame,
        albums_area,
        "Top Albums",
        report
            .top_albums
            .iter()
            .map(|album| (album.plays, album.name.clone())),
    );

    let recent_block = pane_block("Recent", false);
    let recent_inner = recent_block.inner(recent_area);
    frame.render_widget(recent_block, recent_area);
    let lines: Vec<Line<'static>> = report
        .recent
        .iter()
        .take(recent_inner.height as usize)
        .map(|play| {
            Line::from(vec![
                Span::raw(format!("{} — {}", play.title, play.artist)),
                Span::styled(format!("  ({})", age_text(play.played_at)), dim_style()),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(base_style()),
        recent_inner,
    );
}

fn render_history_songs(
    frame: &mut Frame,
    area: Rect,
    report: &crate::stats::StatsReport,
    cursor: usize,
) {
    let block = pane_block("Top Songs", true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if report.top_songs.is_empty() {
        frame.render_widget(
            Paragraph::new("—")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }
    let width = inner.width as usize;
    for (row, song) in report
        .top_songs
        .iter()
        .take(inner.height as usize)
        .enumerate()
    {
        let selected = row == cursor;
        let line = Line::from(vec![
            Span::styled(
                format!("{:>4}  ", song.plays),
                Style::default().fg(theme().accent),
            ),
            Span::raw(fit(
                &format!("{} — {}", song.title, song.artist),
                width.saturating_sub(6),
            )),
        ]);
        let style = if selected {
            Style::default()
                .bg(theme().selected_bg)
                .fg(theme().fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme().fg)
        };
        frame.render_widget(
            Paragraph::new(line).style(style),
            Rect {
                x: inner.x,
                y: inner.y + row as u16,
                width: inner.width,
                height: 1,
            },
        );
    }
}

fn render_history_column(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    items: impl Iterator<Item = (u64, String)>,
) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let lines: Vec<Line<'static>> = items
        .map(|(plays, text)| {
            Line::from(vec![
                Span::styled(format!("{plays:>4}  "), Style::default().fg(theme().accent)),
                Span::raw(fit(&text, inner_width.saturating_sub(6))),
            ])
        })
        .collect();
    let paragraph = if lines.is_empty() {
        Paragraph::new("—")
            .style(dim_style())
            .alignment(Alignment::Center)
    } else {
        Paragraph::new(Text::from(lines)).style(base_style())
    };
    frame.render_widget(paragraph.block(pane_block(title, false)), area);
}

fn draw_outputs(app: &mut App, frame: &mut Frame, area: Rect) {
    let block = pane_block("Outputs", true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if app.outputs.is_empty() {
        frame.render_widget(
            Paragraph::new("No audio outputs reported by MPD")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    app.outputs_view
        .ensure_visible(app.outputs.len(), inner.height as usize);
    let cursor = app.outputs_view.cursor;
    let start = app.outputs_view.offset;
    let end = (start + inner.height as usize).min(app.outputs.len());
    app.mouse.outputs = Some(HitArea {
        rect: inner,
        offset: start,
    });

    for (row, index) in (start..end).enumerate() {
        let output = &app.outputs[index];
        let selected = index == cursor;
        let (icon, style) = if output.enabled {
            ("[●]", Style::default().fg(theme().green))
        } else {
            ("[ ]", dim_style())
        };
        let line = Line::from(vec![
            Span::styled(icon, style),
            Span::raw(" "),
            Span::raw(output.name.to_string()),
            Span::styled(format!("   id {}", output.id), dim_style()),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(row_style(selected, false)),
            Rect {
                x: inner.x,
                y: inner.y + row as u16,
                width: inner.width,
                height: 1,
            },
        );
    }
}

fn draw_stats(app: &App, frame: &mut Frame, area: Rect) {
    let block = pane_block("Server & current track", true);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(version) = &app.version {
        lines.push(kv("MPD", version.clone()));
    }
    if let Some(status) = &app.status {
        lines.push(kv(
            "State",
            format!(
                "{:?} — volume {}",
                status.state,
                status
                    .volume
                    .map(|volume| format!("{volume}%"))
                    .unwrap_or_else(|| "n/a".into())
            ),
        ));
        if let Some(audio) = status.audio {
            lines.push(kv(
                "Audio",
                format!(
                    "{} @ {} kbps",
                    audio,
                    status
                        .bitrate
                        .map(|bitrate| bitrate.to_string())
                        .unwrap_or_else(|| "?".into())
                ),
            ));
        }
        lines.push(kv("Queue", status.playlist_length.to_string()));
        if let Some(partition) = &status.partition {
            lines.push(kv("Partition", partition.to_string()));
        }
        if let Some(error) = &status.error {
            lines.push(Line::from(Span::styled(
                format!("MPD error: {error}"),
                Style::default().fg(theme().red),
            )));
        }
    }
    lines.push(Line::default());

    if let Some(song) = &app.current {
        lines.push(Line::from(Span::styled(
            format!("{} — {}", song.display_artist(), song.display_title()),
            Style::default()
                .fg(theme().green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(kv("Album", song.display_album().to_string()));
        if let Some(date) = &song.date {
            lines.push(kv("Date", date.to_string()));
        }
        if let Some(genre) = &song.genre {
            lines.push(kv("Genre", genre.to_string()));
        }
        if let Some(duration) = song.duration {
            lines.push(kv("Duration", format_duration(duration)));
        }
        if let Some(format) = &song.format {
            lines.push(kv("Format", format.to_string()));
        }
        lines.push(kv("URI", song.uri.to_string()));
        lines.push(Line::default());
    }

    if let Some(stats) = &app.stats {
        lines.push(Line::from(Span::styled(
            "Library",
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(kv("Artists", stats.artists.to_string()));
        lines.push(kv("Albums", stats.albums.to_string()));
        lines.push(kv("Songs", stats.songs.to_string()));
        lines.push(kv("Uptime", format_long_duration(stats.uptime)));
        lines.push(kv("Playtime", format_long_duration(stats.playtime)));
        lines.push(kv("DB playtime", format_long_duration(stats.db_playtime)));
        if let Some(updated) = stats.db_update {
            lines.push(kv("DB updated", age_text(updated)));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Loading server statistics…",
            dim_style(),
        )));
    }

    if app.config.scrobbling.enabled {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "Scrobbling",
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(kv("Pending", app.scrobble_pending.len().to_string()));
        for queued in app.scrobble_pending.iter().take(3) {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}  ", queued.service), dim_style()),
                Span::raw(format!(
                    "{} — {}",
                    queued.event.track.artist, queued.event.track.title
                )),
            ]));
        }
        if !app.scrobble_pending.is_empty() {
            lines.push(Line::from(Span::styled(
                "  Retry from the command palette (:)",
                dim_style(),
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(base_style())
            .wrap(Wrap { trim: true }),
        inner,
    );
}

fn draw_progress(app: &App, frame: &mut Frame, area: Rect) {
    let elapsed = app.effective_elapsed();
    // `status` omits duration when the player is stopped; fall back to the
    // current song's tag so the bar still shows a sensible total.
    let duration = app
        .status
        .as_ref()
        .and_then(|status| status.duration)
        .or_else(|| app.current.as_ref().and_then(|song| song.duration));
    let (ratio, label, fill) = match &app.status {
        Some(status) => {
            let ratio = duration
                .map(|duration| {
                    if duration.is_zero() {
                        0.0
                    } else {
                        (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0)
                    }
                })
                .unwrap_or(0.0);
            let label = format!(
                "{} / {}",
                format_duration(elapsed),
                duration
                    .map(format_duration)
                    .unwrap_or_else(|| "live".into())
            );
            let fill = match status.state {
                PlaybackState::Playing => theme().green,
                PlaybackState::Paused => theme().yellow,
                PlaybackState::Stopped => theme().dim,
            };
            (ratio, label, fill)
        }
        None => (0.0, "no track".to_string(), theme().dim),
    };

    frame.render_widget(
        Gauge::default()
            .ratio(ratio)
            .label(Span::styled(label, Style::default().fg(theme().fg)))
            .gauge_style(Style::default().fg(fill).bg(theme().selected_bg))
            .style(Style::default().bg(theme().bg)),
        area,
    );
}

fn draw_now_playing(app: &App, frame: &mut Frame, area: Rect) {
    let info_width = if area.width >= 80 { 40 } else { 0 };
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(info_width)]).areas(area);

    let (text, style) = match (&app.status, &app.current) {
        (Some(status), Some(song)) => {
            let icon = match status.state {
                PlaybackState::Playing => "▶",
                PlaybackState::Paused => "❚❚",
                PlaybackState::Stopped => "■",
            };
            let style = match status.state {
                PlaybackState::Playing => Style::default().fg(theme().green),
                PlaybackState::Paused => Style::default().fg(theme().yellow),
                PlaybackState::Stopped => dim_style(),
            };
            (
                format!(
                    "{icon} {} — {}",
                    song.display_artist(),
                    song.display_title()
                ),
                style,
            )
        }
        _ => ("· Nothing playing".to_string(), dim_style()),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            fit(&text, left.width as usize),
            style,
        ))),
        left,
    );

    if info_width > 0 {
        let info = app
            .status
            .as_ref()
            .and_then(|status| {
                let audio = status.audio?;
                Some(match status.bitrate {
                    Some(bitrate) => format!("{audio} · {bitrate} kbps"),
                    None => audio.to_string(),
                })
            })
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                fit(&info, right.width as usize),
                dim_style(),
            )))
            .alignment(Alignment::Right),
            right,
        );
    }
}

fn draw_footer(app: &App, frame: &mut Frame, area: Rect) {
    let hints_width = if area.width >= 70 { 30 } else { 0 };
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(hints_width)]).areas(area);

    let mut spans = Vec::new();
    if let Some(status) = &app.status {
        spans.push(Span::styled(
            format!(
                " vol {}% ",
                status
                    .volume
                    .map(|volume| volume.to_string())
                    .unwrap_or_else(|| "n/a".into())
            ),
            Style::default().fg(theme().cyan),
        ));
        for (label, enabled) in [
            ("rep", status.repeat),
            ("rnd", status.random),
            ("sgl", status.single),
            ("cns", status.consume),
        ] {
            let style = if enabled {
                Style::default().fg(theme().green)
            } else {
                dim_style()
            };
            spans.push(Span::styled(
                format!("{label}{} ", if enabled { "●" } else { "○" }),
                style,
            ));
        }
        if let Some(crossfade) = status.crossfade.filter(|value| *value > 0) {
            spans.push(Span::styled(
                format!("xfade {crossfade}s "),
                Style::default().fg(theme().purple),
            ));
        }
    } else {
        spans.push(Span::styled(" connecting… ", dim_style()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), left);

    if hints_width > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "? help  q quit  space play ",
                dim_style(),
            )))
            .alignment(Alignment::Right),
            right,
        );
    }
}

struct SongColumns {
    show_artist: bool,
    show_album: bool,
    title: usize,
}

fn song_columns(width: usize) -> SongColumns {
    let show_artist = width >= 66;
    let show_album = width >= 100;
    let fixed =
        2 + 4 + 1 + 1 + 7 + if show_artist { 21 } else { 0 } + if show_album { 21 } else { 0 };
    SongColumns {
        show_artist,
        show_album,
        title: width.saturating_sub(fixed),
    }
}

fn render_song_header(frame: &mut Frame, area: Rect) {
    let columns = song_columns(area.width as usize);
    let mut spans = vec![
        Span::styled("  ", dim_style()),
        Span::styled(fit_right("#", 4), dim_style()),
        Span::raw(" "),
        Span::styled(fit("Title", columns.title), dim_style()),
    ];
    if columns.show_artist {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(fit("Artist", 20), dim_style()));
    }
    if columns.show_album {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(fit("Album", 20), dim_style()));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(fit_right("Time", 7), dim_style()));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn song_line(song: &Song, width: usize, current: bool, compact: bool) -> Line<'static> {
    let columns = if compact {
        SongColumns {
            show_artist: false,
            show_album: false,
            title: width.saturating_sub(15),
        }
    } else {
        song_columns(width)
    };

    let dim = dim_style();
    let title_style = if current {
        Style::default()
            .fg(theme().green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let marker_style = if current {
        Style::default().fg(theme().green)
    } else {
        dim
    };

    let track = song
        .track
        .or_else(|| song.pos.map(|position| position + 1))
        .map(|track| track.to_string())
        .unwrap_or_default();
    let time = song
        .duration
        .map(format_duration)
        .unwrap_or_else(|| "--:--".into());

    let mut spans = vec![
        Span::styled(if current { "▶ " } else { "  " }, marker_style),
        Span::styled(fit_right(&track, 4), dim),
        Span::raw(" "),
        Span::styled(fit(song.display_title(), columns.title), title_style),
    ];
    if columns.show_artist {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(fit(song.display_artist(), 20), dim));
    }
    if columns.show_album {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(fit(song.display_album(), 20), dim));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(fit_right(&time, 7), dim));
    Line::from(spans)
}

fn row_style(selected: bool, current: bool) -> Style {
    if selected {
        Style::default()
            .bg(theme().selected_bg)
            .fg(theme().fg)
            .add_modifier(Modifier::BOLD)
    } else if current {
        Style::default()
            .fg(theme().green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme().fg)
    }
}

fn render_list_pane<F>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    len: usize,
    view: &mut ListView,
    active: bool,
    render_item: F,
) -> Option<HitArea>
where
    F: Fn(usize) -> Line<'static>,
{
    let block = pane_block(title, active);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return None;
    }
    if len == 0 {
        frame.render_widget(
            Paragraph::new("—")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return None;
    }

    view.ensure_visible(len, inner.height as usize);
    let cursor = view.cursor;
    let start = view.offset;
    let end = (start + inner.height as usize).min(len);

    for (row, index) in (start..end).enumerate() {
        let selected = active && index == cursor;
        let style = if selected {
            Style::default()
                .bg(theme().selected_bg)
                .fg(theme().fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme().fg)
        };
        frame.render_widget(
            Paragraph::new(render_item(index)).style(style),
            Rect {
                x: inner.x,
                y: inner.y + row as u16,
                width: inner.width,
                height: 1,
            },
        );
    }
    Some(HitArea {
        rect: inner,
        offset: start,
    })
}

fn draw_search(app: &mut App, frame: &mut Frame, area: Rect) {
    let Some(index) = app.search_index.as_ref() else {
        return;
    };
    let state = &mut app.search;

    let rect = centered_rect(area, 100, 34);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_style(Style::default().fg(theme().accent))
        .title(Span::styled(
            format!(
                " Search — {} matches{} ",
                state.results.len(),
                if state.pending {
                    " · searching…"
                } else {
                    ""
                }
            ),
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 3 || inner.width == 0 {
        return;
    }

    let [query_area, rule_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);

    let query_width = query_area.width.saturating_sub(3) as usize;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "> ",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(fit(&state.query, query_width)),
            Span::styled("█", Style::default().fg(theme().accent)),
        ])),
        query_area,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(rule_area.width as usize),
            dim_style(),
        ))),
        rule_area,
    );

    if state.results.is_empty() {
        frame.render_widget(
            Paragraph::new("No matches")
                .style(dim_style())
                .alignment(Alignment::Center),
            list_area,
        );
    } else {
        state
            .view
            .ensure_visible(state.results.len(), list_area.height as usize);
        let cursor = state.view.cursor;
        let start = state.view.offset;
        let end = (start + list_area.height as usize).min(state.results.len());
        let width = list_area.width as usize;
        let title_width = (width * 2 / 5).max(8);
        let detail_width = width.saturating_sub(3 + title_width);

        for (row, position) in (start..end).enumerate() {
            let item = &index.items[state.results[position]];
            let color = match item.kind {
                SearchKind::Artist => theme().purple,
                SearchKind::Album => theme().cyan,
                SearchKind::Song => theme().green,
            };
            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", item.kind.marker()),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(fit(&item.title, title_width)),
                Span::raw(" "),
                Span::styled(fit(&item.detail, detail_width), dim_style()),
            ]);
            let style = if position == cursor {
                Style::default()
                    .bg(theme().selected_bg)
                    .fg(theme().fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme().fg)
            };
            frame.render_widget(
                Paragraph::new(line).style(style),
                Rect {
                    x: list_area.x,
                    y: list_area.y + row as u16,
                    width: list_area.width,
                    height: 1,
                },
            );
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Enter play · ^A enqueue · ^R play all · ^P/^N history · filters: artist: year: · Esc",
            dim_style(),
        ))),
        footer_area,
    );
}

fn draw_toast(frame: &mut Frame, area: Rect, toast: &Toast) {
    if area.width < 12 || area.height < 2 {
        return;
    }
    let (background, foreground) = match toast.level {
        ToastLevel::Info => (theme().accent, theme().bg),
        ToastLevel::Success => (theme().green, theme().bg),
        ToastLevel::Error => (theme().red, theme().bg),
    };
    let message = format!(" {} ", toast.message);
    let width = (message.width() as u16)
        .min(area.width.saturating_sub(4))
        .max(6);
    let rect = Rect {
        x: area.x + area.width.saturating_sub(width + 1),
        y: area.y + 2,
        width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            fit(&message, width as usize),
            Style::default()
                .fg(foreground)
                .bg(background)
                .add_modifier(Modifier::BOLD),
        ))),
        rect,
    );
}

fn draw_palette(app: &App, frame: &mut Frame, area: Rect) {
    let rect = centered_rect(area, 76, 22);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_style(Style::default().fg(theme().accent))
        .title(Span::styled(
            format!(" Commands — {} matches ", app.palette.results.len()),
            Style::default()
                .fg(theme().accent)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 3 || inner.width == 0 {
        return;
    }
    let [query_area, rule_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);

    let query_width = query_area.width.saturating_sub(3) as usize;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "> ",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(fit(&app.palette.query, query_width)),
            Span::styled("█", Style::default().fg(theme().accent)),
        ])),
        query_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(rule_area.width as usize),
            dim_style(),
        ))),
        rule_area,
    );

    if app.palette.results.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching commands")
                .style(dim_style())
                .alignment(Alignment::Center),
            list_area,
        );
    } else {
        let height = list_area.height as usize;
        let start = app
            .palette
            .cursor
            .saturating_sub(height.saturating_sub(1))
            .min(app.palette.results.len().saturating_sub(height));
        let end = (start + height).min(app.palette.results.len());
        for (row, position) in (start..end).enumerate() {
            let Some(label) = app.palette.result_label(position) else {
                continue;
            };
            let selected = position == app.palette.cursor;
            let style = if selected {
                Style::default()
                    .bg(theme().selected_bg)
                    .fg(theme().fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme().fg)
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::raw(label))).style(style),
                Rect {
                    x: list_area.x,
                    y: list_area.y + row as u16,
                    width: list_area.width,
                    height: 1,
                },
            );
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Enter run · Esc close · Ctrl-U clear",
            dim_style(),
        ))),
        footer_area,
    );
}

fn draw_input(frame: &mut Frame, area: Rect, input: &InputModal) {
    let rect = centered_rect(area, 70, 7);
    frame.render_widget(Clear, rect);
    let width = rect.width.saturating_sub(4) as usize;
    let text = Text::from(vec![
        Line::from(Span::styled(
            input.prompt.clone(),
            Style::default().fg(theme().fg).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(vec![
            Span::styled(
                "> ",
                Style::default()
                    .fg(theme().accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(fit(&input.value, width.saturating_sub(1))),
            Span::styled("█", Style::default().fg(theme().accent)),
        ]),
        Line::default(),
        Line::from(Span::styled("Enter confirm · Esc cancel", dim_style())),
    ]);
    frame.render_widget(
        Paragraph::new(text).style(base_style()).block(
            Block::bordered()
                .title(Span::styled(
                    " Input ",
                    Style::default()
                        .fg(theme().accent)
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(theme().accent)),
        ),
        rect,
    );
}

fn draw_confirm(frame: &mut Frame, area: Rect, confirm: &Confirm) {
    let rect = centered_rect(area, 60, 5);
    frame.render_widget(Clear, rect);
    let text = Text::from(vec![
        Line::from(Span::styled(
            confirm.message.clone(),
            Style::default().fg(theme().fg).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(vec![
            Span::styled("[y] yes", Style::default().fg(theme().green)),
            Span::raw("     "),
            Span::styled("[n] no", dim_style()),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(text)
            .alignment(Alignment::Center)
            .style(base_style())
            .block(
                Block::bordered()
                    .title(Span::styled(
                        " Confirm ",
                        Style::default()
                            .fg(theme().red)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::default().fg(theme().red)),
            ),
        rect,
    );
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let rect = centered_rect(area, 90, 42);
    frame.render_widget(Clear, rect);

    let key = |keys: &'static str, description: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {keys:<10}"), Style::default().fg(theme().accent)),
            Span::raw(description),
        ])
    };
    let heading = |title: &'static str| {
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(theme().purple)
                .add_modifier(Modifier::BOLD),
        ))
    };

    let lines = vec![
        heading("Global"),
        key("? :", "help / command palette"),
        key("q", "quit"),
        key("Tab 0-9", "next screen / jump to screen"),
        key("R", "reload MPD state and library"),
        heading("Playback"),
        key("space p s", "play or pause / stop"),
        key("> . n", "next track"),
        key("< , N", "previous track"),
        key("f b", "seek +5s / -5s"),
        key("+ -", "volume up / down"),
        key("r z x c", "repeat / random / single / consume"),
        heading("Queue"),
        key("j k g G", "move cursor"),
        key("Enter", "play selected queue item"),
        key("d J K", "remove / move selected"),
        key("C S", "clear (confirms) / shuffle"),
        heading("Library"),
        key("h l", "previous / next pane"),
        key("Enter a A", "play / enqueue / replace and play"),
        heading("Playlists"),
        key("h l", "playlists / songs pane"),
        key("Enter a", "play / enqueue"),
        key("S e x", "save queue / rename / delete"),
        key("d J K", "remove / move song"),
        heading("Browser"),
        key("0", "file browser"),
        key("h bksp", "parent directory"),
        key("Enter", "open directory / play song"),
        key("a A", "enqueue / replace queue and play"),
        heading("Screens and extras"),
        key("4", "outputs — Enter toggles"),
        key("5", "statistics"),
        key("6", "lyrics — o refetch, [ ] offset"),
        key("7", "visualizer"),
        key("8", "tag editor — Enter edits, w writes"),
        key("9", "history — Enter plays top song"),
    ];

    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(base_style()).block(
            Block::bordered()
                .title(Span::styled(
                    " Help ",
                    Style::default()
                        .fg(theme().accent)
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(theme().accent)),
        ),
        rect,
    );
}

fn centered_rect(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = max_width.min(area.width);
    let height = max_height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn kv(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<14}"), dim_style()),
        Span::raw(value.into()),
    ])
}

fn short_date(value: &str) -> String {
    value.get(..10).unwrap_or(value).to_string()
}

fn format_long_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let minutes = (total % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn age_text(unix_seconds: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    if now >= unix_seconds {
        format!(
            "{} ago",
            format_long_duration(Duration::from_secs(now - unix_seconds))
        )
    } else {
        "unknown".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_fill_the_row() {
        for width in [70, 90, 120, 200] {
            let columns = song_columns(width);
            let fixed = 2
                + 4
                + 1
                + 1
                + 7
                + if columns.show_artist { 21 } else { 0 }
                + if columns.show_album { 21 } else { 0 };
            assert_eq!(fixed + columns.title, width);
        }
    }

    #[test]
    fn compact_rows_leave_room_for_the_title() {
        assert_eq!(40usize.saturating_sub(15) + 15, 40);
    }

    #[test]
    fn parses_theme_colors() {
        assert_eq!(
            parse_color("#112233", Color::Reset),
            Color::Rgb(0x11, 0x22, 0x33)
        );
        assert_eq!(parse_color("red", Color::Reset), Color::Red);
        assert_eq!(parse_color("bogus", Color::Yellow), Color::Yellow);
        assert_eq!(parse_color("#xyzxyz", Color::Yellow), Color::Yellow);
    }

    #[test]
    fn long_durations_are_human_readable() {
        assert_eq!(format_long_duration(Duration::from_secs(30)), "0m");
        assert_eq!(format_long_duration(Duration::from_secs(3_600)), "1h 0m");
        assert_eq!(
            format_long_duration(Duration::from_secs(90_000)),
            "1d 1h 0m"
        );
    }
}
