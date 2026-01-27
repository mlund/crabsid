// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

use crate::hvsc::{HvscBrowser, HvscEntry};
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
        Bar, BarChart, BarGroup, Block, Borders, Clear, List, ListItem, ListState, Paragraph,
        canvas::{Canvas, Line as CanvasLine},
    },
};
use resid::ChipModel;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const TARGET_FPS: u64 = 30;
/// Number of samples to display in oscilloscope (downsampled from player buffer)
const SCOPE_DISPLAY_SAMPLES: usize = 256;

/// C64 palette colors.
#[allow(dead_code)]
mod c64 {
    use ratatui::style::Color;
    pub const BLACK: Color = Color::Rgb(0x00, 0x00, 0x00);
    pub const WHITE: Color = Color::Rgb(0xFF, 0xFF, 0xFF);
    pub const RED: Color = Color::Rgb(0x88, 0x00, 0x00);
    pub const CYAN: Color = Color::Rgb(0xAA, 0xFF, 0xEE);
    pub const PURPLE: Color = Color::Rgb(0xCC, 0x44, 0xCC);
    pub const GREEN: Color = Color::Rgb(0x00, 0xCC, 0x55);
    pub const BLUE: Color = Color::Rgb(0x00, 0x00, 0xAA);
    pub const YELLOW: Color = Color::Rgb(0xEE, 0xEE, 0x77);
    pub const ORANGE: Color = Color::Rgb(0xDD, 0x88, 0x55);
    pub const BROWN: Color = Color::Rgb(0x66, 0x44, 0x00);
    pub const LIGHT_RED: Color = Color::Rgb(0xFF, 0x77, 0x77);
    pub const DARK_GREY: Color = Color::Rgb(0x33, 0x33, 0x33);
    pub const GREY: Color = Color::Rgb(0x77, 0x77, 0x77);
    pub const LIGHT_GREEN: Color = Color::Rgb(0xAA, 0xFF, 0x66);
    pub const LIGHT_BLUE: Color = Color::Rgb(0x00, 0x88, 0xFF);
    pub const LIGHT_GREY: Color = Color::Rgb(0xBB, 0xBB, 0xBB);
}

#[allow(dead_code)]
mod dracula {
    use ratatui::style::Color;
    pub const BACKGROUND: Color = Color::Rgb(0x28, 0x2a, 0x36);
    pub const FOREGROUND: Color = Color::Rgb(0xf8, 0xf8, 0xf2);
    pub const COMMENT: Color = Color::Rgb(0x62, 0x72, 0xa4);
    pub const CYAN: Color = Color::Rgb(0x8b, 0xe9, 0xfd);
    pub const GREEN: Color = Color::Rgb(0x50, 0xfa, 0x7b);
    pub const ORANGE: Color = Color::Rgb(0xff, 0xb8, 0x6c);
    pub const PINK: Color = Color::Rgb(0xff, 0x79, 0xc6);
    pub const PURPLE: Color = Color::Rgb(0xbd, 0x93, 0xf9);
    pub const RED: Color = Color::Rgb(0xff, 0x55, 0x55);
    pub const YELLOW: Color = Color::Rgb(0xf1, 0xfa, 0x8c);
}

/// Complete color scheme for TUI theming.
#[derive(Clone, Copy)]
struct ColorScheme {
    name: &'static str,
    background: Color,
    voices: [Color; 3],
    accent: Color,
    title: Color,
    border_focus: Color,
    border_dim: Color,
    text_primary: Color,
    text_secondary: Color,
    highlight_bg: Color,
    highlight_fg: Color,
}

const SCHEMES: &[ColorScheme] = &[
    ColorScheme {
        name: "Dark Primary",
        background: c64::BLACK,
        voices: [c64::RED, c64::GREEN, c64::BLUE],
        accent: c64::CYAN,
        title: c64::LIGHT_BLUE,
        border_focus: c64::CYAN,
        border_dim: c64::DARK_GREY,
        text_primary: c64::LIGHT_GREY,
        text_secondary: c64::GREY,
        highlight_bg: c64::BLUE,
        highlight_fg: c64::CYAN,
    },
    ColorScheme {
        name: "Warm",
        background: c64::BLACK,
        voices: [c64::ORANGE, c64::YELLOW, c64::BROWN],
        accent: c64::YELLOW,
        title: c64::ORANGE,
        border_focus: c64::YELLOW,
        border_dim: c64::BROWN,
        text_primary: c64::LIGHT_GREY,
        text_secondary: c64::ORANGE,
        highlight_bg: c64::BROWN,
        highlight_fg: c64::YELLOW,
    },
    ColorScheme {
        name: "Cool",
        background: c64::BLACK,
        voices: [c64::PURPLE, c64::CYAN, c64::LIGHT_BLUE],
        accent: c64::CYAN,
        title: c64::PURPLE,
        border_focus: c64::CYAN,
        border_dim: c64::BLUE,
        text_primary: c64::LIGHT_GREY,
        text_secondary: c64::LIGHT_BLUE,
        highlight_bg: c64::BLUE,
        highlight_fg: c64::CYAN,
    },
    ColorScheme {
        name: "Monochrome",
        background: c64::BLACK,
        voices: [c64::LIGHT_GREY, c64::GREY, c64::WHITE],
        accent: c64::GREEN,
        title: c64::GREEN,
        border_focus: c64::GREEN,
        border_dim: c64::DARK_GREY,
        text_primary: c64::LIGHT_GREY,
        text_secondary: c64::GREY,
        highlight_bg: c64::DARK_GREY,
        highlight_fg: c64::GREEN,
    },
    ColorScheme {
        name: "Neon",
        background: c64::BLACK,
        voices: [c64::LIGHT_RED, c64::LIGHT_GREEN, c64::LIGHT_BLUE],
        accent: c64::CYAN,
        title: c64::YELLOW,
        border_focus: c64::PURPLE,
        border_dim: c64::DARK_GREY,
        text_primary: c64::WHITE,
        text_secondary: c64::LIGHT_GREY,
        highlight_bg: c64::PURPLE,
        highlight_fg: c64::CYAN,
    },
    ColorScheme {
        name: "C64",
        background: c64::BLUE,
        voices: [c64::LIGHT_BLUE, c64::CYAN, c64::WHITE],
        accent: c64::LIGHT_BLUE,
        title: c64::CYAN,
        border_focus: c64::LIGHT_BLUE,
        border_dim: c64::LIGHT_BLUE,
        text_primary: c64::LIGHT_BLUE,
        text_secondary: c64::CYAN,
        highlight_bg: c64::DARK_GREY,
        highlight_fg: c64::WHITE,
    },
    ColorScheme {
        name: "Frost",
        background: c64::BLUE,
        voices: [c64::WHITE, c64::LIGHT_GREY, c64::CYAN],
        accent: c64::WHITE,
        title: c64::WHITE,
        border_focus: c64::LIGHT_GREY,
        border_dim: c64::CYAN,
        text_primary: c64::WHITE,
        text_secondary: c64::LIGHT_GREY,
        highlight_bg: c64::DARK_GREY,
        highlight_fg: c64::CYAN,
    },
    ColorScheme {
        name: "VIC-20",
        background: c64::CYAN,
        voices: [c64::BLUE, c64::PURPLE, c64::RED],
        accent: c64::BLUE,
        title: c64::BLUE,
        border_focus: c64::BLUE,
        border_dim: c64::PURPLE,
        text_primary: c64::BLUE,
        text_secondary: c64::PURPLE,
        highlight_bg: c64::BLUE,
        highlight_fg: c64::CYAN,
    },
    ColorScheme {
        name: "C128",
        background: c64::DARK_GREY,
        voices: [c64::LIGHT_GREEN, c64::GREEN, c64::CYAN],
        accent: c64::LIGHT_GREEN,
        title: c64::LIGHT_GREEN,
        border_focus: c64::LIGHT_GREEN,
        border_dim: c64::GREEN,
        text_primary: c64::LIGHT_GREEN,
        text_secondary: c64::GREEN,
        highlight_bg: c64::GREEN,
        highlight_fg: c64::DARK_GREY,
    },
    ColorScheme {
        name: "PET",
        background: c64::BLACK,
        voices: [c64::GREEN, c64::GREEN, c64::GREEN],
        accent: c64::GREEN,
        title: c64::GREEN,
        border_focus: c64::GREEN,
        border_dim: c64::DARK_GREY,
        text_primary: c64::GREEN,
        text_secondary: c64::GREEN,
        highlight_bg: c64::GREEN,
        highlight_fg: c64::BLACK,
    },
    ColorScheme {
        name: "Dracula",
        background: dracula::BACKGROUND,
        voices: [dracula::PINK, dracula::CYAN, dracula::GREEN],
        accent: dracula::PURPLE,
        title: dracula::PINK,
        border_focus: dracula::PURPLE,
        border_dim: dracula::COMMENT,
        text_primary: dracula::FOREGROUND,
        text_secondary: dracula::COMMENT,
        highlight_bg: dracula::COMMENT,
        highlight_fg: dracula::CYAN,
    },
];

const DEFAULT_SCHEME: usize = 10; // Dracula

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

/// Popup dialog state.
#[derive(Debug, Clone)]
pub enum Popup {
    None,
    Help,
    Error(String),
    SaveConfirm,
    HvscSearch,
    ColorScheme,
}

enum KeyHandled {
    Consumed(Option<io::Result<()>>),
    PassThrough,
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
        self.state
            .select(Some(self.selected_index().saturating_sub(1)));
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
    playlist_browser: PlaylistBrowser,
    /// Path to save playlist
    playlist_path: PathBuf,
    /// HVSC browser (lower left)
    hvsc_browser: HvscBrowser,
    /// Which browser panel has focus
    browser_focus: BrowserFocus,
    /// Currently loaded SID from browsers (owned for URL loads)
    current_browser_sid: Option<SidFile>,
    /// Source path/URL of the currently playing SID
    current_source: Option<String>,
    /// Current popup dialog
    popup: Popup,
    /// Whether playlist has unsaved changes
    playlist_modified: bool,
    /// Current voice color scheme index
    color_scheme: usize,
    /// HVSC search query (None = not searching)
    hvsc_search: Option<String>,
    /// HVSC search results (paths from STIL)
    hvsc_search_results: Vec<String>,
    /// Current index in search results
    hvsc_search_index: usize,
}

impl<'a> App<'a> {
    /// Creates the application with all components.
    pub fn new(
        player: SharedPlayer,
        sid_file: &'a SidFile,
        song: u16,
        playlist: Playlist,
        playlist_path: PathBuf,
        focus_hvsc: bool,
        playlist_modified: bool,
    ) -> Self {
        let chip_model = player
            .lock()
            .map(|p| p.chip_model())
            .unwrap_or(ChipModel::Mos6581);

        let mut hvsc_browser = HvscBrowser::new();
        hvsc_browser.load_stil();

        let browser_focus = if focus_hvsc {
            BrowserFocus::Hvsc
        } else {
            BrowserFocus::Playlist
        };

        Self {
            player,
            sid_file,
            current_song: song,
            total_songs: sid_file.songs,
            paused: false,
            chip_model,
            vu_meter: VuMeter::new(),
            voice_scopes: VoiceScopes::new(),
            playlist_browser: PlaylistBrowser::new(playlist),
            playlist_path,
            hvsc_browser,
            browser_focus,
            current_browser_sid: None,
            current_source: None,
            popup: Popup::None,
            playlist_modified,
            color_scheme: DEFAULT_SCHEME,
            hvsc_search: None,
            hvsc_search_results: Vec::new(),
            hvsc_search_index: 0,
        }
    }

    fn start_hvsc_search(&mut self) {
        if self.browser_focus == BrowserFocus::Hvsc {
            self.hvsc_search = Some(String::new());
            self.hvsc_search_results.clear();
            self.hvsc_search_index = 0;
            self.popup = Popup::HvscSearch;
        }
    }

    fn cancel_hvsc_search(&mut self) {
        self.hvsc_search = None;
        self.hvsc_search_results.clear();
    }

    fn hvsc_search_input(&mut self, ch: char) {
        if let Some(ref mut query) = self.hvsc_search {
            query.push(ch);
        }
    }

    fn hvsc_search_backspace(&mut self) {
        if let Some(ref mut query) = self.hvsc_search {
            query.pop();
        }
    }

    fn update_search_results(&mut self) {
        let query = match &self.hvsc_search {
            Some(q) if !q.is_empty() => q.clone(),
            _ => {
                self.hvsc_search_results.clear();
                return;
            }
        };

        // Search STIL database for matching paths
        if let Some(ref stil) = self.hvsc_browser.stil {
            self.hvsc_search_results = stil.search(&query).into_iter().map(String::from).collect();
            self.hvsc_search_results.sort();
            self.hvsc_search_results.truncate(100); // Limit results
            self.hvsc_search_index = 0;
        }
    }

    fn hvsc_search_next(&mut self) {
        if !self.hvsc_search_results.is_empty() {
            self.hvsc_search_index = (self.hvsc_search_index + 1) % self.hvsc_search_results.len();
        }
    }

    fn hvsc_search_prev(&mut self) {
        if !self.hvsc_search_results.is_empty() {
            self.hvsc_search_index = self
                .hvsc_search_index
                .checked_sub(1)
                .unwrap_or(self.hvsc_search_results.len() - 1);
        }
    }

    fn hvsc_search_select(&mut self) {
        let start = self.hvsc_search_index;
        let len = self.hvsc_search_results.len();
        if len == 0 {
            return;
        }

        // Try results starting from selected, skipping failures
        for offset in 0..len {
            let idx = (start + offset) % len;
            let path = &self.hvsc_search_results[idx];
            let entry = HvscEntry {
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                path: path.clone(),
                is_dir: false,
            };
            let source = entry.url();

            match entry.load() {
                Ok(sid_file) => {
                    let start_song = sid_file.start_song;
                    if self.play_sid_file(sid_file, start_song, source) {
                        self.hvsc_search_index = idx;
                        return;
                    }
                }
                Err(e) => self.show_error(format!("Skipped: {e}")),
            }
        }
    }

    fn open_color_picker(&mut self) {
        self.popup = Popup::ColorScheme;
    }

    fn next_color_scheme(&mut self) {
        self.color_scheme = (self.color_scheme + 1) % SCHEMES.len();
    }

    fn prev_color_scheme(&mut self) {
        self.color_scheme = self
            .color_scheme
            .checked_sub(1)
            .unwrap_or(SCHEMES.len() - 1);
    }

    fn scheme(&self) -> &ColorScheme {
        &SCHEMES[self.color_scheme]
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
            self.load_song_on_player(self.current_song);
        }
    }

    fn prev_song(&mut self) {
        if self.current_song > 1 {
            self.current_song -= 1;
            self.load_song_on_player(self.current_song);
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
            BrowserFocus::Playlist => BrowserFocus::Hvsc,
            BrowserFocus::Hvsc => BrowserFocus::Playlist,
        };
    }

    fn browser_next(&mut self) {
        match self.browser_focus {
            BrowserFocus::Playlist => self.playlist_browser.select_next(),
            BrowserFocus::Hvsc => self.hvsc_browser.select_next(),
        }
    }

    fn browser_prev(&mut self) {
        match self.browser_focus {
            BrowserFocus::Playlist => self.playlist_browser.select_prev(),
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
        let start_idx = self.playlist_browser.selected_index();
        let len = self.playlist_browser.playlist.len();
        if len == 0 {
            return;
        }

        // Try entries starting from selected, wrapping around once
        for offset in 0..len {
            let idx = (start_idx + offset) % len;
            let entry = &self.playlist_browser.playlist.entries[idx];
            let source = entry.source.clone();
            let subsong = entry.subsong;

            match entry.load() {
                Ok(sid_file) => {
                    let song = subsong.unwrap_or(sid_file.start_song);
                    if self.play_sid_file(sid_file, song, source) {
                        self.playlist_browser.state.select(Some(idx));
                        return;
                    }
                }
                Err(e) => self.show_error(format!("Skipped: {e}")),
            }
        }
    }

    fn load_hvsc_selected(&mut self) {
        let Some(entry) = self.hvsc_browser.enter() else {
            return;
        };

        let source = entry.url();
        match entry.load() {
            Ok(sid_file) => {
                let start_song = sid_file.start_song;
                if !self.play_sid_file(sid_file, start_song, source) {
                    self.try_next_hvsc_file();
                }
            }
            Err(e) => {
                self.show_error(format!("Skipped: {e}"));
                self.try_next_hvsc_file();
            }
        }
    }

    /// Tries to play the next HVSC file, skipping directories and failures.
    fn try_next_hvsc_file(&mut self) {
        let start = self.hvsc_browser.selected;
        let len = self.hvsc_browser.entries.len();

        for offset in 1..len {
            let idx = (start + offset) % len;
            let entry = &self.hvsc_browser.entries[idx];

            if entry.is_dir {
                continue;
            }

            self.hvsc_browser.selected = idx;
            let source = entry.url();
            match entry.load() {
                Ok(sid_file) => {
                    let start_song = sid_file.start_song;
                    if self.play_sid_file(sid_file, start_song, source) {
                        return;
                    }
                }
                Err(e) => self.show_error(format!("Skipped: {e}")),
            }
        }
    }

    /// Adds the currently playing song to the playlist.
    fn add_current_to_playlist(&mut self) {
        let Some(source) = &self.current_source else {
            return;
        };

        // Include current subsong
        let subsong = Some(self.current_song);

        self.playlist_browser.playlist.add(source, subsong);
        self.playlist_modified = true;
    }

    /// Removes selected entry from playlist.
    fn remove_from_playlist(&mut self) {
        if self.browser_focus != BrowserFocus::Playlist {
            return;
        }
        let idx = self.playlist_browser.selected_index();
        self.playlist_browser.playlist.remove(idx);

        // Adjust selection if needed
        let len = self.playlist_browser.playlist.len();
        if len > 0 && idx >= len {
            self.playlist_browser.state.select(Some(len - 1));
        }
        self.playlist_modified = true;
    }

    fn save_playlist(&self) {
        if let Err(e) = self.playlist_browser.playlist.save(&self.playlist_path) {
            eprintln!("Failed to save playlist: {e}");
        }
    }

    /// Attempts to play a SID file. Returns true on success, false on failure.
    fn play_sid_file(&mut self, sid_file: SidFile, song: u16, source: String) -> bool {
        // Check before attempting emulation
        if sid_file.requires_full_emulation() {
            self.show_error("Skipped: Unsupported RSID-like format".to_string());
            return false;
        }

        self.current_song = song;
        self.total_songs = sid_file.songs;

        let error = match self.player.lock() {
            Ok(mut player) => {
                let res = player.load_sid_file(&sid_file, song);
                let chip = player.chip_model();
                match res {
                    Ok(_) => {
                        self.chip_model = chip;
                        None
                    }
                    Err(e) => Some(format!("Skipped: {e}")),
                }
            }
            Err(_) => Some("Skipped: player lock poisoned".to_string()),
        };

        if let Some(msg) = error {
            self.show_error(msg);
            return false;
        }

        self.current_browser_sid = Some(sid_file);
        self.current_source = Some(source);
        true
    }

    /// Jumps to a specific subsong (1-indexed).
    fn goto_song(&mut self, song: u16) {
        if song >= 1 && song <= self.total_songs {
            self.current_song = song;
            self.load_song_on_player(song);
        }
    }

    fn load_song_on_player(&mut self, song: u16) {
        let error = match self.player.lock() {
            Ok(mut player) => player
                .load_song(song)
                .err()
                .map(|e| format!("Init error: {e}")),
            Err(_) => Some("Init error: player lock poisoned".to_string()),
        };
        if let Some(msg) = error {
            self.show_error(msg);
        }
    }

    fn show_help(&mut self) {
        self.popup = Popup::Help;
    }

    fn show_error(&mut self, msg: String) {
        self.popup = Popup::Error(msg);
    }

    fn close_popup(&mut self) {
        self.popup = Popup::None;
    }

    /// Shows save confirmation if playlist modified, returns true to quit immediately.
    fn request_quit(&mut self) -> bool {
        if self.playlist_modified {
            self.popup = Popup::SaveConfirm;
            false
        } else {
            true
        }
    }

    /// Returns the SID file to display metadata from.
    fn display_sid(&self) -> &SidFile {
        self.current_browser_sid.as_ref().unwrap_or(self.sid_file)
    }
}

/// Main entry point for the TUI.
pub fn run_tui(
    player: SharedPlayer,
    sid_file: &SidFile,
    song: u16,
    playlist: Playlist,
    playlist_path: PathBuf,
    focus_hvsc: bool,
    playlist_modified: bool,
) -> io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;

    let terminal = ratatui::init();
    let app = App::new(
        player,
        sid_file,
        song,
        playlist,
        playlist_path,
        focus_hvsc,
        playlist_modified,
    );
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

/// Processes key input, returning Some to exit the app.
fn handle_key(app: &mut App, key: KeyCode) -> Option<io::Result<()>> {
    // Save confirmation needs Y/N before other keys work
    if matches!(app.popup, Popup::SaveConfirm) {
        return handle_save_confirm(app, key);
    }

    match handle_popups(app, key) {
        KeyHandled::Consumed(res) => return res,
        KeyHandled::PassThrough => {}
    }

    // HVSC search results: intercept navigation keys, otherwise continue
    if app.hvsc_search.is_some()
        && app.browser_focus == BrowserFocus::Hvsc
        && handle_hvsc_search_results(app, key)
    {
        return None;
    }

    match key {
        KeyCode::Char('q') if app.request_quit() => return Some(Ok(())),
        KeyCode::Esc => app.close_popup(),
        KeyCode::Char(' ') => app.toggle_pause(),
        KeyCode::Char('s') => app.switch_chip(),
        KeyCode::Char('c') => app.open_color_picker(),
        KeyCode::Char('h' | '?') => app.show_help(),
        KeyCode::Tab => app.toggle_browser_focus(),
        KeyCode::Char('/') => app.start_hvsc_search(),

        KeyCode::Char(c @ '1'..='9') => app.goto_song(c.to_digit(10).unwrap() as u16),
        KeyCode::Char('+' | 'n') => app.next_song(),
        KeyCode::Char('-' | 'p') => app.prev_song(),

        KeyCode::Up | KeyCode::Char('k') => app.browser_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.browser_next(),
        KeyCode::Left => app.browser_back(),
        KeyCode::Enter => app.load_selected(),
        KeyCode::Char('a') => app.add_current_to_playlist(),
        KeyCode::Backspace => handle_backspace(app),

        _ => {}
    }
    None
}

fn handle_hvsc_search_popup(app: &mut App, key: KeyCode) -> Option<io::Result<()>> {
    match key {
        KeyCode::Esc => {
            app.popup = Popup::None;
            app.cancel_hvsc_search();
        }
        KeyCode::Enter => {
            app.popup = Popup::None;
            app.update_search_results();
        }
        KeyCode::Backspace => app.hvsc_search_backspace(),
        KeyCode::Char(ch) => app.hvsc_search_input(ch),
        _ => {}
    }
    None
}

fn handle_hvsc_search_results(app: &mut App, key: KeyCode) -> bool {
    match key {
        KeyCode::Esc => app.cancel_hvsc_search(),
        KeyCode::Enter => app.hvsc_search_select(),
        KeyCode::Up => app.hvsc_search_prev(),
        KeyCode::Down => app.hvsc_search_next(),
        KeyCode::Char('/') => app.start_hvsc_search(), // reopen popup to edit query
        _ => return false,
    }
    true
}

fn handle_color_scheme_popup(app: &mut App, key: KeyCode) -> Option<io::Result<()>> {
    match key {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('c') => app.popup = Popup::None,
        KeyCode::Up | KeyCode::Char('k') => app.prev_color_scheme(),
        KeyCode::Down | KeyCode::Char('j') => app.next_color_scheme(),
        _ => {}
    }
    None
}

fn handle_popups(app: &mut App, key: KeyCode) -> KeyHandled {
    match app.popup {
        Popup::HvscSearch => KeyHandled::Consumed(handle_hvsc_search_popup(app, key)),
        Popup::SaveConfirm => KeyHandled::Consumed(handle_save_confirm(app, key)),
        Popup::Help | Popup::Error(_) => {
            app.close_popup();
            KeyHandled::Consumed(None)
        }
        Popup::ColorScheme => KeyHandled::Consumed(handle_color_scheme_popup(app, key)),
        Popup::None => KeyHandled::PassThrough,
    }
}

fn handle_save_confirm(app: &mut App, key: KeyCode) -> Option<io::Result<()>> {
    match key {
        KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
            app.save_playlist();
            Some(Ok(()))
        }
        KeyCode::Char('n' | 'N') => Some(Ok(())),
        _ => {
            app.close_popup();
            None
        }
    }
}

fn handle_backspace(app: &mut App) {
    if app.browser_focus == BrowserFocus::Playlist {
        app.remove_from_playlist();
    } else {
        app.browser_back();
    }
}

fn draw(frame: &mut Frame, app: &mut App) {
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

    // Show search results if searching, otherwise show directory
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
            // Show just filename
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

    let status = if app.paused {
        Span::styled("  [PAUSED]", Style::default().fg(c64::YELLOW).bold())
    } else {
        Span::styled("  [PLAYING]", Style::default().fg(c64::GREEN))
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
            Span::styled(chip, Style::default().fg(c64::PURPLE)),
            status,
        ]),
    ]
}

/// Returns the CrabSid logo with fixed C64 rainbow colors.
fn logo_lines() -> Vec<Line<'static>> {
    let crab = Style::default().fg(c64::ORANGE);
    // Rainbow: red -> orange -> yellow -> green -> cyan -> blue -> purple
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

    // Center the chart horizontally
    let chart_width = 3 * 8 + 2 * 3; // 3 bars + 2 gaps
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

    let (title, content, small) = match &app.popup {
        Popup::None | Popup::ColorScheme => return,
        Popup::Help => (" Help ", help_text(), true),
        Popup::Error(msg) => (" Error ", vec![Line::from(msg.as_str())], false),
        Popup::SaveConfirm => (
            " Save Playlist? ",
            vec![
                Line::from(""),
                Line::from("  Save changes before quitting?"),
                Line::from(""),
                Line::from(vec![
                    Span::raw("    "),
                    Span::styled("Y", Style::default().fg(Color::Green).bold()),
                    Span::raw("/Enter = Save    "),
                    Span::styled("N", Style::default().fg(Color::Red).bold()),
                    Span::raw(" = Discard"),
                ]),
            ],
            true,
        ),
        Popup::HvscSearch => {
            let query = app.hvsc_search.as_deref().unwrap_or("");
            let line = Line::from(vec![
                Span::styled(" > ", Style::default().fg(Color::Cyan)),
                Span::raw(query),
                Span::styled("_", Style::default().fg(Color::Cyan)),
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
        .title_style(Style::default().fg(Color::Yellow).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let para = Paragraph::new(content).block(block);
    frame.render_widget(para, area);
}

fn help_text() -> Vec<Line<'static>> {
    let key = Style::default().fg(Color::Cyan);
    let hdr = Style::default().fg(Color::Yellow).bold();
    let dim = Style::default().fg(Color::DarkGray);

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
