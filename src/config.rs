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
    /// Directory containing VPN config files (empty = default).
    pub servers_dir: String,
    /// Default connection strategy: first, random, preferred.
    pub default_connect: String,
    /// Preferred protocol when both exist: wireguard, openvpn.
    pub preferred_protocol: String,
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
    /// Specific server config name (without extension).
    pub server: String,
    /// Country code for preferred connections.
    pub country: String,
    /// City name for preferred connections.
    pub city: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Autoconnect {
    /// Strategy for the autoconnect systemd service.
    pub strategy: String,
    /// Seconds to wait for network connectivity.
    pub wait_timeout: u32,
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
            servers_dir: String::new(),
            default_connect: "first".into(),
            preferred_protocol: "wireguard".into(),
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
            server: String::new(),
            country: "US".into(),
            city: String::new(),
        }
    }
}

impl Default for Autoconnect {
    fn default() -> Self {
        Self {
            strategy: "first".into(),
            wait_timeout: 30,
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
    pub fn load() -> Result<Self> {
        let path = Self::path();

        if !path.exists() {
            log::debug!("No config file at {}, using defaults", path.display());
            return Ok(Self::default());
        }

        let text = std::fs::read_to_string(&path)
            .context(format!("Failed to read config: {}", path.display()))?;

        let config: Config = toml::from_str(&text)
            .context("Failed to parse config TOML")?;

        Ok(config)
    }

    pub fn path() -> PathBuf {
        config_dir().join("config.toml")
    }
}

/// ~/.config/tuinnel/
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("tuinnel")
}

/// Resolved servers directory.
pub fn servers_dir(config: &Config) -> PathBuf {
    if config.general.servers_dir.is_empty() {
        config_dir().join("servers")
    } else {
        PathBuf::from(&config.general.servers_dir)
    }
}

/// ~/.local/state/tuinnel/
pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("~"))
                .join(".local/state")
        })
        .join("tuinnel")
}

/// ~/.local/state/tuinnel/tuinnel.log
pub fn log_file() -> PathBuf {
    state_dir().join("tuinnel.log")
}
