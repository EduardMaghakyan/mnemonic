use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CONFIG_RELATIVE: &str = ".config/mnemonic/config.toml";
pub const DEFAULT_CONFIG_TOML: &str = include_str!("default_config.toml");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub hotkey: HotkeySection,
    #[serde(default)]
    pub audio: AudioSection,
    #[serde(default)]
    pub paths: PathsSection,
    #[serde(default)]
    pub model: ModelSection,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: HotkeySection::default(),
            audio: AudioSection::default(),
            paths: PathsSection::default(),
            model: ModelSection::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyMode {
    /// Press to start, press again to stop.
    Toggle,
    /// Hold to record, release to stop. Default.
    #[default]
    Hold,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HotkeySection {
    pub combo: String,
    pub mode: HotkeyMode,
    /// Optional combo that triggers `screencapture -i` and then auto-starts a
    /// recording with the captured image attached. Empty string disables it.
    pub screenshot_combo: String,
}
impl Default for HotkeySection {
    fn default() -> Self {
        Self {
            combo: "ctrl+alt+space".into(),
            mode: HotkeyMode::Hold,
            screenshot_combo: "ctrl+alt+cmd+space".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AudioSection {
    pub max_seconds: u32,
    pub keep_raw: bool,
}
impl Default for AudioSection {
    fn default() -> Self { Self { max_seconds: 300, keep_raw: true } }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PathsSection {
    pub notes_dir: String,
    pub audio_dir: String,
    pub inbox_dir: String,
}
impl Default for PathsSection {
    fn default() -> Self {
        Self {
            notes_dir: "~/Mnemonic/notes".into(),
            audio_dir: "~/Mnemonic/audio".into(),
            inbox_dir: "~/Mnemonic/inbox".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModelSection {
    pub endpoint: String,
    pub name: String,
    pub thinking: bool,
}
impl Default for ModelSection {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:5809".into(),
            name: "gemma-4-e4b-it".into(),
            thinking: true,
        }
    }
}

impl Config {
    /// Load the config from the given path, or return the default if the file
    /// is missing. Surfaces parse errors so the caller can decide to log them.
    pub fn load_from(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("parse config: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("read config: {e}")),
        }
    }

    /// If the config file does not exist, write the bundled default to its
    /// path (creating parent directories as needed).
    pub fn ensure_exists(path: &Path) -> Result<bool, String> {
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir config: {e}"))?;
        }
        fs::write(path, DEFAULT_CONFIG_TOML).map_err(|e| format!("write config: {e}"))?;
        Ok(true)
    }

    /// Path to the user's config file, computed as `$HOME / .config/mnemonic/config.toml`.
    pub fn default_path(home: &Path) -> PathBuf {
        home.join(CONFIG_RELATIVE)
    }

    /// Expand a leading `~` in a path string against `home`.
    pub fn expand_home(value: &str, home: &Path) -> PathBuf {
        if let Some(rest) = value.strip_prefix("~/") {
            home.join(rest)
        } else if value == "~" {
            home.to_path_buf()
        } else {
            PathBuf::from(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_returns_default() {
        let path = std::env::temp_dir().join(format!(
            "mnemonic-missing-{}.toml",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_file_falls_back_to_defaults_per_section() {
        let toml = "[model]\nendpoint = \"http://localhost:9999\"\n";
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.model.endpoint, "http://localhost:9999");
        assert_eq!(cfg.model.name, "gemma-4-e4b-it");
        assert!(cfg.model.thinking);
        assert_eq!(cfg.hotkey.combo, "ctrl+alt+space");
        assert_eq!(cfg.audio.max_seconds, 300);
    }

    #[test]
    fn default_config_toml_parses_to_default() {
        let cfg: Config = toml::from_str(DEFAULT_CONFIG_TOML).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn expand_home_handles_tilde() {
        let home = Path::new("/Users/alice");
        assert_eq!(Config::expand_home("~/Mnemonic", home), Path::new("/Users/alice/Mnemonic"));
        assert_eq!(Config::expand_home("~", home), Path::new("/Users/alice"));
        assert_eq!(Config::expand_home("/abs/path", home), Path::new("/abs/path"));
    }

    #[test]
    fn screenshot_combo_defaults_when_omitted() {
        let toml = "[hotkey]\ncombo = \"ctrl+alt+space\"\nmode = \"hold\"\n";
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.hotkey.screenshot_combo, "ctrl+alt+cmd+space");
    }

    #[test]
    fn screenshot_combo_accepts_empty_string_to_disable() {
        let toml = "[hotkey]\nscreenshot_combo = \"\"\n";
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.hotkey.screenshot_combo, "");
    }

    #[test]
    fn ensure_exists_writes_default_when_missing() {
        let dir = std::env::temp_dir().join(format!("mnemonic-cfg-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested/config.toml");
        let wrote = Config::ensure_exists(&path).unwrap();
        assert!(wrote);
        let cfg = Config::load_from(&path).unwrap();
        assert_eq!(cfg, Config::default());
        let wrote_again = Config::ensure_exists(&path).unwrap();
        assert!(!wrote_again);
        let _ = fs::remove_dir_all(&dir);
    }
}
