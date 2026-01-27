// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! Terminal user interface for the SID player.

mod app;
mod draw;
mod input;
pub mod theme;
mod widgets;

use app::App;
use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyEventKind},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use draw::draw;
use input::handle_key;
use ratatui::DefaultTerminal;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::player::SharedPlayer;
use crate::playlist::Playlist;
use crate::sid_file::SidFile;

const TARGET_FPS: u64 = 30;

/// Configuration for the TUI.
pub struct TuiConfig<'a> {
    pub player: SharedPlayer,
    pub sid_file: &'a SidFile,
    pub song: u16,
    pub playlist: Playlist,
    pub playlist_path: PathBuf,
    pub focus_hvsc: bool,
    pub playlist_modified: bool,
    pub hvsc_url: &'a str,
    pub playtime_secs: u64,
}

/// Main entry point for the TUI.
pub fn run_tui(config: TuiConfig) -> io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;

    let terminal = ratatui::init();
    let app = App::new(config);
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

        let elapsed = frame_start.elapsed();
        let timeout = frame_duration.saturating_sub(elapsed);

        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(action) = handle_key(&mut app, key.code)
        {
            return action;
        }
    }
}
