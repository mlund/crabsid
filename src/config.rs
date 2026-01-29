// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Mikael Lund

//! User configuration persistence.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Default color scheme index (Gruvbox Dark Hard).
const fn default_color_scheme() -> usize {
    12
}

/// User configuration stored in config file.
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Selected color scheme index
    #[serde(default = "default_color_scheme")]
    pub color_scheme: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            color_scheme: default_color_scheme(),
        }
    }
}

impl Config {
    /// Loads config from file, returning defaults if not found or invalid.
    pub fn load() -> Self {
        config_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Saves config to file (best-effort, errors ignored).
    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        let Some(parent) = path.parent() else { return };
        let _ = fs::create_dir_all(parent);
        if let Ok(content) = toml::to_string_pretty(self) {
            let _ = fs::write(&path, content);
        }
    }
}

/// Returns the config file path (~/.config/crabsid/config.toml).
fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("crabsid").join("config.toml"))
}
