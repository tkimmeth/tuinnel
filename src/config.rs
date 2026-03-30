// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// config.rs — Configuration loading and defaults
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub preferred: Preferred,
    pub autoconnect: Autoconnect,
    pub logging: Logging,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct General {
    /// Override the ProtonVPN binary name (empty = auto-detect).
    pub cli_binary: String,
    /// Default connection strategy: fastest, random, preferred.
    pub default_connect: String,
    /// Automatically enable kill switch when connecting.
    pub kill_switch_on_connect: bool,
    /// Warn if kill switch is off before connecting.
    pub warn_kill_switch: bool,
    /// User latitude for globe marker (omit to hide marker).
    pub user_lat: Option<f64>,
    /// User longitude for globe marker (omit to hide marker).
    pub user_lon: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Preferred {
    /// Country code for --preferred connections.
    pub country: String,
    /// Specific server names (e.g., ["US-NY#1"]).
    pub servers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Autoconnect {
    /// Strategy for the autoconnect systemd service.
    pub strategy: String,
    /// Seconds to wait for NetworkManager connectivity.
    pub nm_wait_timeout: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Logging {
    /// Log level: DEBUG, INFO, WARNING, ERROR.
    pub level: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            preferred: Preferred::default(),
            autoconnect: Autoconnect::default(),
            logging: Logging::default(),
        }
    }
}

impl Default for General {
    fn default() -> Self {
        Self {
            cli_binary: String::new(),
            default_connect: "fastest".into(),
            kill_switch_on_connect: false,
            warn_kill_switch: true,
            user_lat: None,
            user_lon: None,
        }
    }
}

impl Default for Preferred {
    fn default() -> Self {
        Self {
            country: "US".into(),
            servers: Vec::new(),
        }
    }
}

impl Default for Autoconnect {
    fn default() -> Self {
        Self {
            strategy: "fastest".into(),
            nm_wait_timeout: 30,
        }
    }
}

impl Default for Logging {
    fn default() -> Self {
        Self {
            level: "INFO".into(),
        }
    }
}

impl Config {
    /// Load config from ~/.config/pvpn-bot/config.toml, falling back to defaults.
    pub fn load() -> Result<Self> {
        let path = Self::path();

        if !path.exists() {
            log::debug!("No config file at {}, using defaults", path.display());
            return Ok(Self::default());
        }

        // std::fs::read_to_string reads an entire file into a String.
        // .context() adds a human-readable message if the operation fails.
        let text = std::fs::read_to_string(&path)
            .context(format!("Failed to read config: {}", path.display()))?;

        // toml::from_str parses the TOML text into our Config struct.
        // Thanks to #[serde(default)], missing fields get default values.
        let config: Config = toml::from_str(&text)
            .context("Failed to parse config TOML")?;

        Ok(config)
    }

    /// Return the config file path.
    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }
}

/// ~/.config/pvpn-bot/
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("tuinnel")
}

/// ~/.local/state/pvpn-bot/
pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(".local/state")
        })
        .join("tuinnel")
}

/// ~/.local/state/pvpn-bot/pvpn.log
pub fn log_file() -> PathBuf {
    state_dir().join("tuinnel.log")
}
