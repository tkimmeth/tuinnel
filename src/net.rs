// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// net.rs — Network utility functions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// These are all free functions — no struct needed. They shell out to
// standard Linux networking tools (curl, ip, nmcli, resolvectl).

use crate::util::run_cmd;
use std::fs;
use std::path::Path;

/// Fetch public IP address via ip.me.
pub fn public_ip() -> String {
    let (ok, out, _) = run_cmd("curl", &["-s", "--max-time", "5", "https://ip.me"]);
    if ok && !out.is_empty() {
        out
    } else {
        "unavailable".into()
    }
}

/// Get the default route line from `ip route`.
pub fn default_route() -> String {
    let (ok, out, _) = run_cmd("ip", &["route", "show", "default"]);
    if ok { out } else { "unavailable".into() }
}

/// Get all routes.
pub fn all_routes() -> String {
    let (ok, out, _) = run_cmd("ip", &["route"]);
    if ok { out } else { "unavailable".into() }
}

/// Get DNS servers in use.
pub fn dns_servers() -> String {
    // Try systemd-resolved first
    let (ok, out, _) = run_cmd("resolvectl", &["dns"]);
    if ok && !out.is_empty() {
        return out;
    }

    let resolv = fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let servers: Vec<&str> = resolv
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.starts_with("nameserver"))
        .collect();

    if servers.is_empty() {
        "none found".into()
    } else {
        servers.join("\n")
    }
}

/// Get the current WiFi SSID (or "N/A").
pub fn wifi_ssid() -> String {
    let (ok, out, _) = run_cmd("nmcli", &["-t", "-f", "ACTIVE,SSID", "dev", "wifi"]);
    if !ok {
        return "N/A (wired or not connected)".into();
    }

    out.lines()
        .find(|l| l.starts_with("yes:"))
        .and_then(|l| l.split_once(':')) // Split into (before, after)
        .map(|(_, ssid)| {
            if ssid.is_empty() {
                "hidden".to_string()
            } else {
                ssid.to_string()
            }
        })
        .unwrap_or_else(|| "N/A (wired or not connected)".into())
}

/// Detect the WiFi backend (wpa_supplicant or iwd).
pub fn wifi_backend() -> String {
    let conf_dir = Path::new("/etc/NetworkManager/conf.d");

    if !conf_dir.is_dir() {
        return "wpa_supplicant (default)".into();
    }

    // Read each .conf file looking for wifi.backend
    if let Ok(entries) = fs::read_dir(conf_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "conf") {
                if let Ok(text) = fs::read_to_string(&path) {
                    if let Some(line) = text.lines().find(|l| l.contains("wifi.backend")) {
                        if let Some((_, val)) = line.split_once('=') {
                            return val.trim().to_string();
                        }
                    }
                }
            }
        }
    }

    "wpa_supplicant (default)".into()
}

/// Get `nmcli general status` output.
pub fn nm_status() -> String {
    let (ok, out, _) = run_cmd("nmcli", &["general", "status"]);
    if ok { out } else { "unavailable".into() }
}

/// Check if traffic is going through a VPN tunnel interface.
/// If `expected_iface` is provided, checks for that specific interface.
/// Otherwise falls back to generic VPN interface patterns.
/// Returns (is_tunneled, raw_route_output).
pub fn is_tunneled(expected_iface: Option<&str>) -> (bool, String) {
    let (ok, out, _) = run_cmd("ip", &["route", "show", "default"]);
    if !ok {
        return (false, "could not read routes".into());
    }

    let lower = out.to_lowercase();
    let tunneled = if let Some(iface) = expected_iface {
        lower.contains(&iface.to_lowercase())
    } else {
        ["tun", "wg"].iter().any(|pat| lower.contains(pat))
    };

    (tunneled, out)
}
