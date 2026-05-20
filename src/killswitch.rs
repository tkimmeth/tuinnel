// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// killswitch.rs — Native nftables kill switch
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Installs/removes an nftables table that blocks all traffic except:
//   - Loopback (lo)
//   - The VPN tunnel interface (wg*, tun*)
//   - Traffic to the VPN endpoint IP (so the tunnel can be established)
//   - DHCP (UDP 67/68, so the LAN connection stays alive)
//   - Established/related return traffic
//
// State is the table's existence — no sidecar files needed.

use crate::backend::SessionInfo;
use crate::privilege::{run_privileged, run_privileged_stdin};
use crate::util::run_cmd;

/// Enable the kill switch for the given session.
pub fn enable(session: &SessionInfo) -> Result<String, String> {
    // Get endpoint IP: try wg show first (works for probed sessions),
    // fall back to parsing config file
    let endpoint_ip = get_endpoint_from_wg(&session.interface)
        .or_else(|| parse_endpoint_ip(&session.config_path))
        .unwrap_or_default();

    if endpoint_ip.is_empty() {
        return Err("Could not determine VPN endpoint IP".into());
    }

    // Build the nftables ruleset
    let ruleset = format!(
        r#"table inet tuinnel_killswitch {{
    chain output {{
        type filter hook output priority 0; policy drop;
        oifname "lo" accept
        oifname "{iface}" accept
        ip daddr {endpoint} accept
        udp dport {{ 67, 68 }} accept
        ct state established,related accept
        reject
    }}
    chain input {{
        type filter hook input priority 0; policy drop;
        iifname "lo" accept
        iifname "{iface}" accept
        ct state established,related accept
        udp sport {{ 67, 68 }} accept
        reject
    }}
}}"#,
        iface = session.interface,
        endpoint = endpoint_ip,
    );

    // Remove existing table first (ignore errors if it doesn't exist)
    let _ = run_privileged("nft", &["delete", "table", "inet", "tuinnel_killswitch"]);

    // Install the new ruleset by piping through nft's stdin. Avoiding a
    // predictable temp file (`/tmp/tuinnel-killswitch.nft`) closes the
    // TOCTOU window where another local user could swap the file between
    // write and `nft -f`.
    let (ok, stdout, stderr) = run_privileged_stdin("nft", &["-f", "-"], &ruleset);

    if !ok {
        let err = if !stderr.is_empty() { stderr } else { stdout };
        return Err(format!("Failed to install kill switch rules: {err}"));
    }

    log::info!("Kill switch enabled for interface {} (endpoint {})", session.interface, endpoint_ip);
    Ok(format!("Kill switch ON — only {} traffic allowed", session.interface))
}

/// Disable the kill switch.
pub fn disable() -> Result<String, String> {
    let (ok, stdout, stderr) = run_privileged("nft", &["delete", "table", "inet", "tuinnel_killswitch"]);

    if !ok {
        let err = if !stderr.is_empty() { stderr } else { stdout };
        // If table doesn't exist, that's fine
        if err.to_lowercase().contains("no such") || err.to_lowercase().contains("does not exist") {
            return Ok("Kill switch already OFF".into());
        }
        return Err(format!("Failed to remove kill switch rules: {err}"));
    }

    log::info!("Kill switch disabled");
    Ok("Kill switch OFF — all traffic allowed".into())
}

/// Check if the kill switch is currently active.
pub fn is_active() -> bool {
    // Try without sudo first
    let (ok, _, _) = run_cmd("nft", &["list", "table", "inet", "tuinnel_killswitch"]);
    if ok {
        return true;
    }
    // Try with sudo (nft list often needs root)
    let (ok, _, _) = run_privileged("nft", &["list", "table", "inet", "tuinnel_killswitch"]);
    ok
}

/// Get kill switch status as a display string.
pub fn status_string() -> String {
    if is_active() {
        "ON (nftables rules active)".into()
    } else {
        "OFF".into()
    }
}

/// Get the endpoint IP from a live WireGuard interface via `wg show`.
fn get_endpoint_from_wg(iface: &str) -> Option<String> {
    // Try without sudo first, then with
    let (ok, out, _) = run_cmd("wg", &["show", iface, "endpoints"]);
    let output = if ok && !out.is_empty() {
        out
    } else {
        let (ok2, out2, _) = run_privileged("wg", &["show", iface, "endpoints"]);
        if !ok2 { return None; }
        out2
    };

    // Format: "<pubkey>\t<ip:port>"
    let endpoint = output.lines().next()?.split('\t').nth(1)?.trim();
    if endpoint.starts_with('[') {
        // IPv6: [addr]:port
        endpoint.strip_prefix('[')?.split(']').next().map(|s| s.to_string())
    } else {
        // IPv4: addr:port
        endpoint.rsplit_once(':').map(|(ip, _)| ip.to_string())
    }
}

/// Parse the VPN endpoint IP from a WireGuard config file.
fn parse_endpoint_ip(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("Endpoint") {
            let value = value.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
            // Format: "IP:port" or "[IPv6]:port"
            if value.starts_with('[') {
                // IPv6: [addr]:port
                return value.strip_prefix('[')?.split(']').next().map(|s| s.to_string());
            } else {
                // IPv4: addr:port
                return value.rsplit_once(':').map(|(ip, _)| ip.to_string());
            }
        }
    }

    None
}
