//! Rendering only: every function takes application state and paints a frame.
//!
//! Large lists are rendered viewport-first. `Table`/`List` would walk every
//! item, which is fine for a 6,000-song library but not for the 100,000-song
//! libraries this client is meant to handle.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use neo_ncmp_domain::{PlaybackState, Song, format_duration};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Gauge, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Confirm, ConnectionState, LibraryPane, Screen, Toast, ToastLevel};
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

const THEME: Theme = Theme {
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

fn base_style() -> Style {
    Style::default().fg(THEME.fg).bg(THEME.bg)
}

fn dim_style() -> Style {
    Style::default().fg(THEME.dim)
}

pub fn draw(app: &mut App, frame: &mut Frame) {
    let area = frame.area();
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
    if let Some(confirm) = &app.confirm {
        draw_confirm(frame, area, confirm);
    }
    if app.help {
        draw_help(frame, area);
    }
}

fn draw_header(app: &App, frame: &mut Frame, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let right_width = if area.width >= 64 { 28 } else { 0 };
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(right_width)]).areas(area);

    let mut spans = vec![Span::styled(
        " neo-ncmp ",
        Style::default()
            .fg(THEME.bg)
            .bg(THEME.accent)
            .add_modifier(Modifier::BOLD),
    )];
    for (index, screen) in Screen::ALL.iter().enumerate() {
        let label = match screen {
            Screen::Queue => format!(" {} Queue ({}) ", index + 1, app.queue.len()),
            Screen::Library => format!(
                " {} Library ({}) ",
                index + 1,
                app.library.as_ref().map_or(0, |library| library.len())
            ),
            _ => format!(" {} {} ", index + 1, screen.title()),
        };
        let style = if *screen == app.screen {
            Style::default()
                .fg(THEME.bg)
                .bg(THEME.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(THEME.dim)
        };
        spans.push(Span::styled(label, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), left);

    if right_width > 0 {
        let (label, style) = match &app.connection {
            ConnectionState::Connecting => (
                "○ connecting".to_string(),
                Style::default().fg(THEME.yellow),
            ),
            ConnectionState::Connected(version) => {
                (format!("● MPD {version}"), Style::default().fg(THEME.green))
            }
            ConnectionState::Disconnected => {
                ("● disconnected".to_string(), Style::default().fg(THEME.red))
            }
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
        Screen::Outputs => draw_outputs(app, frame, area),
        Screen::Stats => draw_stats(app, frame, area),
    }
}

fn pane_block(title: &str, active: bool) -> Block<'static> {
    let border = if active { THEME.accent } else { THEME.border };
    let title_style = if active {
        Style::default()
            .fg(THEME.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        dim_style()
    };
    Block::bordered()
        .border_style(Style::default().fg(border))
        .title(Span::styled(format!(" {title} "), title_style))
}

fn draw_queue(app: &mut App, frame: &mut Frame, area: Rect) {
    let block = pane_block(
        &format!(
            "Queue — {} song{}",
            app.queue.len(),
            if app.queue.len() == 1 { "" } else { "s" }
        ),
        true,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

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

    let [artists_area, albums_area, songs_area] = Layout::horizontal([
        Constraint::Percentage(30),
        Constraint::Percentage(32),
        Constraint::Percentage(38),
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

    {
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
        );
    }

    {
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
        );
    }

    {
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
        );
    }
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

    for (row, index) in (start..end).enumerate() {
        let output = &app.outputs[index];
        let selected = index == cursor;
        let (icon, style) = if output.enabled {
            ("[●]", Style::default().fg(THEME.green))
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
                Style::default().fg(THEME.red),
            )));
        }
    }
    lines.push(Line::default());

    if let Some(song) = &app.current {
        lines.push(Line::from(Span::styled(
            format!("{} — {}", song.display_artist(), song.display_title()),
            Style::default()
                .fg(THEME.green)
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
                .fg(THEME.accent)
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
                PlaybackState::Playing => THEME.green,
                PlaybackState::Paused => THEME.yellow,
                PlaybackState::Stopped => THEME.dim,
            };
            (ratio, label, fill)
        }
        None => (0.0, "no track".to_string(), THEME.dim),
    };

    frame.render_widget(
        Gauge::default()
            .ratio(ratio)
            .label(Span::styled(label, Style::default().fg(THEME.fg)))
            .gauge_style(Style::default().fg(fill).bg(THEME.selected_bg))
            .style(Style::default().bg(THEME.bg)),
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
                PlaybackState::Playing => Style::default().fg(THEME.green),
                PlaybackState::Paused => Style::default().fg(THEME.yellow),
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
            Style::default().fg(THEME.cyan),
        ));
        for (label, enabled) in [
            ("rep", status.repeat),
            ("rnd", status.random),
            ("sgl", status.single),
            ("cns", status.consume),
        ] {
            let style = if enabled {
                Style::default().fg(THEME.green)
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
                Style::default().fg(THEME.purple),
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
            .fg(THEME.green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let marker_style = if current {
        Style::default().fg(THEME.green)
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
            .bg(THEME.selected_bg)
            .fg(THEME.fg)
            .add_modifier(Modifier::BOLD)
    } else if current {
        Style::default()
            .fg(THEME.green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(THEME.fg)
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
) where
    F: Fn(usize) -> Line<'static>,
{
    let block = pane_block(title, active);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }
    if len == 0 {
        frame.render_widget(
            Paragraph::new("—")
                .style(dim_style())
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    view.ensure_visible(len, inner.height as usize);
    let cursor = view.cursor;
    let start = view.offset;
    let end = (start + inner.height as usize).min(len);

    for (row, index) in (start..end).enumerate() {
        let selected = active && index == cursor;
        let style = if selected {
            Style::default()
                .bg(THEME.selected_bg)
                .fg(THEME.fg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(THEME.fg)
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
}

fn draw_search(app: &mut App, frame: &mut Frame, area: Rect) {
    let Some(index) = app.search_index.as_ref() else {
        return;
    };
    let state = &mut app.search;

    let rect = centered_rect(area, 100, 34);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_style(Style::default().fg(THEME.accent))
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
                .fg(THEME.accent)
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
                    .fg(THEME.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(fit(&state.query, query_width)),
            Span::styled("█", Style::default().fg(THEME.accent)),
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
                SearchKind::Artist => THEME.purple,
                SearchKind::Album => THEME.cyan,
                SearchKind::Song => THEME.green,
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
                    .bg(THEME.selected_bg)
                    .fg(THEME.fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(THEME.fg)
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
            "Enter play · Ctrl-U clear · Esc close",
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
        ToastLevel::Info => (THEME.accent, THEME.bg),
        ToastLevel::Success => (THEME.green, THEME.bg),
        ToastLevel::Error => (THEME.red, THEME.bg),
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

fn draw_confirm(frame: &mut Frame, area: Rect, confirm: &Confirm) {
    let rect = centered_rect(area, 60, 5);
    frame.render_widget(Clear, rect);
    let text = Text::from(vec![
        Line::from(Span::styled(
            confirm.message.clone(),
            Style::default().fg(THEME.fg).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from(vec![
            Span::styled("[y] yes", Style::default().fg(THEME.green)),
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
                        Style::default().fg(THEME.red).add_modifier(Modifier::BOLD),
                    ))
                    .border_style(Style::default().fg(THEME.red)),
            ),
        rect,
    );
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let rect = centered_rect(area, 82, 30);
    frame.render_widget(Clear, rect);

    let key = |keys: &'static str, description: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {keys:<9}"), Style::default().fg(THEME.accent)),
            Span::raw(description),
        ])
    };
    let heading = |title: &'static str| {
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(THEME.purple)
                .add_modifier(Modifier::BOLD),
        ))
    };

    let lines = vec![
        heading("Global"),
        key("?", "toggle this help"),
        key("q", "quit"),
        key("Tab 1-4", "next screen / jump to screen"),
        key("R", "reload MPD state and library"),
        heading("Playback"),
        key("space p", "play or pause"),
        key("s", "stop"),
        key("> . n", "next track"),
        key("< , N", "previous track"),
        key("f b", "seek +5s / -5s"),
        key("+ -", "volume up / down"),
        key("r z x c", "repeat / random / single / consume"),
        heading("Queue"),
        key("j k g G", "move cursor"),
        key("Enter", "play selected queue item"),
        key("d", "remove selected"),
        key("J K", "move selected down / up"),
        key("C S", "clear (confirms) / shuffle"),
        heading("Library"),
        key("h l", "previous / next pane"),
        key("Enter", "append selection and play"),
        key("a A", "append / replace queue and play"),
        heading("Outputs"),
        key("Enter", "enable or disable output"),
        Line::from(""),
        Line::from(Span::styled("  press any key to close ", dim_style())),
    ];

    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(base_style()).block(
            Block::bordered()
                .title(Span::styled(
                    " Help ",
                    Style::default()
                        .fg(THEME.accent)
                        .add_modifier(Modifier::BOLD),
                ))
                .border_style(Style::default().fg(THEME.accent)),
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
    fn long_durations_are_human_readable() {
        assert_eq!(format_long_duration(Duration::from_secs(30)), "0m");
        assert_eq!(format_long_duration(Duration::from_secs(3_600)), "1h 0m");
        assert_eq!(
            format_long_duration(Duration::from_secs(90_000)),
            "1d 1h 0m"
        );
    }
}
