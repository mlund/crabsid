// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! Application state and logic.

use crate::hvsc::{HvscBrowser, HvscEntry};
use crate::player::SharedPlayer;
use crate::playlist::Playlist;
use crate::sid_file::SidFile;
use ratatui::widgets::ListState;
use resid::ChipModel;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::TuiConfig;
use super::theme::{ColorScheme, DEFAULT_SCHEME, SCHEMES};
use super::widgets::{VoiceScopes, VuMeter};

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

/// Browser state for playlist navigation.
pub struct PlaylistBrowser {
    pub playlist: Playlist,
    pub state: ListState,
}

impl PlaylistBrowser {
    pub fn new(playlist: Playlist) -> Self {
        let mut state = ListState::default();
        if !playlist.is_empty() {
            state.select(Some(0));
        }
        Self { playlist, state }
    }

    pub fn selected_index(&self) -> usize {
        self.state.selected().unwrap_or(0)
    }

    pub fn select_next(&mut self) {
        let len = self.playlist.len();
        if len == 0 {
            return;
        }
        let i = self.selected_index();
        self.state.select(Some((i + 1).min(len - 1)));
    }

    pub fn select_prev(&mut self) {
        self.state
            .select(Some(self.selected_index().saturating_sub(1)));
    }
}

/// TUI application state holding the player and display data.
pub struct App<'a> {
    pub player: SharedPlayer,
    pub sid_file: &'a SidFile,
    pub current_song: u16,
    pub total_songs: u16,
    pub paused: bool,
    /// Chip models for each SID (1-3 entries)
    pub chip_models: Vec<ChipModel>,
    /// Index of currently selected SID for chip switching (cycles through)
    pub selected_sid: usize,
    pub vu_meter: VuMeter,
    pub voice_scopes: VoiceScopes,
    pub playlist_browser: PlaylistBrowser,
    pub playlist_path: PathBuf,
    pub hvsc_browser: HvscBrowser,
    pub browser_focus: BrowserFocus,
    pub current_browser_sid: Option<SidFile>,
    pub current_source: Option<String>,
    pub popup: Popup,
    pub playlist_modified: bool,
    pub color_scheme: usize,
    pub hvsc_search: Option<String>,
    pub hvsc_search_results: Vec<String>,
    pub hvsc_search_index: usize,
    pub song_elapsed: Duration,
    pub song_resumed_at: Instant,
    pub song_timeout: Duration,
    pub default_timeout: Duration,
}

impl<'a> App<'a> {
    /// Creates the application with all components.
    pub fn new(config: TuiConfig<'a>) -> Self {
        let chip_models = config
            .player
            .lock()
            .map(|p| p.chip_models().to_vec())
            .unwrap_or_else(|_| vec![ChipModel::Mos6581]);

        let sid_count = chip_models.len();

        let mut hvsc_browser = HvscBrowser::new(config.hvsc_url);
        hvsc_browser.load_stil();

        let browser_focus = if config.focus_hvsc {
            BrowserFocus::Hvsc
        } else {
            BrowserFocus::Playlist
        };

        Self {
            player: config.player,
            sid_file: config.sid_file,
            current_song: config.song,
            total_songs: config.sid_file.songs,
            paused: false,
            chip_models,
            selected_sid: 0,
            vu_meter: VuMeter::with_voice_count(sid_count * 3),
            voice_scopes: VoiceScopes::with_voice_count(sid_count * 3),
            playlist_browser: PlaylistBrowser::new(config.playlist),
            playlist_path: config.playlist_path,
            hvsc_browser,
            browser_focus,
            current_browser_sid: None,
            current_source: None,
            popup: Popup::None,
            playlist_modified: config.playlist_modified,
            color_scheme: DEFAULT_SCHEME,
            hvsc_search: None,
            hvsc_search_results: Vec::new(),
            hvsc_search_index: 0,
            song_elapsed: Duration::ZERO,
            song_resumed_at: Instant::now(),
            song_timeout: Duration::from_secs(config.playtime_secs),
            default_timeout: Duration::from_secs(config.playtime_secs),
        }
    }

    pub fn scheme(&self) -> &ColorScheme {
        &SCHEMES[self.color_scheme]
    }

    /// Returns the SID file to display metadata from.
    pub fn display_sid(&self) -> &SidFile {
        self.current_browser_sid.as_ref().unwrap_or(self.sid_file)
    }

    /// Returns total elapsed play time (excludes paused time).
    pub fn song_elapsed_total(&self) -> Duration {
        if self.paused {
            self.song_elapsed
        } else {
            self.song_elapsed + self.song_resumed_at.elapsed()
        }
    }

    /// Resets the song timer for a new song/subsong.
    fn reset_song_timer(&mut self) {
        self.song_elapsed = Duration::ZERO;
        self.song_resumed_at = Instant::now();
    }

    /// Updates song_timeout from Songlengths database, falling back to default_timeout.
    fn update_song_timeout(&mut self, md5: &str, song: u16) {
        self.song_timeout = self
            .hvsc_browser
            .song_duration(md5, song)
            .unwrap_or(self.default_timeout);
    }

    pub fn update(&mut self) {
        let playback_error = if let Ok(mut player) = self.player.lock() {
            self.vu_meter.update(&player.voice_levels());
            self.voice_scopes.update(&player.envelope_samples());
            self.paused = player.is_paused();
            self.chip_models = player.chip_models().to_vec();
            player.take_error()
        } else {
            None
        };

        // Show playback error after releasing player lock
        if let Some(err) = playback_error {
            self.show_error(format!("Playback error: {err}"));
        }

        // Auto-advance when playtime exceeded (pause if error popup is showing)
        let has_error_popup = matches!(self.popup, Popup::Error(_));
        if !self.paused && !has_error_popup && self.song_elapsed_total() >= self.song_timeout {
            self.advance_song();
        }
    }

    /// Advances to next subsong, or next playlist/HVSC entry if at last subsong.
    fn advance_song(&mut self) {
        if self.current_song < self.total_songs {
            self.current_song += 1;
            self.load_song_on_player(self.current_song);
            self.reset_song_timer();
        } else {
            // Reset timer before attempting load to prevent infinite loop if all files fail
            self.reset_song_timer();
            match self.browser_focus {
                BrowserFocus::Playlist => {
                    self.playlist_browser.select_next();
                    self.load_playlist_selected();
                }
                BrowserFocus::Hvsc => {
                    if !self.hvsc_search_results.is_empty() {
                        self.try_next_hvsc_search_result();
                    } else {
                        self.try_next_hvsc_file();
                    }
                }
            }
        }
    }

    pub fn toggle_pause(&mut self) {
        if let Ok(mut player) = self.player.lock() {
            player.toggle_pause();
            let was_paused = self.paused;
            self.paused = player.is_paused();

            if self.paused && !was_paused {
                self.song_elapsed += self.song_resumed_at.elapsed();
            } else if !self.paused && was_paused {
                self.song_resumed_at = Instant::now();
            }
        }
    }

    pub fn next_song(&mut self) {
        if self.current_song < self.total_songs {
            self.current_song += 1;
            self.load_song_on_player(self.current_song);
            self.reset_song_timer();
        }
    }

    pub fn prev_song(&mut self) {
        if self.current_song > 1 {
            self.current_song -= 1;
            self.load_song_on_player(self.current_song);
            self.reset_song_timer();
        }
    }

    pub fn goto_song(&mut self, song: u16) {
        if song >= 1 && song <= self.total_songs {
            self.current_song = song;
            self.load_song_on_player(song);
            self.reset_song_timer();
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

        let md5 = self
            .current_browser_sid
            .as_ref()
            .map(|s| &s.md5)
            .unwrap_or(&self.sid_file.md5)
            .clone();
        self.update_song_timeout(&md5, song);
    }

    /// Cycles the chip model for the currently selected SID.
    /// For multi-SID tunes, pressing 's' repeatedly cycles through all SIDs.
    pub fn switch_chip(&mut self) {
        if let Ok(mut player) = self.player.lock() {
            // Ensure selected_sid is valid for current SID count
            let sid_count = player.sid_count();
            if self.selected_sid >= sid_count {
                self.selected_sid = 0;
            }

            player.switch_chip_model(Some(self.selected_sid));
            self.chip_models = player.chip_models().to_vec();

            // Cycle to next SID for the next 's' press
            if sid_count > 1 {
                self.selected_sid = (self.selected_sid + 1) % sid_count;
            }
        }
    }

    pub fn toggle_browser_focus(&mut self) {
        self.browser_focus = match self.browser_focus {
            BrowserFocus::Playlist => BrowserFocus::Hvsc,
            BrowserFocus::Hvsc => BrowserFocus::Playlist,
        };
    }

    pub fn browser_next(&mut self) {
        match self.browser_focus {
            BrowserFocus::Playlist => self.playlist_browser.select_next(),
            BrowserFocus::Hvsc => self.hvsc_browser.select_next(),
        }
    }

    pub fn browser_prev(&mut self) {
        match self.browser_focus {
            BrowserFocus::Playlist => self.playlist_browser.select_prev(),
            BrowserFocus::Hvsc => self.hvsc_browser.select_prev(),
        }
    }

    pub fn browser_back(&mut self) {
        if self.browser_focus == BrowserFocus::Hvsc {
            self.hvsc_browser.go_up();
        }
    }

    pub fn load_selected(&mut self) {
        match self.browser_focus {
            BrowserFocus::Playlist => self.load_playlist_selected(),
            BrowserFocus::Hvsc => self.load_hvsc_selected(),
        }
    }

    pub fn load_playlist_selected(&mut self) {
        let start_idx = self.playlist_browser.selected_index();
        let len = self.playlist_browser.playlist.len();
        if len == 0 {
            return;
        }

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
            // Stop if error popup is showing
            if matches!(self.popup, Popup::Error(_)) {
                self.playlist_browser.state.select(Some(idx));
                return;
            }
        }
    }

    fn load_hvsc_selected(&mut self) {
        let Some(entry) = self.hvsc_browser.enter() else {
            return;
        };

        let source = entry.url(&self.hvsc_browser.base_url);
        match entry.load(&self.hvsc_browser.base_url) {
            Ok(sid_file) => {
                let start_song = sid_file.start_song;
                self.play_sid_file(sid_file, start_song, source);
            }
            Err(e) => {
                self.show_error(format!("Skipped: {e}"));
            }
        }
    }

    fn try_next_hvsc_file(&mut self) {
        let start = self.hvsc_browser.selected;
        let len = self.hvsc_browser.entries.len();
        let base_url = self.hvsc_browser.base_url.clone();

        for offset in 1..len {
            let idx = (start + offset) % len;
            let entry = &self.hvsc_browser.entries[idx];

            if entry.is_dir {
                continue;
            }

            self.hvsc_browser.selected = idx;
            let source = entry.url(&base_url);
            match entry.load(&base_url) {
                Ok(sid_file) => {
                    let start_song = sid_file.start_song;
                    if self.play_sid_file(sid_file, start_song, source) {
                        return;
                    }
                }
                Err(e) => self.show_error(format!("Skipped: {e}")),
            }
            // Stop if error popup is showing
            if matches!(self.popup, Popup::Error(_)) {
                return;
            }
        }
    }

    /// Attempts to play a SID file. Returns true on success, false on failure.
    fn play_sid_file(&mut self, sid_file: SidFile, song: u16, source: String) -> bool {
        if sid_file.requires_full_emulation() {
            self.show_error("Skipped: Unsupported RSID-like format".to_string());
            return false;
        }

        self.current_song = song;
        self.total_songs = sid_file.songs;

        let error = match self.player.lock() {
            Ok(mut player) => {
                let res = player.load_sid_file(&sid_file, song);
                match res {
                    Ok(_) => {
                        self.chip_models = player.chip_models().to_vec();
                        self.selected_sid = 0;
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

        self.update_song_timeout(&sid_file.md5, song);
        self.current_browser_sid = Some(sid_file);
        self.current_source = Some(source);
        self.song_elapsed = Duration::ZERO;
        self.song_resumed_at = Instant::now();
        true
    }

    pub fn add_current_to_playlist(&mut self) {
        let Some(source) = &self.current_source else {
            return;
        };
        let subsong = Some(self.current_song);
        self.playlist_browser.playlist.add(source, subsong);
        self.playlist_modified = true;
    }

    pub fn remove_from_playlist(&mut self) {
        if self.browser_focus != BrowserFocus::Playlist {
            return;
        }
        let idx = self.playlist_browser.selected_index();
        self.playlist_browser.playlist.remove(idx);

        let len = self.playlist_browser.playlist.len();
        if len > 0 && idx >= len {
            self.playlist_browser.state.select(Some(len - 1));
        }
        self.playlist_modified = true;
    }

    pub fn save_playlist(&self) {
        if let Err(e) = self.playlist_browser.playlist.save(&self.playlist_path) {
            eprintln!("Failed to save playlist: {e}");
        }
    }

    // HVSC search methods
    pub fn start_hvsc_search(&mut self) {
        if self.browser_focus == BrowserFocus::Hvsc {
            self.hvsc_search = Some(String::new());
            self.hvsc_search_results.clear();
            self.hvsc_search_index = 0;
            self.popup = Popup::HvscSearch;
        }
    }

    pub fn cancel_hvsc_search(&mut self) {
        self.hvsc_search = None;
        self.hvsc_search_results.clear();
    }

    pub fn hvsc_search_input(&mut self, ch: char) {
        if let Some(ref mut query) = self.hvsc_search {
            query.push(ch);
        }
    }

    pub fn hvsc_search_backspace(&mut self) {
        if let Some(ref mut query) = self.hvsc_search {
            query.pop();
        }
    }

    pub fn update_search_results(&mut self) {
        let query = match &self.hvsc_search {
            Some(q) if !q.is_empty() => q.clone(),
            _ => {
                self.hvsc_search_results.clear();
                return;
            }
        };

        if let Some(ref stil) = self.hvsc_browser.stil {
            self.hvsc_search_results = stil.search(&query).into_iter().map(String::from).collect();
            self.hvsc_search_results.sort();
            self.hvsc_search_results.truncate(100);
            self.hvsc_search_index = 0;
        }
    }

    pub fn hvsc_search_next(&mut self) {
        if !self.hvsc_search_results.is_empty() {
            self.hvsc_search_index = (self.hvsc_search_index + 1) % self.hvsc_search_results.len();
        }
    }

    pub fn hvsc_search_prev(&mut self) {
        if !self.hvsc_search_results.is_empty() {
            self.hvsc_search_index = self
                .hvsc_search_index
                .checked_sub(1)
                .unwrap_or(self.hvsc_search_results.len() - 1);
        }
    }

    pub fn hvsc_search_select(&mut self) {
        self.try_load_hvsc_search_result(0);
    }

    fn try_next_hvsc_search_result(&mut self) {
        self.try_load_hvsc_search_result(1);
    }

    /// Tries to load a search result starting from current index + offset.
    fn try_load_hvsc_search_result(&mut self, start_offset: usize) {
        let start = self.hvsc_search_index;
        let len = self.hvsc_search_results.len();
        if len == 0 {
            return;
        }

        for offset in start_offset..len {
            let idx = (start + offset) % len;
            let path = &self.hvsc_search_results[idx];
            let entry = HvscEntry {
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                path: path.clone(),
                is_dir: false,
            };
            let source = entry.url(&self.hvsc_browser.base_url);

            match entry.load(&self.hvsc_browser.base_url) {
                Ok(sid_file) => {
                    let start_song = sid_file.start_song;
                    if self.play_sid_file(sid_file, start_song, source) {
                        self.hvsc_search_index = idx;
                        return;
                    }
                }
                Err(e) => self.show_error(format!("Skipped: {e}")),
            }
            // Stop if error popup is showing
            if matches!(self.popup, Popup::Error(_)) {
                self.hvsc_search_index = idx;
                return;
            }
        }
    }

    // Color scheme methods
    pub fn open_color_picker(&mut self) {
        self.popup = Popup::ColorScheme;
    }

    pub fn next_color_scheme(&mut self) {
        self.color_scheme = (self.color_scheme + 1) % SCHEMES.len();
    }

    pub fn prev_color_scheme(&mut self) {
        self.color_scheme = self
            .color_scheme
            .checked_sub(1)
            .unwrap_or(SCHEMES.len() - 1);
    }

    // Popup methods
    pub fn show_help(&mut self) {
        self.popup = Popup::Help;
    }

    pub fn show_error(&mut self, msg: String) {
        self.popup = Popup::Error(msg);
        // Pause playback so user can read the error
        if let Ok(mut player) = self.player.lock()
            && !player.is_paused()
        {
            player.toggle_pause();
            self.paused = true;
            self.song_elapsed += self.song_resumed_at.elapsed();
        }
    }

    pub fn close_popup(&mut self) {
        self.popup = Popup::None;
    }

    pub fn request_quit(&mut self) -> bool {
        if self.playlist_modified {
            self.popup = Popup::SaveConfirm;
            false
        } else {
            true
        }
    }
}
