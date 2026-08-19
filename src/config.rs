//! User configuration — the tool adapts to the writer.
//! `~/.config/perfectstar2k/config.toml` (or the platform equivalent).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::rtf::ManuscriptFont;
use crate::theme::Theme;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "wp-blue", "wordstar", or "terminal".
    pub theme: String,
    /// How long a prefix key waits before its menu appears.
    pub menu_delay_ms: u64,
    /// Seconds of idle time before a dirty buffer autosaves; 0 disables.
    pub autosave_secs: u64,
    /// Number of timestamped rolling backups retained per document; 0 disables
    /// new rolling backups without deleting copies already on disk.
    pub backup_depth: usize,
    /// Number of *automatic* snapshots retained per document (R4.2). Manual
    /// labelled snapshots are never pruned. 0 disables new automatic snapshots
    /// without deleting versions already on disk.
    pub snapshot_keep: usize,
    /// Seconds of idle time between automatic snapshots of a dirty buffer;
    /// 0 leaves automatic snapshots to happen on save only.
    pub autosnapshot_secs: u64,
    /// Soft word wrap on startup.
    pub wrap: bool,
    /// Wrap margin in columns; 0 wraps at the window width.
    pub wrap_margin: usize,
    /// 0 = clean screen (no menus), 1 = delayed menus, 2 = menus + hint bar.
    pub help_level: u8,
    /// Underline misspelled words on startup.
    pub spellcheck: bool,
    /// Keep the current line pinned at a fixed row and scroll the document
    /// under it, instead of only scrolling once the cursor hits the edge.
    pub typewriter: bool,
    /// In focus mode, dim everything outside the paragraph being written
    /// (R3.4). Set false to keep the whole page evenly lit.
    pub focus_dim: bool,
    /// Body font for `^KM` manuscript RTF export: "times" or "courier".
    pub manuscript_font: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: String::from("wp-blue"),
            menu_delay_ms: 700,
            autosave_secs: 60,
            backup_depth: 10,
            snapshot_keep: 20,
            autosnapshot_secs: 0,
            wrap: true,
            wrap_margin: 0,
            help_level: 1,
            spellcheck: true,
            typewriter: false,
            focus_dim: true,
            manuscript_font: String::from("times"),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("perfectstar2k")
            .join("config.toml"),
    )
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(data) => toml::from_str(&data).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    pub fn theme(&self) -> Theme {
        match self.theme.as_str() {
            "wordstar" => Theme::wordstar(),
            "terminal" => Theme::terminal_default(),
            _ => Theme::wp_blue(),
        }
    }

    pub fn manuscript_font(&self) -> ManuscriptFont {
        match self.manuscript_font.as_str() {
            "courier" => ManuscriptFont::Courier,
            _ => ManuscriptFont::TimesNewRoman,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_without_backup_depth_uses_default() {
        let config: Config = toml::from_str("theme = 'wordstar'\nautosave_secs = 30\n").unwrap();

        assert_eq!(config.theme, "wordstar");
        assert_eq!(config.autosave_secs, 30);
        assert_eq!(config.backup_depth, 10);
    }

    #[test]
    fn backup_depth_accepts_zero_to_disable_new_backups() {
        let config: Config = toml::from_str("backup_depth = 0\n").unwrap();

        assert_eq!(config.backup_depth, 0);
    }

    #[test]
    fn old_config_without_snapshot_keys_uses_defaults() {
        let config: Config = toml::from_str("theme = 'wordstar'\n").unwrap();

        assert_eq!(config.snapshot_keep, 20);
        assert_eq!(config.autosnapshot_secs, 0);
    }

    #[test]
    fn focus_dim_defaults_on_and_can_be_turned_off() {
        // R3.4: dimming is optional and SHALL be configurable off.
        assert!(Config::default().focus_dim);
        let config: Config = toml::from_str("focus_dim = false\n").unwrap();
        assert!(!config.focus_dim);
    }

    #[test]
    fn snapshot_keys_are_configurable() {
        let config: Config =
            toml::from_str("snapshot_keep = 0\nautosnapshot_secs = 300\n").unwrap();

        assert_eq!(config.snapshot_keep, 0);
        assert_eq!(config.autosnapshot_secs, 300);
    }
}
