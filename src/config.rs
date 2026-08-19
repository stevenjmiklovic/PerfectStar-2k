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
    /// Style checking on at startup (R8.1). Off by default: style advice is
    /// opinionated, and a fresh install shouldn't start by arguing.
    pub style: bool,
    /// Individual checks (R8.7, ADR-015).
    pub style_passive: bool,
    pub style_adverbs: bool,
    pub style_filler: bool,
    pub style_long_sentences: bool,
    /// Words past which a sentence is flagged as very long.
    pub style_sentence_words: usize,
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
            style: false,
            style_passive: true,
            style_adverbs: true,
            style_filler: true,
            style_long_sentences: true,
            style_sentence_words: 30,
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

    /// The per-check configuration for the style engine (R8.7).
    pub fn style_checks(&self) -> crate::style::StyleChecks {
        crate::style::StyleChecks {
            passive: self.style_passive,
            adverbs: self.style_adverbs,
            filler: self.style_filler,
            long_sentences: self.style_long_sentences,
            // Zero would flag every sentence; treat it as "use the default".
            sentence_words: if self.style_sentence_words == 0 {
                crate::style::StyleChecks::default().sentence_words
            } else {
                self.style_sentence_words
            },
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
    fn style_defaults_are_off_with_every_check_ready() {
        let config = Config::default();
        assert!(!config.style, "style advice is opt-in");
        let checks = config.style_checks();
        assert!(checks.passive && checks.adverbs && checks.filler && checks.long_sentences);
        assert_eq!(checks.sentence_words, 30);
    }

    #[test]
    fn individual_style_checks_are_configurable() {
        let config: Config =
            toml::from_str("style = true\nstyle_adverbs = false\nstyle_sentence_words = 20\n")
                .unwrap();

        assert!(config.style);
        let checks = config.style_checks();
        assert!(!checks.adverbs);
        assert!(checks.passive, "the others keep their defaults");
        assert_eq!(checks.sentence_words, 20);
    }

    #[test]
    fn a_zero_sentence_threshold_falls_back_to_the_default() {
        // Zero would flag every sentence in the document, which is nobody's
        // intent when they type it.
        let config: Config = toml::from_str("style_sentence_words = 0\n").unwrap();
        assert_eq!(config.style_checks().sentence_words, 30);
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
