// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! UI rendering functions.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Bar, BarChart, BarGroup, Block, Borders, Clear, List, ListItem, ListState, Paragraph,
        canvas::{Canvas, Line as CanvasLine},
    },
};
use resid::ChipModel;

use super::app::{App, BrowserFocus, Popup};
use super::theme::{ColorScheme, SCHEMES, c64};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let full_area = frame.area();
    let scheme = app.scheme();

    // Fill background with scheme color
    frame.render_widget(
        Block::default().style(Style::default().bg(scheme.background)),
        full_area,
    );

    let [browser_area, player_area] =
        Layout::horizontal([Constraint::Length(32), Constraint::Min(60)]).areas(full_area);

    let [playlist_area, hvsc_area] =
        Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).areas(browser_area);

    draw_playlist_browser(frame, playlist_area, app);
    draw_hvsc_browser(frame, hvsc_area, app);

    let [header_area, main_area, footer_area] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Min(10),
        Constraint::Length(1),
    ])
    .areas(player_area);

    let [vu_area, scope_area] =
        Layout::horizontal([Constraint::Length(40), Constraint::Min(30)]).areas(main_area);

    draw_header(frame, header_area, app);
    draw_vu_meters(frame, vu_area, app);
    draw_voice_scopes(frame, scope_area, app);
    draw_footer(frame, footer_area, app);
    draw_popup(frame, app);
}

fn draw_playlist_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    let scheme = *app.scheme();
    let is_focused = app.browser_focus == BrowserFocus::Playlist;
    let border_color = if is_focused {
        scheme.border_focus
    } else {
        scheme.border_dim
    };

    let block = Block::default()
        .title(" Playlist ")
        .title_style(Style::default().fg(scheme.title).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let items: Vec<ListItem> = app
        .playlist_browser
        .playlist
        .entries
        .iter()
        .map(|entry| {
            let mut name = entry.display_name.clone();
            if let Some(sub) = entry.subsong {
                name.push_str(&format!(" @{sub}"));
            }
            ListItem::new(name).style(Style::default().fg(scheme.text_primary))
        })
        .collect();

    let inner_height = area.height.saturating_sub(2) as usize;
    let selected = app.playlist_browser.selected_index();
    let offset = selected.saturating_sub(inner_height / 2);
    *app.playlist_browser.state.offset_mut() = offset;

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(scheme.highlight_bg)
                .fg(scheme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(if is_focused { "> " } else { "  " });

    frame.render_stateful_widget(list, area, &mut app.playlist_browser.state);
}

/// Formats HVSC entry for display, enriching with STIL metadata when available.
fn format_hvsc_entry(
    entry: &crate::hvsc::HvscEntry,
    stil: Option<&crate::hvsc::StilDatabase>,
    scheme: &ColorScheme,
) -> (String, Style) {
    if entry.is_dir {
        return (
            format!("{}/", entry.name),
            Style::default().fg(scheme.accent),
        );
    }

    let stil_title = stil
        .and_then(|db| db.get(&entry.path))
        .and_then(|info| info.title.as_ref());

    let display = match stil_title {
        Some(title) => format!("{} - {title}", entry.name.trim_end_matches(".sid")),
        None => entry.name.clone(),
    };

    (display, Style::default().fg(scheme.text_primary))
}

fn draw_hvsc_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    let scheme = *app.scheme();
    let is_focused = app.browser_focus == BrowserFocus::Hvsc;
    let border_color = if is_focused {
        scheme.border_focus
    } else {
        scheme.border_dim
    };

    if app.hvsc_search.is_some() {
        draw_hvsc_search_results(frame, area, app, &scheme, border_color);
    } else {
        draw_hvsc_directory(frame, area, app, &scheme, is_focused, border_color);
    }
}

fn draw_hvsc_search_results(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    scheme: &ColorScheme,
    border_color: Color,
) {
    let query = app.hvsc_search.as_deref().unwrap_or("");
    let count = app.hvsc_search_results.len();
    let title = if let Some(err) = &app.hvsc_browser.stil_error {
        format!(" Search: {}_ [{}] ", query, err)
    } else {
        match &app.hvsc_browser.stil {
            None => format!(" Search: {}_ [STIL not loaded] ", query),
            Some(stil) => format!(" Search: {}_ ({} of {} entries) ", query, count, stil.len()),
        }
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(scheme.accent).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let items: Vec<ListItem> = app
        .hvsc_search_results
        .iter()
        .map(|path| {
            let name = path.rsplit('/').next().unwrap_or(path);
            ListItem::new(name).style(Style::default().fg(scheme.text_primary))
        })
        .collect();

    let mut list_state = ListState::default();
    if !app.hvsc_search_results.is_empty() {
        list_state.select(Some(app.hvsc_search_index));
    }

    let inner_height = area.height.saturating_sub(2) as usize;
    let offset = app.hvsc_search_index.saturating_sub(inner_height / 2);
    *list_state.offset_mut() = offset;

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(scheme.highlight_bg)
                .fg(scheme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_hvsc_directory(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    scheme: &ColorScheme,
    is_focused: bool,
    border_color: Color,
) {
    let title = if app.hvsc_browser.current_path == "/" {
        " HVSC (/ to search) ".to_string()
    } else {
        format!(" HVSC: {} ", app.hvsc_browser.current_path)
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(scheme.title).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let items: Vec<ListItem> = app
        .hvsc_browser
        .entries
        .iter()
        .map(|entry| {
            let (name, style) = format_hvsc_entry(entry, app.hvsc_browser.stil.as_ref(), scheme);
            ListItem::new(name).style(style)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(app.hvsc_browser.selected));

    let inner_height = area.height.saturating_sub(2) as usize;
    let selected = app.hvsc_browser.selected;
    let offset = selected.saturating_sub(inner_height / 2);
    *list_state.offset_mut() = offset;

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(scheme.highlight_bg)
                .fg(scheme.highlight_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(if is_focused { "> " } else { "  " });

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let scheme = app.scheme();

    let block = Block::default()
        .title(" SID Player ")
        .title_style(Style::default().fg(scheme.title).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(scheme.border_dim));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [info_area, logo_area] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(32)]).areas(inner);

    frame.render_widget(Paragraph::new(sid_info_lines(app)), info_area);
    frame.render_widget(Paragraph::new(logo_lines()), logo_area);
}

fn sid_info_lines(app: &App) -> Vec<Line<'static>> {
    let scheme = app.scheme();
    let sid = app.display_sid();
    let label = Style::default().fg(scheme.text_secondary);

    let remaining = app.song_timeout.saturating_sub(app.song_elapsed_total());
    let mins = remaining.as_secs() / 60;
    let secs = remaining.as_secs() % 60;
    let time_str = format!(" [{mins}:{secs:02}]");

    let status = if app.paused {
        Span::styled("  [PAUSED]", Style::default().fg(scheme.title).bold())
    } else {
        Span::styled(
            format!("  [PLAYING]{time_str}"),
            Style::default().fg(scheme.accent),
        )
    };

    let chip = match app.chip_model {
        ChipModel::Mos6581 => "[6581]",
        ChipModel::Mos8580 => "[8580]",
    };

    vec![
        Line::from(vec![
            Span::styled("Title:    ", label),
            Span::styled(
                sid.name.clone(),
                Style::default().fg(scheme.text_primary).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Author:   ", label),
            Span::styled(sid.author.clone(), Style::default().fg(scheme.accent)),
        ]),
        Line::from(vec![
            Span::styled("Released: ", label),
            Span::styled(
                sid.released.clone(),
                Style::default().fg(scheme.text_secondary),
            ),
        ]),
        Line::from(vec![
            Span::styled("Song:     ", label),
            Span::styled(
                format!("{} / {}", app.current_song, app.total_songs),
                Style::default().fg(scheme.accent),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(chip, Style::default().fg(scheme.text_secondary)),
            status,
        ]),
    ]
}

/// Returns the CrabSid logo with fixed C64 rainbow colors.
fn logo_lines() -> Vec<Line<'static>> {
    let crab = Style::default().fg(c64::ORANGE);
    let c = Style::default().fg(c64::LIGHT_RED);
    let r = Style::default().fg(c64::ORANGE);
    let a = Style::default().fg(c64::YELLOW);
    let b = Style::default().fg(c64::GREEN);
    let s = Style::default().fg(c64::CYAN);
    let i = Style::default().fg(c64::LIGHT_BLUE);
    let d = Style::default().fg(c64::PURPLE);

    vec![
        Line::from(vec![
            Span::styled(" (\\/)  ", crab),
            Span::styled("╔═╗ ", c),
            Span::styled("╦═╗ ", r),
            Span::styled("╔═╗ ", a),
            Span::styled("╔╗  ", b),
            Span::styled("╔═╗ ", s),
            Span::styled("╦ ", i),
            Span::styled("╔╦╗", d),
        ]),
        Line::from(vec![
            Span::styled("( °°)  ", crab),
            Span::styled("║   ", c),
            Span::styled("╠╦╝ ", r),
            Span::styled("╠═╣ ", a),
            Span::styled("╠╩╗ ", b),
            Span::styled("╚═╗ ", s),
            Span::styled("║ ", i),
            Span::styled(" ║║", d),
        ]),
        Line::from(vec![
            Span::styled(" /||\\  ", crab),
            Span::styled("╚═╝ ", c),
            Span::styled("╩╚═ ", r),
            Span::styled("╩ ╩ ", a),
            Span::styled("╚═╝ ", b),
            Span::styled("╚═╝ ", s),
            Span::styled("╩ ", i),
            Span::styled("═╩╝", d),
        ]),
        Line::from(vec![
            Span::raw("                      "),
            Span::styled("B", d),
            Span::styled("y", i),
            Span::raw(" "),
            Span::styled("W", s),
            Span::styled("o", b),
            Span::styled("m", a),
            Span::styled("b", r),
            Span::styled("a", c),
            Span::styled("t", d),
        ]),
    ]
}

fn draw_vu_meters(frame: &mut Frame, area: Rect, app: &App) {
    let scheme = app.scheme();

    let block = Block::default()
        .title(" Voice Levels ")
        .title_style(Style::default().fg(scheme.title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(scheme.border_dim));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let voice_names = ["Voice 1", "Voice 2", "Voice 3"];

    let bars: Vec<Bar> = (0..3)
        .map(|i| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let level = (app.vu_meter.levels[i] * 100.0) as u64;
            Bar::default()
                .value(level)
                .label(Line::from(voice_names[i]))
                .style(Style::default().fg(scheme.voices[i]))
                .value_style(Style::default().fg(scheme.text_primary).bold())
        })
        .collect();

    let chart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .bar_width(8)
        .bar_gap(3)
        .max(100)
        .direction(ratatui::layout::Direction::Vertical);

    let chart_width = 3 * 8 + 2 * 3;
    let [_, centered, _] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(chart_width),
        Constraint::Min(0),
    ])
    .areas(inner);

    frame.render_widget(chart, centered);
}

fn draw_voice_scopes(frame: &mut Frame, area: Rect, app: &App) {
    let scheme = app.scheme();
    let voice_names = ["Voice 1", "Voice 2", "Voice 3"];

    let areas = Layout::vertical([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .areas::<3>(area);

    for (i, &voice_area) in areas.iter().enumerate() {
        draw_single_scope(
            frame,
            voice_area,
            &app.voice_scopes.samples[i],
            voice_names[i],
            scheme.voices[i],
            scheme.border_dim,
        );
    }
}

fn draw_single_scope(
    frame: &mut Frame,
    area: Rect,
    samples: &[f32],
    title: &str,
    color: Color,
    border: Color,
) {
    let block = Block::default()
        .title(format!(" {title} "))
        .title_style(Style::default().fg(color))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let width = f64::from(inner.width);
    #[allow(clippy::cast_precision_loss)]
    let x_scale = width / samples.len() as f64;

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([0.0, width])
        .y_bounds([0.0, 1.0])
        .paint(|ctx| {
            for i in 0..samples.len().saturating_sub(1) {
                #[allow(clippy::cast_precision_loss)]
                let x1 = i as f64 * x_scale;
                #[allow(clippy::cast_precision_loss)]
                let x2 = (i + 1) as f64 * x_scale;
                let y1 = f64::from(samples[i]);
                let y2 = f64::from(samples[i + 1]);

                ctx.draw(&CanvasLine {
                    x1,
                    y1,
                    x2,
                    y2,
                    color,
                });
            }
        });

    frame.render_widget(canvas, inner);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let scheme = app.scheme();
    let key = Style::default().fg(scheme.accent).bold();
    let dim = Style::default().fg(scheme.text_secondary);
    let sep = Style::default().fg(scheme.border_dim);

    let spans = vec![
        Span::styled(" h", key),
        Span::styled(" Help ", dim),
        Span::styled("\u{2502} ", sep),
        Span::styled("1-9/+/-", key),
        Span::styled(" Song ", dim),
        Span::styled("\u{2502} ", sep),
        Span::styled("Tab", key),
        Span::styled(" Switch ", dim),
        Span::styled("\u{2502} ", sep),
        Span::styled("c", key),
        Span::styled(" Color ", dim),
        Span::styled("\u{2502} ", sep),
        Span::styled("a", key),
        Span::styled(" Add ", dim),
        Span::styled("\u{2502} ", sep),
        Span::styled("q", key),
        Span::styled(" Quit", dim),
    ];

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_color_scheme_popup(frame: &mut Frame, app: &App) {
    let scheme = app.scheme();
    let area = centered_rect(25, 50, frame.area());

    frame.render_widget(Clear, area);

    let items: Vec<ListItem> = SCHEMES
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == app.color_scheme {
                Style::default()
                    .fg(scheme.highlight_fg)
                    .bg(scheme.highlight_bg)
            } else {
                Style::default().fg(scheme.text_primary)
            };
            ListItem::new(format!(" {} ", s.name)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Color Scheme ")
            .title_style(Style::default().fg(scheme.title).bold())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(scheme.border_focus))
            .style(Style::default().bg(scheme.background)),
    );

    frame.render_widget(list, area);
}

fn draw_popup(frame: &mut Frame, app: &App) {
    if matches!(app.popup, Popup::ColorScheme) {
        draw_color_scheme_popup(frame, app);
        return;
    }

    let scheme = app.scheme();

    let (title, content, small) = match &app.popup {
        Popup::None | Popup::ColorScheme => return,
        Popup::Help => (" Help ", help_text(scheme), true),
        Popup::Error(msg) => (" Error ", vec![Line::from(msg.as_str())], false),
        Popup::SaveConfirm => (
            " Save Playlist? ",
            vec![
                Line::from(""),
                Line::from("  Save changes before quitting?"),
                Line::from(""),
                Line::from(vec![
                    Span::raw("    "),
                    Span::styled("Y", Style::default().fg(scheme.accent).bold()),
                    Span::raw("/Enter = Save    "),
                    Span::styled("N", Style::default().fg(scheme.title).bold()),
                    Span::raw(" = Discard"),
                ]),
            ],
            true,
        ),
        Popup::HvscSearch => {
            let query = app.hvsc_search.as_deref().unwrap_or("");
            let line = Line::from(vec![
                Span::styled(" > ", Style::default().fg(scheme.accent)),
                Span::raw(query),
                Span::styled("_", Style::default().fg(scheme.accent)),
            ]);
            (
                " STIL Search ",
                vec![
                    Line::from("  Type search text, Enter to search, Esc to cancel"),
                    Line::from(""),
                    line,
                ],
                true,
            )
        }
    };

    let area = if small {
        centered_rect(40, 35, frame.area())
    } else {
        centered_rect(60, 70, frame.area())
    };

    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(scheme.title).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(scheme.border_focus));

    let para = Paragraph::new(content).block(block);
    frame.render_widget(para, area);
}

fn help_text(scheme: &ColorScheme) -> Vec<Line<'static>> {
    let key = Style::default().fg(scheme.accent);
    let hdr = Style::default().fg(scheme.title).bold();
    let dim = Style::default().fg(scheme.text_secondary);

    macro_rules! row {
        ($k1:expr, $d1:expr, $k2:expr, $d2:expr) => {
            Line::from(vec![
                Span::styled(format!(" {:<5}", $k1), key),
                Span::raw(format!("{:<11}", $d1)),
                Span::styled("│", dim),
                Span::styled(format!(" {:<5}", $k2), key),
                Span::raw($d2),
            ])
        };
    }

    vec![
        Line::from(vec![
            Span::styled(" Player          ", hdr),
            Span::styled("│", dim),
            Span::styled(" Browser", hdr),
        ]),
        row!("SPC", "Play/pause", "↑↓", "Navigate"),
        row!("1-9", "Subsong", "Enter", "Open/play"),
        row!("+/-", "Next/prev", "←/BS", "Parent dir"),
        row!("s", "6581/8580", "/", "Search STIL"),
        row!("c", "Colors", "Tab", "Switch panel"),
        row!("a", "Add to list", "BS", "Remove item"),
        Line::from("─────────────────┴────────────────"),
        Line::from(vec![
            Span::styled(" h/?", key),
            Span::raw(" Help   "),
            Span::styled("q", key),
            Span::raw(" Quit"),
        ]),
    ]
}

/// Creates a centered rectangle for popups.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [_, center, _] = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .areas(center);

    center
}
