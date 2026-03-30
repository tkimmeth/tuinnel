// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// wireguard.rs — WireGuard backend via wg-quick
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Manages WireGuard tunnels using wg-quick (from wireguard-tools).
// Interface name = config file stem (e.g., us-nyc-001.conf → us-nyc-001).

use crate::backend::{Protocol, SessionInfo, VpnBackend};
use crate::privilege::run_privileged;
use crate::servers::ServerEntry;
use crate::util::run_cmd;
use std::fs;
use std::time::SystemTime;

pub struct WireGuardBackend;

impl VpnBackend for WireGuardBackend {
    fn connect(&self, server: &ServerEntry) -> Result<SessionInfo, String> {
        let path = &server.config_path;
        let iface = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("Invalid config filename")?
            .to_string();

        // Disconnect any existing tunnel on this interface first
        let (_, _, _) = run_privileged("wg-quick", &["down", &iface]);

        log::info!("wg-quick up {}", path.display());
        let (ok, stdout, stderr) = run_privileged("wg-quick", &["up", &path.to_string_lossy()]);

        if !ok {
            let err = if !stderr.is_empty() { stderr } else { stdout };
            return Err(format!("wg-quick up failed: {err}"));
        }

        // Parse DNS from config file for SessionInfo
        let dns_servers = parse_dns_from_config(path);

        let display_name = if !server.city.is_empty() {
            format!("{} - {}", server.country, server.city)
        } else if !server.country.is_empty() {
            server.country.clone()
        } else {
            server.name.clone()
        };

        Ok(SessionInfo {
            config_path: path.clone(),
            protocol: Protocol::WireGuard,
            interface: iface,
            pid: None,
            dns_servers,
            display_name,
            provider: server.provider.clone(),
            country: server.country.clone(),
            city: server.city.clone(),
            lat: server.lat,
            lon: server.lon,
            connected_at: SystemTime::now(),
        })
    }

    fn disconnect(&self, session: &SessionInfo) -> Result<(), String> {
        log::info!("wg-quick down {}", session.interface);
        let (ok, stdout, stderr) = run_privileged("wg-quick", &["down", &session.interface]);

        if !ok {
            let err = if !stderr.is_empty() { stderr } else { stdout };
            return Err(format!("wg-quick down failed: {err}"));
        }

        Ok(())
    }

    fn probe_session(&self) -> Option<SessionInfo> {
        // Find active WireGuard interfaces
        let (ok, out, _) = run_cmd("ip", &["-o", "link", "show", "type", "wireguard"]);
        if !ok || out.trim().is_empty() {
            return None;
        }

        // Parse interface name from ip output: "N: <iface>: <FLAGS>..."
        let iface = out
            .lines()
            .next()?
            .split(':')
            .nth(1)?
            .trim()
            .to_string();

        // Verify it has a recent handshake (active connection)
        let (wg_ok, wg_out, _) = run_cmd("wg", &["show", &iface, "latest-handshakes"]);
        if !wg_ok {
            // wg show might need root — try with sudo
            let (wg_ok2, wg_out2, _) = run_privileged("wg", &["show", &iface, "latest-handshakes"]);
            if !wg_ok2 {
                return None;
            }
            return check_handshake(&iface, &wg_out2);
        }

        check_handshake(&iface, &wg_out)
    }

    fn protocol(&self) -> Protocol {
        Protocol::WireGuard
    }
}

/// Check if the handshake is recent enough to consider the tunnel active.
fn check_handshake(iface: &str, wg_output: &str) -> Option<SessionInfo> {
    // Output format: "<pubkey>\t<epoch_seconds>"
    let timestamp: u64 = wg_output
        .lines()
        .next()?
        .split('\t')
        .nth(1)?
        .trim()
        .parse()
        .ok()?;

    if timestamp == 0 {
        return None; // Never had a handshake
    }

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();

    // Consider active if handshake was within the last 5 minutes
    if now - timestamp > 300 {
        return None;
    }

    Some(SessionInfo {
        config_path: std::path::PathBuf::new(),
        protocol: Protocol::WireGuard,
        interface: iface.to_string(),
        pid: None,
        dns_servers: Vec::new(),
        display_name: iface.to_string(),
        provider: String::new(),
        country: String::new(),
        city: String::new(),
        lat: None,
        lon: None,
        connected_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(timestamp),
    })
}

/// Parse DNS servers from a WireGuard config file.
fn parse_dns_from_config(path: &std::path::Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(dns_value) = trimmed.strip_prefix("DNS") {
            // Handle "DNS = 10.2.0.1, 10.2.0.2" or "DNS=10.2.0.1"
            let value = dns_value.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
            return value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    Vec::new()
}
