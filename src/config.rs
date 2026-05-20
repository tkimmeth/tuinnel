// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// config.rs — Configuration loading and defaults
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Home directory of the user whose configs and state we should use.
///
/// When tuinnel is launched via `sudo` (e.g. from a waybar `on-click` that
/// elevates so the TUI can run privileged ops without mid-session prompts),
/// `HOME` becomes `/root` and the dirs crate would point us at root's empty
/// `~/.config/tuinnel/`. SUDO_USER is preserved by sudo even when HOME is
/// reset, so we resolve the invoking user's real home via NSS (getent) and
/// fall back to `/home/<user>` if NSS is unavailable.
pub fn effective_home() -> PathBuf {
    if let Ok(user) = std::env::var("SUDO_USER") {
        if let Ok(out) = Command::new("getent").args(["passwd", &user]).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Some(home) = s.split(':').nth(5) {
                    let home = home.trim();
                    if !home.is_empty() {
                        return PathBuf::from(home);
                    }
                }
            }
        }
        return PathBuf::from(format!("/home/{user}"));
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// (uid, gid) of the invoking user when running under sudo, else None.
/// Used to hand state files back to that user so subsequent user-mode
/// invocations can read them.
fn invoking_user_ids() -> Option<(u32, u32)> {
    let user = std::env::var("SUDO_USER").ok()?;
    let out = Command::new("getent")
        .args(["passwd", &user])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let fields: Vec<&str> = s.split(':').collect();
    let uid: u32 = fields.get(2)?.trim().parse().ok()?;
    let gid: u32 = fields.get(3)?.trim().parse().ok()?;
    Some((uid, gid))
}

/// If running under sudo, chown `path` to the invoking user. No-op otherwise.
/// Call this after writing files under `state_dir` so a later user-mode run
/// of tuinnel can read them.
pub fn chown_to_invoking_user(path: &Path) {
    if let Some((uid, gid)) = invoking_user_ids() {
        let _ = std::os::unix::fs::chown(path, Some(uid), Some(gid));
    }
}

/// ~/.config/tuinnel/
pub fn config_dir() -> PathBuf {
    effective_home().join(".config").join("tuinnel")
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
    effective_home().join(".local/state/tuinnel")
}

/// ~/.local/state/tuinnel/tuinnel.log
pub fn log_file() -> PathBuf {
    state_dir().join("tuinnel.log")
}

/// ~/.local/state/tuinnel/runtime/
///
/// Holds the staged copy of the active config that wg-quick actually reads.
/// Lives under state_dir (not config_dir) because it's ephemeral derived state
/// — wiped on disconnect, rewritten on every connect.
pub fn runtime_dir() -> PathBuf {
    state_dir().join("runtime")
}

/// ~/.local/state/tuinnel/runtime/tuinnel0.conf
///
/// Fixed path where the chosen .conf is staged before `wg-quick up`. The
/// filename stem dictates the kernel interface name, so the tunnel is always
/// called `tuinnel0` regardless of which provider/country the user picked.
pub fn runtime_config_path() -> PathBuf {
    runtime_dir().join("tuinnel0.conf")
}
