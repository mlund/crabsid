// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! Color schemes and palettes for TUI theming.

use ratatui::style::Color;

/// C64 palette colors.
#[allow(dead_code)]
pub mod c64 {
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

/// Dracula theme colors.
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

/// Monokai theme colors.
#[allow(dead_code)]
mod monokai {
    use ratatui::style::Color;
    pub const BACKGROUND: Color = Color::Rgb(0x27, 0x28, 0x22);
    pub const FOREGROUND: Color = Color::Rgb(0xf8, 0xf8, 0xf2);
    pub const COMMENT: Color = Color::Rgb(0x75, 0x71, 0x5e);
    pub const RED: Color = Color::Rgb(0xf9, 0x26, 0x72);
    pub const ORANGE: Color = Color::Rgb(0xfd, 0x97, 0x1f);
    pub const YELLOW: Color = Color::Rgb(0xe6, 0xdb, 0x74);
    pub const GREEN: Color = Color::Rgb(0xa6, 0xe2, 0x2e);
    pub const CYAN: Color = Color::Rgb(0x66, 0xd9, 0xef);
    pub const PURPLE: Color = Color::Rgb(0xae, 0x81, 0xff);
}

/// Gruvbox dark hard theme colors.
#[allow(dead_code)]
mod gruvbox {
    use ratatui::style::Color;
    pub const BG: Color = Color::Rgb(0x1d, 0x20, 0x21);
    pub const FG: Color = Color::Rgb(0xeb, 0xdb, 0xb2);
    pub const GRAY: Color = Color::Rgb(0x92, 0x83, 0x74);
    pub const RED: Color = Color::Rgb(0xfb, 0x49, 0x34);
    pub const GREEN: Color = Color::Rgb(0xb8, 0xbb, 0x26);
    pub const YELLOW: Color = Color::Rgb(0xfa, 0xbd, 0x2f);
    pub const BLUE: Color = Color::Rgb(0x83, 0xa5, 0x98);
    pub const PURPLE: Color = Color::Rgb(0xd3, 0x86, 0x9b);
    pub const AQUA: Color = Color::Rgb(0x8e, 0xc0, 0x7c);
    pub const ORANGE: Color = Color::Rgb(0xfe, 0x80, 0x19);
}

/// Complete color scheme for TUI theming.
/// Voice colors support up to 9 voices (3 SIDs × 3 voices).
#[derive(Clone, Copy)]
pub struct ColorScheme {
    pub name: &'static str,
    pub background: Color,
    /// Colors for up to 9 voices (3 per SID chip)
    pub voices: [Color; 9],
    pub accent: Color,
    pub title: Color,
    pub border_focus: Color,
    pub border_dim: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub highlight_bg: Color,
    pub highlight_fg: Color,
}

pub const SCHEMES: &[ColorScheme] = &[
    ColorScheme {
        name: "Dark Primary",
        background: c64::BLACK,
        voices: [
            c64::RED,
            c64::GREEN,
            c64::BLUE, // SID 1
            c64::LIGHT_RED,
            c64::LIGHT_GREEN,
            c64::LIGHT_BLUE, // SID 2
            c64::ORANGE,
            c64::CYAN,
            c64::PURPLE, // SID 3
        ],
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
        voices: [
            c64::ORANGE,
            c64::YELLOW,
            c64::BROWN,
            c64::LIGHT_RED,
            c64::LIGHT_GREEN,
            c64::CYAN,
            c64::PURPLE,
            c64::LIGHT_BLUE,
            c64::WHITE,
        ],
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
        voices: [
            c64::PURPLE,
            c64::CYAN,
            c64::LIGHT_BLUE,
            c64::LIGHT_GREEN,
            c64::LIGHT_RED,
            c64::YELLOW,
            c64::WHITE,
            c64::ORANGE,
            c64::GREEN,
        ],
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
        voices: [
            c64::LIGHT_GREY,
            c64::GREY,
            c64::WHITE,
            c64::LIGHT_GREY,
            c64::GREY,
            c64::WHITE,
            c64::LIGHT_GREY,
            c64::GREY,
            c64::WHITE,
        ],
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
        voices: [
            c64::LIGHT_RED,
            c64::LIGHT_GREEN,
            c64::LIGHT_BLUE,
            c64::YELLOW,
            c64::CYAN,
            c64::PURPLE,
            c64::ORANGE,
            c64::WHITE,
            c64::GREEN,
        ],
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
        voices: [
            c64::LIGHT_BLUE,
            c64::CYAN,
            c64::WHITE,
            c64::LIGHT_GREEN,
            c64::YELLOW,
            c64::ORANGE,
            c64::LIGHT_RED,
            c64::PURPLE,
            c64::GREEN,
        ],
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
        voices: [
            c64::WHITE,
            c64::LIGHT_GREY,
            c64::CYAN,
            c64::LIGHT_BLUE,
            c64::LIGHT_GREEN,
            c64::YELLOW,
            c64::PURPLE,
            c64::ORANGE,
            c64::LIGHT_RED,
        ],
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
        voices: [
            c64::BLUE,
            c64::PURPLE,
            c64::RED,
            c64::GREEN,
            c64::YELLOW,
            c64::ORANGE,
            c64::LIGHT_BLUE,
            c64::WHITE,
            c64::LIGHT_RED,
        ],
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
        voices: [
            c64::LIGHT_GREEN,
            c64::GREEN,
            c64::CYAN,
            c64::LIGHT_BLUE,
            c64::YELLOW,
            c64::ORANGE,
            c64::LIGHT_RED,
            c64::PURPLE,
            c64::WHITE,
        ],
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
        voices: [
            c64::GREEN,
            c64::GREEN,
            c64::GREEN,
            c64::GREEN,
            c64::GREEN,
            c64::GREEN,
            c64::GREEN,
            c64::GREEN,
            c64::GREEN,
        ],
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
        voices: [
            dracula::PINK,
            dracula::CYAN,
            dracula::GREEN,
            dracula::YELLOW,
            dracula::ORANGE,
            dracula::PURPLE,
            dracula::RED,
            dracula::FOREGROUND,
            dracula::COMMENT,
        ],
        accent: dracula::PURPLE,
        title: dracula::PINK,
        border_focus: dracula::PURPLE,
        border_dim: dracula::COMMENT,
        text_primary: dracula::FOREGROUND,
        text_secondary: dracula::COMMENT,
        highlight_bg: dracula::COMMENT,
        highlight_fg: dracula::CYAN,
    },
    ColorScheme {
        name: "Monokai",
        background: monokai::BACKGROUND,
        voices: [
            monokai::RED,
            monokai::GREEN,
            monokai::CYAN,
            monokai::YELLOW,
            monokai::ORANGE,
            monokai::PURPLE,
            monokai::FOREGROUND,
            monokai::COMMENT,
            monokai::RED,
        ],
        accent: monokai::CYAN,
        title: monokai::RED,
        border_focus: monokai::CYAN,
        border_dim: monokai::COMMENT,
        text_primary: monokai::FOREGROUND,
        text_secondary: monokai::COMMENT,
        highlight_bg: monokai::COMMENT,
        highlight_fg: monokai::CYAN,
    },
    ColorScheme {
        name: "Gruvbox Dark Hard",
        background: gruvbox::BG,
        voices: [
            gruvbox::RED,
            gruvbox::GREEN,
            gruvbox::BLUE,
            gruvbox::YELLOW,
            gruvbox::ORANGE,
            gruvbox::PURPLE,
            gruvbox::AQUA,
            gruvbox::FG,
            gruvbox::GRAY,
        ],
        accent: gruvbox::AQUA,
        title: gruvbox::YELLOW,
        border_focus: gruvbox::AQUA,
        border_dim: gruvbox::GRAY,
        text_primary: gruvbox::FG,
        text_secondary: gruvbox::GRAY,
        highlight_bg: gruvbox::GRAY,
        highlight_fg: gruvbox::YELLOW,
    },
];
