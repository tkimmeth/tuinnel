// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// backend.rs — VPN backend trait, session model, and manager
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// SessionInfo is the single source of truth for all consumers (TUI,
// security, bandwidth, doctor). Backends produce it; everything else
// reads it. No module should guess interface names or provider
// conventions from system state.

use crate::servers::{self, ServerEntry};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

// ── Protocol ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    WireGuard,
    OpenVPN,
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::WireGuard => write!(f, "WireGuard"),
            Protocol::OpenVPN => write!(f, "OpenVPN"),
        }
    }
}

// ── SessionInfo ────────────────────────────────────────────────────────────

/// Canonical state of an active VPN connection. Every consumer reads this
/// instead of probing system state directly.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Path to the config file that established this session.
    pub config_path: PathBuf,
    /// WireGuard or OpenVPN.
    pub protocol: Protocol,
    /// Network interface name (e.g. "us-nyc-001", "tun0").
    pub interface: String,
    /// OpenVPN daemon PID (None for WireGuard).
    pub pid: Option<u32>,
    /// DNS servers expected while connected (parsed from config file).
    pub dns_servers: Vec<String>,
    /// Human-readable name for display (e.g. "US - New York").
    pub display_name: String,
    /// Provider name (e.g. "mullvad", "protonvpn", "self-hosted").
    pub provider: String,
    /// Country code (e.g. "US").
    pub country: String,
    /// City name (e.g. "New York").
    pub city: String,
    /// Server coordinates for globe rendering.
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    /// When the connection was established.
    pub connected_at: SystemTime,
}

// ── VpnBackend trait ───────────────────────────────────────────────────────

/// A VPN protocol backend. Implementations manage the actual tunnel.
pub trait VpnBackend: Send + Sync {
    /// Bring up a tunnel to the given server. Returns session state on success.
    fn connect(&self, server: &ServerEntry) -> Result<SessionInfo, String>;

    /// Tear down the tunnel described by the session.
    fn disconnect(&self, session: &SessionInfo) -> Result<(), String>;

    /// Detect an existing connection (e.g. on startup). Returns None if
    /// no tunnel matching this backend is active.
    fn probe_session(&self) -> Option<SessionInfo>;

    /// Protocol this backend manages.
    fn protocol(&self) -> Protocol;
}

// ── ConnectTarget ──────────────────────────────────────────────────────────

/// How to select a server for connection.
#[derive(Debug, Clone)]
pub enum ConnectTarget {
    First,
    Random,
    Country(String),
    City(String),
    Server(String),
    Preferred,
}

// ── VpnManager ─────────────────────────────────────────────────────────────

/// Owns the backend, server list, and active session. This is the central
/// object that commands, TUI, and CLI all interact with.
pub struct VpnManager {
    pub backend: Arc<dyn VpnBackend>,
    pub servers: Vec<ServerEntry>,
    pub session: Option<SessionInfo>,
}

impl VpnManager {
    pub fn new(backend: Arc<dyn VpnBackend>, servers: Vec<ServerEntry>) -> Self {
        let session = backend.probe_session();
        Self { backend, servers, session }
    }

    /// Resolve a ConnectTarget to a specific ServerEntry.
    pub fn resolve_target(
        &self,
        target: &ConnectTarget,
        config: &crate::config::Config,
    ) -> Option<ServerEntry> {
        match target {
            ConnectTarget::First => {
                servers::pick_first(&self.servers).cloned()
            }
            ConnectTarget::Random => {
                servers::pick_random(&self.servers).cloned()
            }
            ConnectTarget::Country(cc) => {
                let matches = servers::find_by_country(&self.servers, cc);
                matches.first().cloned().cloned()
            }
            ConnectTarget::City(name) => {
                let matches = servers::find_by_city(&self.servers, name);
                matches.first().cloned().cloned()
            }
            ConnectTarget::Server(name) => {
                servers::find_by_name(&self.servers, name).cloned()
            }
            ConnectTarget::Preferred => {
                // Try server name first, then city, then country
                if !config.preferred.server.is_empty() {
                    if let Some(s) = servers::find_by_name(&self.servers, &config.preferred.server) {
                        return Some(s.clone());
                    }
                }
                if !config.preferred.city.is_empty() {
                    let matches = servers::find_by_city(&self.servers, &config.preferred.city);
                    if let Some(s) = matches.first() {
                        return Some((*s).clone());
                    }
                }
                let matches = servers::find_by_country(&self.servers, &config.preferred.country);
                matches.first().cloned().cloned()
            }
        }
    }

    /// Connect to a server matching the target.
    pub fn connect(
        &mut self,
        target: &ConnectTarget,
        config: &crate::config::Config,
    ) -> Result<SessionInfo, String> {
        let server = self
            .resolve_target(target, config)
            .ok_or_else(|| format!("No server found for {:?}", target))?;
        let session = self.backend.connect(&server)?;
        self.session = Some(session.clone());
        Ok(session)
    }

    /// Disconnect the active session.
    pub fn disconnect(&mut self) -> Result<(), String> {
        let session = self.session.take().ok_or("Not connected")?;
        self.backend.disconnect(&session)
    }
}

