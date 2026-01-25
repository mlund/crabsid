// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use crate::player::SharedPlayer;
use crate::sid_file::SidFile;
use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Bar, BarChart, BarGroup, Block, Borders, Paragraph,
        canvas::{Canvas, Line as CanvasLine},
    },
};
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
            let target = env as f32 / 255.0;

            // Fast attack, slow decay for classic VU behavior
            self.levels[i] = if target > self.levels[i] {
                self.levels[i] + (target - self.levels[i]) * ATTACK_RATE
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

/// Oscilloscope waveform buffer
pub struct Oscilloscope {
    samples: Vec<f32>,
}

impl Oscilloscope {
    pub fn new() -> Self {
        Self {
            samples: vec![0.0; SCOPE_DISPLAY_SAMPLES],
        }
    }

    /// Downsample from player buffer to display resolution
    pub fn update(&mut self, raw_samples: &[f32]) {
        if raw_samples.is_empty() {
            return;
        }
        let step = raw_samples.len() / SCOPE_DISPLAY_SAMPLES;
        if step == 0 {
            return;
        }
        for (i, sample) in self.samples.iter_mut().enumerate() {
            *sample = raw_samples.get(i * step).copied().unwrap_or(0.0);
        }
    }
}

pub struct App<'a> {
    player: SharedPlayer,
    sid_file: &'a SidFile,
    current_song: u16,
    total_songs: u16,
    paused: bool,
    vu_meter: VuMeter,
    oscilloscope: Oscilloscope,
}

impl<'a> App<'a> {
    pub fn new(player: SharedPlayer, sid_file: &'a SidFile, song: u16) -> Self {
        Self {
            player,
            sid_file,
            current_song: song,
            total_songs: sid_file.songs,
            paused: false,
            vu_meter: VuMeter::new(),
            oscilloscope: Oscilloscope::new(),
        }
    }

    fn update(&mut self) {
        if let Ok(player) = self.player.lock() {
            self.vu_meter.update(player.voice_levels());
            self.oscilloscope.update(&player.scope_samples());
            self.paused = player.is_paused();
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
}

pub fn run(player: SharedPlayer, sid_file: &SidFile, song: u16) -> io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;

    let terminal = ratatui::init();
    let result = run_app(terminal, App::new(player, sid_file, song));

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_app(mut terminal: DefaultTerminal, mut app: App) -> io::Result<()> {
    let frame_duration = Duration::from_millis(1000 / TARGET_FPS);

    loop {
        let frame_start = Instant::now();

        app.update();
        terminal.draw(|frame| draw(frame, &app))?;

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
                _ => {}
            }
        }
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let [header_area, main_area, footer_area] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Min(10),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // Split main area: VU meters left, oscilloscope right
    let [vu_area, scope_area] =
        Layout::horizontal([Constraint::Length(40), Constraint::Min(30)]).areas(main_area);

    draw_header(frame, header_area, app);
    draw_vu_meters(frame, vu_area, app);
    draw_oscilloscope(frame, scope_area, app);
    draw_footer(frame, footer_area, app);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" SID Player ")
        .title_style(Style::default().fg(Color::Cyan).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let info = vec![
        Line::from(vec![
            Span::styled("Title:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.sid_file.name, Style::default().fg(Color::White).bold()),
        ]),
        Line::from(vec![
            Span::styled("Author:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.sid_file.author, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Released: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&app.sid_file.released, Style::default().fg(Color::Gray)),
        ]),
        Line::from(vec![
            Span::styled("Song:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} / {}", app.current_song, app.sid_file.songs),
                Style::default().fg(Color::Cyan),
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

fn draw_oscilloscope(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Oscilloscope ")
        .title_style(Style::default().fg(Color::Cyan))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let samples = &app.oscilloscope.samples;
    let width = inner.width as f64;
    let x_scale = width / samples.len() as f64;

    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([0.0, width])
        .y_bounds([-1.0, 1.0])
        .paint(|ctx| {
            // Draw center line
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: 0.0,
                x2: width,
                y2: 0.0,
                color: Color::DarkGray,
            });

            // Draw waveform as connected line segments
            for i in 0..samples.len().saturating_sub(1) {
                let x1 = i as f64 * x_scale;
                let x2 = (i + 1) as f64 * x_scale;
                let y1 = samples[i] as f64;
                let y2 = samples[i + 1] as f64;

                ctx.draw(&CanvasLine {
                    x1,
                    y1,
                    x2,
                    y2,
                    color: Color::Green,
                });
            }
        });

    frame.render_widget(canvas, inner);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let pause_text = if app.paused { "Play" } else { "Pause" };

    let help = Line::from(vec![
        Span::styled(" SPACE", Style::default().fg(Color::Cyan).bold()),
        Span::styled(
            format!(" {} ", pause_text),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2190}/p", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Prev ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2192}/n", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Next ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled("q", Style::default().fg(Color::Cyan).bold()),
        Span::styled("/", Style::default().fg(Color::DarkGray)),
        Span::styled("ESC", Style::default().fg(Color::Cyan).bold()),
        Span::styled(" Quit", Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(help), area);
}
