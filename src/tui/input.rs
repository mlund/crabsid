// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! Keyboard input handling.

use crossterm::event::KeyCode;
use std::io;

use super::app::{App, BrowserFocus, Popup};

pub enum KeyHandled {
    Consumed(Option<io::Result<()>>),
    PassThrough,
}

/// Processes key input, returning Some to exit the app.
pub fn handle_key(app: &mut App, key: KeyCode) -> Option<io::Result<()>> {
    // Save confirmation needs Y/N before other keys work
    if matches!(app.popup, Popup::SaveConfirm) {
        return handle_save_confirm(app, key);
    }

    match handle_popups(app, key) {
        KeyHandled::Consumed(res) => return res,
        KeyHandled::PassThrough => {}
    }

    // HVSC search results: intercept navigation keys
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
        KeyCode::Char('/') => app.start_hvsc_search(),
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
