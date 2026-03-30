// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// backend.rs — VPN backend trait, session model, and manager
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// SessionInfo is the single source of truth for all consumers (TUI,
// security, bandwidth, doctor). Backends produce it; everything else
// reads it. No module should guess interface names or provider
// conventions from system state.

use crate::servers::ServerEntry;
use std::path::PathBuf;
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
