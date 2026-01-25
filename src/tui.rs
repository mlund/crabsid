// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

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

/// Browser state for playlist navigation.
pub struct Browser {
    playlist: Playlist,
    state: ListState,
    /// Currently loaded SID file (owned since we may load from URLs)
    current_sid: Option<SidFile>,
}

impl Browser {
    fn new(playlist: Playlist) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            playlist,
            state,
            current_sid: None,
        }
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
        let i = self.selected_index();
        self.state.select(Some(i.saturating_sub(1)));
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
    browser: Option<Browser>,
}

impl<'a> App<'a> {
    /// Creates the application with the given player and initial song.
    pub fn new(player: SharedPlayer, sid_file: &'a SidFile, song: u16) -> Self {
        let chip_model = player
            .lock()
            .map(|p| p.chip_model())
            .unwrap_or(ChipModel::Mos6581);
        Self {
            player,
            sid_file,
            current_song: song,
            total_songs: sid_file.songs,
            paused: false,
            chip_model,
            vu_meter: VuMeter::new(),
            voice_scopes: VoiceScopes::new(),
            browser: None,
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
        app.browser = Some(Browser::new(playlist));
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

    fn browser_next(&mut self) {
        if let Some(browser) = &mut self.browser {
            browser.select_next();
        }
    }

    fn browser_prev(&mut self) {
        if let Some(browser) = &mut self.browser {
            browser.select_prev();
        }
    }

    /// Loads the currently selected playlist entry.
    fn load_selected(&mut self) {
        let Some(browser) = &mut self.browser else {
            return;
        };

        let idx = browser.selected_index();
        let Some(entry) = browser.playlist.entries.get(idx) else {
            return;
        };

        let sid_file = match entry.load() {
            Ok(f) => f,
            Err(_) => return, // Silently skip on load error
        };

        let song = entry.subsong.unwrap_or(sid_file.start_song);
        self.current_song = song;
        self.total_songs = sid_file.songs;

        if let Ok(mut player) = self.player.lock() {
            player.load_sid_file(&sid_file, song);
            self.chip_model = player.chip_model();
        }

        browser.current_sid = Some(sid_file);
    }

    /// Returns the SID file to display metadata from (browser's current or initial).
    fn display_sid(&self) -> &SidFile {
        self.browser
            .as_ref()
            .and_then(|b| b.current_sid.as_ref())
            .unwrap_or(self.sid_file)
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
                _ => {}
            }
        }
    }
}

fn draw(frame: &mut Frame, app: &mut App) {
    let full_area = frame.area();

    // If browser exists, split horizontally: browser left, player right
    let (browser_area, player_area) = if app.browser.is_some() {
        let [left, right] =
            Layout::horizontal([Constraint::Length(30), Constraint::Min(60)]).areas(full_area);
        (Some(left), right)
    } else {
        (None, full_area)
    };

    // Draw browser if present
    if let Some(area) = browser_area {
        draw_browser(frame, area, app);
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

fn draw_browser(frame: &mut Frame, area: Rect, app: &mut App) {
    let Some(browser) = &mut app.browser else {
        return;
    };

    let block = Block::default()
        .title(" Playlist ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let items: Vec<ListItem> = browser
        .playlist
        .entries
        .iter()
        .map(|entry| {
            let style = Style::default().fg(Color::White);
            let mut name = entry.display_name.clone();
            if let Some(sub) = entry.subsong {
                name.push_str(&format!(" @{sub}"));
            }
            ListItem::new(name).style(style)
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
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut browser.state);
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
    ];

    // Add browser keys if playlist is present
    if app.browser.is_some() {
        spans.extend([
            Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled("\u{2191}\u{2193}", Style::default().fg(Color::Cyan).bold()),
            Span::styled(" Browse ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::Cyan).bold()),
            Span::styled(" Load ", Style::default().fg(Color::DarkGray)),
        ]);
    }

    spans.extend([
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Cyan).bold()),
        Span::styled("/", Style::default().fg(Color::DarkGray)),
        Span::styled("ESC", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Quit", Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
