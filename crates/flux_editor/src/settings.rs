//! Editor settings: appearance, script-editor font, syntax colors, and grid
//! defaults. Persisted to `<config>/Flux/settings.json` (same config dir as the
//! recent-projects list).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Syntax highlighting colors for the script editor, as sRGB `[r, g, b]` so they
/// serialize cleanly and bind directly to egui's `color_edit_button_srgb`.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct SyntaxTheme {
    pub text: [u8; 3],
    pub keyword: [u8; 3],
    pub string: [u8; 3],
    pub number: [u8; 3],
    pub comment: [u8; 3],
    pub global: [u8; 3],
    pub service: [u8; 3],
    pub function: [u8; 3],
}

impl Default for SyntaxTheme {
    fn default() -> Self {
        // The editor's original VS Code-dark palette.
        Self {
            text: [212, 212, 212],
            keyword: [197, 134, 192],
            string: [206, 145, 120],
            number: [181, 206, 168],
            comment: [106, 153, 85],
            global: [86, 156, 214],
            service: [78, 201, 176],
            function: [220, 220, 170],
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "yes")]
    pub theme_dark: bool,
    #[serde(default = "default_font")]
    pub font_size: f32,
    #[serde(default)]
    pub syntax: SyntaxTheme,
    #[serde(default = "default_grid")]
    pub grid_size: f32,
    #[serde(default)]
    pub grid_snap: bool,
}

fn yes() -> bool {
    true
}
fn default_font() -> f32 {
    14.0
}
fn default_grid() -> f32 {
    32.0
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme_dark: true,
            font_size: 14.0,
            syntax: SyntaxTheme::default(),
            grid_size: 32.0,
            grid_snap: false,
        }
    }
}

impl Settings {
    /// `<config>/Flux/settings.json` (matches the recent-projects location).
    fn file() -> Option<PathBuf> {
        let dir = if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("Flux")
        } else if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(xdg).join("flux")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("flux")
        } else {
            return None;
        };
        Some(dir.join("settings.json"))
    }

    pub fn load() -> Self {
        Self::file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(file) = Self::file() else { return };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(file, json);
        }
    }
}
