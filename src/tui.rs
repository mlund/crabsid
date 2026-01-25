// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use crate::hvsc::HvscBrowser;
use crate::player::SharedPlayer;
use crate::playlist::Playlist;
use crate::sid_file::SidFile;
use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Bar, BarChart, BarGroup, Block, Borders, List, ListItem, ListState, Paragraph,
        canvas::{Canvas, Line as CanvasLine},
    },
};
use resid::ChipModel;
use std::io::{self, stdout};
use std::time::{Duration, Instant};

const TARGET_FPS: u64 = 30;
/// Number of samples to display in oscilloscope (downsampled from player buffer)
const SCOPE_DISPLAY_SAMPLES: usize = 256;

/// VU meter state with smoothed decay for visual appeal
pub struct VuMeter {
    levels: [f32; 3],
    peaks: [f32; 3],
    peak_hold: [Instant; 3],
}

impl VuMeter {
    /// Creates meters with all levels at zero.
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            levels: [0.0; 3],
            peaks: [0.0; 3],
            peak_hold: [now; 3],
        }
    }

    /// Update meters with new envelope values, applying smoothing
    pub fn update(&mut self, envelope: [u8; 3]) {
        const ATTACK_RATE: f32 = 0.7;
        const DECAY_RATE: f32 = 0.92;
        const PEAK_HOLD_MS: u128 = 500;

        let now = Instant::now();
        for (i, &env) in envelope.iter().enumerate() {
            let target = f32::from(env) / 255.0;

            // Fast attack, slow decay for classic VU behavior
            self.levels[i] = if target > self.levels[i] {
                (target - self.levels[i]).mul_add(ATTACK_RATE, self.levels[i])
            } else {
                self.levels[i] * DECAY_RATE
            };

            // Peak hold with decay
            if self.levels[i] >= self.peaks[i] {
                self.peaks[i] = self.levels[i];
                self.peak_hold[i] = now;
            } else if now.duration_since(self.peak_hold[i]).as_millis() > PEAK_HOLD_MS {
                self.peaks[i] *= 0.95;
            }
        }
    }
}

/// Per-voice envelope scope buffers
pub struct VoiceScopes {
    samples: [Vec<f32>; 3],
}

impl VoiceScopes {
    /// Creates scope buffers initialized to zero.
    pub fn new() -> Self {
        Self {
            samples: std::array::from_fn(|_| vec![0.0; SCOPE_DISPLAY_SAMPLES]),
        }
    }

    /// Downsample from player envelope buffers to display resolution
    pub fn update(&mut self, raw_samples: &[Vec<f32>; 3]) {
        for (display, raw) in self.samples.iter_mut().zip(raw_samples.iter()) {
            if raw.is_empty() {
                continue;
            }
            let step = raw.len() / SCOPE_DISPLAY_SAMPLES;
            if step == 0 {
                continue;
            }
            for (i, sample) in display.iter_mut().enumerate() {
                *sample = raw.get(i * step).copied().unwrap_or(0.0);
            }
        }
    }
}

/// Which browser panel has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserFocus {
    Playlist,
    Hvsc,
}

/// Browser state for playlist navigation.
pub struct PlaylistBrowser {
    playlist: Playlist,
    state: ListState,
}

impl PlaylistBrowser {
    fn new(playlist: Playlist) -> Self {
        let mut state = ListState::default();
        if !playlist.is_empty() {
            state.select(Some(0));
        }
        Self { playlist, state }
    }

    fn selected_index(&self) -> usize {
        self.state.selected().unwrap_or(0)
    }

    fn select_next(&mut self) {
        let len = self.playlist.len();
        if len == 0 {
            return;
        }
        let i = self.selected_index();
        self.state.select(Some((i + 1).min(len - 1)));
    }

    fn select_prev(&mut self) {
        self.state.select(Some(self.selected_index().saturating_sub(1)));
    }
}

/// TUI application state holding the player and display data.
pub struct App<'a> {
    player: SharedPlayer,
    sid_file: &'a SidFile,
    current_song: u16,
    total_songs: u16,
    paused: bool,
    chip_model: ChipModel,
    vu_meter: VuMeter,
    voice_scopes: VoiceScopes,
    /// Playlist browser (upper left)
    playlist_browser: Option<PlaylistBrowser>,
    /// HVSC browser (lower left)
    hvsc_browser: HvscBrowser,
    /// Which browser panel has focus
    browser_focus: BrowserFocus,
    /// Currently loaded SID from browsers (owned for URL loads)
    current_browser_sid: Option<SidFile>,
}

impl<'a> App<'a> {
    /// Creates the application with the given player and initial song.
    pub fn new(player: SharedPlayer, sid_file: &'a SidFile, song: u16) -> Self {
        let chip_model = player
            .lock()
            .map(|p| p.chip_model())
            .unwrap_or(ChipModel::Mos6581);

        let mut hvsc_browser = HvscBrowser::new();
        // Load STIL in the background (blocking for now, could be async)
        hvsc_browser.load_stil();

        Self {
            player,
            sid_file,
            current_song: song,
            total_songs: sid_file.songs,
            paused: false,
            chip_model,
            vu_meter: VuMeter::new(),
            voice_scopes: VoiceScopes::new(),
            playlist_browser: None,
            hvsc_browser,
            browser_focus: BrowserFocus::Hvsc,
            current_browser_sid: None,
        }
    }

    /// Creates the application with a playlist browser.
    pub fn with_playlist(
        player: SharedPlayer,
        sid_file: &'a SidFile,
        song: u16,
        playlist: Playlist,
    ) -> Self {
        let mut app = Self::new(player, sid_file, song);
        app.playlist_browser = Some(PlaylistBrowser::new(playlist));
        app.browser_focus = BrowserFocus::Playlist;
        app
    }

    fn update(&mut self) {
        if let Ok(player) = self.player.lock() {
            self.vu_meter.update(player.voice_levels());
            self.voice_scopes.update(&player.envelope_samples());
            self.paused = player.is_paused();
            self.chip_model = player.chip_model();
        }
    }

    fn toggle_pause(&mut self) {
        if let Ok(mut player) = self.player.lock() {
            player.toggle_pause();
            self.paused = player.is_paused();
        }
    }

    fn next_song(&mut self) {
        if self.current_song < self.total_songs {
            self.current_song += 1;
            if let Ok(mut player) = self.player.lock() {
                player.load_song(self.current_song);
            }
        }
    }

    fn prev_song(&mut self) {
        if self.current_song > 1 {
            self.current_song -= 1;
            if let Ok(mut player) = self.player.lock() {
                player.load_song(self.current_song);
            }
        }
    }

    fn switch_chip(&mut self) {
        if let Ok(mut player) = self.player.lock() {
            player.switch_chip_model();
            self.chip_model = player.chip_model();
        }
    }

    fn toggle_browser_focus(&mut self) {
        self.browser_focus = match self.browser_focus {
            BrowserFocus::Playlist if self.playlist_browser.is_some() => BrowserFocus::Hvsc,
            BrowserFocus::Hvsc if self.playlist_browser.is_some() => BrowserFocus::Playlist,
            _ => self.browser_focus,
        };
    }

    fn browser_next(&mut self) {
        match self.browser_focus {
            BrowserFocus::Playlist => {
                if let Some(browser) = &mut self.playlist_browser {
                    browser.select_next();
                }
            }
            BrowserFocus::Hvsc => self.hvsc_browser.select_next(),
        }
    }

    fn browser_prev(&mut self) {
        match self.browser_focus {
            BrowserFocus::Playlist => {
                if let Some(browser) = &mut self.playlist_browser {
                    browser.select_prev();
                }
            }
            BrowserFocus::Hvsc => self.hvsc_browser.select_prev(),
        }
    }

    fn browser_back(&mut self) {
        if self.browser_focus == BrowserFocus::Hvsc {
            self.hvsc_browser.go_up();
        }
    }

    /// Loads the currently selected entry (playlist or HVSC).
    fn load_selected(&mut self) {
        match self.browser_focus {
            BrowserFocus::Playlist => self.load_playlist_selected(),
            BrowserFocus::Hvsc => self.load_hvsc_selected(),
        }
    }

    fn load_playlist_selected(&mut self) {
        let Some(browser) = &self.playlist_browser else {
            return;
        };

        let idx = browser.selected_index();
        let Some(entry) = browser.playlist.entries.get(idx) else {
            return;
        };

        let sid_file = match entry.load() {
            Ok(f) => f,
            Err(_) => return,
        };

        let song = entry.subsong.unwrap_or(sid_file.start_song);
        self.play_sid_file(sid_file, song);
    }

    fn load_hvsc_selected(&mut self) {
        // Enter directory or load file
        if let Some(entry) = self.hvsc_browser.enter() {
            // It's a file, load it
            let sid_file = match entry.load() {
                Ok(f) => f,
                Err(_) => return,
            };
            let start_song = sid_file.start_song;
            self.play_sid_file(sid_file, start_song);
        }
    }

    fn play_sid_file(&mut self, sid_file: SidFile, song: u16) {
        self.current_song = song;
        self.total_songs = sid_file.songs;

        if let Ok(mut player) = self.player.lock() {
            player.load_sid_file(&sid_file, song);
            self.chip_model = player.chip_model();
        }

        self.current_browser_sid = Some(sid_file);
    }

    /// Returns the SID file to display metadata from.
    fn display_sid(&self) -> &SidFile {
        self.current_browser_sid.as_ref().unwrap_or(self.sid_file)
    }
}

/// Runs the TUI with an optional playlist browser.
pub fn run_with_playlist(
    player: SharedPlayer,
    sid_file: &SidFile,
    song: u16,
    playlist: Option<Playlist>,
) -> io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;

    let terminal = ratatui::init();
    let app = match playlist {
        Some(pl) => App::with_playlist(player, sid_file, song, pl),
        None => App::new(player, sid_file, song),
    };
    let result = run_app(terminal, app);

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_app(mut terminal: DefaultTerminal, mut app: App) -> io::Result<()> {
    let frame_duration = Duration::from_millis(1000 / TARGET_FPS);

    loop {
        let frame_start = Instant::now();

        app.update();
        terminal.draw(|frame| draw(frame, &mut app))?;

        // Poll for input with remaining frame time
        let elapsed = frame_start.elapsed();
        let timeout = frame_duration.saturating_sub(elapsed);

        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char(' ') => app.toggle_pause(),
                KeyCode::Right | KeyCode::Char('n') => app.next_song(),
                KeyCode::Left | KeyCode::Char('p') => app.prev_song(),
                KeyCode::Char('s') => app.switch_chip(),
                KeyCode::Up | KeyCode::Char('k') => app.browser_prev(),
                KeyCode::Down | KeyCode::Char('j') => app.browser_next(),
                KeyCode::Enter => app.load_selected(),
                KeyCode::Tab => app.toggle_browser_focus(),
                KeyCode::Backspace | KeyCode::Char('h') => app.browser_back(),
                _ => {}
            }
        }
    }
}

fn draw(frame: &mut Frame, app: &mut App) {
    let full_area = frame.area();

    // Always show browser panel on left
    let [browser_area, player_area] =
        Layout::horizontal([Constraint::Length(32), Constraint::Min(60)]).areas(full_area);

    // Split browser area: playlist on top (if present), HVSC on bottom
    if app.playlist_browser.is_some() {
        let [playlist_area, hvsc_area] =
            Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
                .areas(browser_area);
        draw_playlist_browser(frame, playlist_area, app);
        draw_hvsc_browser(frame, hvsc_area, app);
    } else {
        draw_hvsc_browser(frame, browser_area, app);
    }

    // Player area layout
    let [header_area, main_area, footer_area] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Min(10),
        Constraint::Length(1),
    ])
    .areas(player_area);

    // Split main area: VU meters left, oscilloscope right
    let [vu_area, scope_area] =
        Layout::horizontal([Constraint::Length(40), Constraint::Min(30)]).areas(main_area);

    draw_header(frame, header_area, app);
    draw_vu_meters(frame, vu_area, app);
    draw_voice_scopes(frame, scope_area, app);
    draw_footer(frame, footer_area, app);
}

fn draw_playlist_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(browser) = &mut app.playlist_browser else {
        return;
    };

    let is_focused = app.browser_focus == BrowserFocus::Playlist;
    let border_color = if is_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(" Playlist ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let items: Vec<ListItem> = browser
        .playlist
        .entries
        .iter()
        .map(|entry| {
            let mut name = entry.display_name.clone();
            if let Some(sub) = entry.subsong {
                name.push_str(&format!(" @{sub}"));
            }
            ListItem::new(name).style(Style::default().fg(Color::White))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(if is_focused { "> " } else { "  " });

    frame.render_stateful_widget(list, area, &mut browser.state);
}

/// Formats HVSC entry for display, enriching with STIL metadata when available.
fn format_hvsc_entry(entry: &crate::hvsc::HvscEntry, stil: Option<&crate::hvsc::StilDatabase>) -> (String, Style) {
    if entry.is_dir {
        return (format!("{}/", entry.name), Style::default().fg(Color::Yellow));
    }

    let stil_title = stil
        .and_then(|db| db.get(&entry.path))
        .and_then(|info| info.title.as_ref());

    let display = match stil_title {
        Some(title) => format!("{} - {title}", entry.name.trim_end_matches(".sid")),
        None => entry.name.clone(),
    };

    (display, Style::default().fg(Color::White))
}

fn draw_hvsc_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    let is_focused = app.browser_focus == BrowserFocus::Hvsc;
    let border_color = if is_focused { Color::Cyan } else { Color::DarkGray };

    let title = if app.hvsc_browser.current_path == "/" {
        " HVSC ".to_string()
    } else {
        format!(" HVSC: {} ", app.hvsc_browser.current_path)
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(Color::Yellow).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let items: Vec<ListItem> = app
        .hvsc_browser
        .entries
        .iter()
        .map(|entry| {
            let (name, style) = format_hvsc_entry(entry, app.hvsc_browser.stil.as_ref());
            ListItem::new(name).style(style)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(app.hvsc_browser.selected));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(if is_focused { "> " } else { "  " });

    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" SID Player ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sid = app.display_sid();
    let info = vec![
        Line::from(vec![
            Span::styled("Title:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(&sid.name, Style::default().fg(Color::White).bold()),
        ]),
        Line::from(vec![
            Span::styled("Author:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(&sid.author, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Released: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&sid.released, Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("Song:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} / {}", app.current_song, app.total_songs),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled("  ", Style::default()),
            Span::styled(
                match app.chip_model {
                    ChipModel::Mos6581 => "[6581]",
                    ChipModel::Mos8580 => "[8580]",
                },
                Style::default().fg(Color::Magenta),
            ),
            if app.paused {
                Span::styled("  [PAUSED]", Style::default().fg(Color::Yellow).bold())
            } else {
                Span::styled("  [PLAYING]", Style::default().fg(Color::Green))
            },
        ]),
    ];

    frame.render_widget(Paragraph::new(info), inner);
}

fn draw_vu_meters(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Voice Levels ")
        .title_style(Style::default().fg(Color::Cyan))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let voice_names = ["Voice 1", "Voice 2", "Voice 3"];
    let colors = [Color::Red, Color::Green, Color::Blue];

    // Scale levels to percentage for bar chart
    let bars: Vec<Bar> = (0..3)
        .map(|i| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let level = (app.vu_meter.levels[i] * 100.0) as u64;
            Bar::default()
                .value(level)
                .label(Line::from(voice_names[i]))
                .style(Style::default().fg(colors[i]))
                .value_style(Style::default().fg(Color::White).bold())
        })
        .collect();

    let chart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .bar_width(8)
        .bar_gap(3)
        .max(100)
        .direction(ratatui::layout::Direction::Vertical);

    frame.render_widget(chart, inner);
}

fn draw_voice_scopes(frame: &mut Frame, area: Rect, app: &App) {
    let voice_names = ["Voice 1", "Voice 2", "Voice 3"];
    let colors = [Color::Red, Color::Green, Color::Blue];

    // Split into three equal vertical sections
    let areas = Layout::vertical([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .areas::<3>(area);

    for (i, (&voice_area, (&name, &color))) in areas
        .iter()
        .zip(voice_names.iter().zip(colors.iter()))
        .enumerate()
    {
        draw_single_scope(frame, voice_area, &app.voice_scopes.samples[i], name, color);
    }
}

fn draw_single_scope(frame: &mut Frame, area: Rect, samples: &[f32], title: &str, color: Color) {
    let block = Block::default()
        .title(format!(" {title} "))
        .title_style(Style::default().fg(color))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let width = f64::from(inner.width);
    // Precision loss acceptable for display coordinates
    #[allow(clippy::cast_precision_loss)]
    let x_scale = width / samples.len() as f64;

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([0.0, width])
        .y_bounds([0.0, 1.0])
        .paint(|ctx| {
            // Draw waveform as connected line segments
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
    let pause_text = if app.paused { "Play" } else { "Pause" };

    let mut spans = vec![
        Span::styled(" SPACE", Style::default().fg(Color::Cyan).bold()),
        Span::styled(
            format!(" {pause_text} "),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2190}/p", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Prev ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2192}/n", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Next ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled("s", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" SID ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2191}\u{2193}", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Nav ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Play ", Style::default().fg(Color::DarkGray)),
        Span::styled("BS", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Back ", Style::default().fg(Color::DarkGray)),
    ];

    // Add Tab key if playlist is present
    if app.playlist_browser.is_some() {
        spans.extend([
            Span::styled("Tab", Style::default().fg(Color::Cyan).bold()),
            Span::styled(" Switch ", Style::default().fg(Color::DarkGray)),
        ]);
    }

    spans.extend([
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Quit", Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
