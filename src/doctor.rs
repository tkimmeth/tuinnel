// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// doctor.rs — System diagnostics for tuinnel
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::backend::{SessionInfo, VpnManager};
use crate::{config, net, output::OutputBuffer, util};
use std::path::Path;

pub fn run(manager: &VpnManager, config: &config::Config, session: Option<&SessionInfo>) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("tuinnel doctor — System Check");
    buf.blank();

    let mut issues: Vec<String> = Vec::new();

    // ── VPN Tools ──────────────────────────────────────────────────────
    buf.header("VPN Backend Tools");

    if util::binary_exists("wg-quick") {
        buf.ok("wg-quick found (WireGuard)");
    } else {
        buf.err("wg-quick not found");
        issues.push("sudo pacman -S wireguard-tools".into());
    }

    if util::binary_exists("wg") {
        buf.ok("wg found");
    } else {
        buf.err("wg not found");
        issues.push("sudo pacman -S wireguard-tools".into());
    }

    if util::binary_exists("openvpn") {
        buf.ok("openvpn found");
    } else {
        buf.dim("– openvpn not found (optional, for OpenVPN configs)");
    }

    // ── Firewall ───────────────────────────────────────────────────────

    if util::binary_exists("nft") {
        buf.ok("nft found (nftables kill switch)");
    } else {
        buf.warn("nft not found — kill switch will not work");
        issues.push("sudo pacman -S nftables".into());
    }

    // ── Core tools ─────────────────────────────────────────────────────

    for (name, pkg) in &[("curl", "curl"), ("ip", "iproute2"), ("sudo", "sudo")] {
        if util::binary_exists(name) {
            buf.ok(&format!("{name} found"));
        } else {
            buf.err(&format!("{name} not found"));
            issues.push(format!("sudo pacman -S {pkg}"));
        }
    }

    // ── Sudo access ────────────────────────────────────────────────────

    let (sudo_ok, _, _) = util::run_cmd("sudo", &["-n", "true"]);
    if sudo_ok {
        buf.ok("Passwordless sudo available");
    } else {
        buf.dim("– Passwordless sudo not configured (will prompt for password)");
        buf.dim("  Optional: add to /etc/sudoers.d/tuinnel:");
        buf.dim("  %wheel ALL=(root) NOPASSWD: /usr/bin/wg-quick, /usr/bin/wg, /usr/bin/nft");
    }

    // ── TUN device ─────────────────────────────────────────────────────

    if Path::new("/dev/net/tun").exists() {
        buf.ok("TUN device available");
    } else {
        buf.err("TUN device not found (/dev/net/tun)");
        issues.push("sudo modprobe tun".into());
    }

    // ── Servers directory ──────────────────────────────────────────────

    let servers_path = config::servers_dir(config);
    if servers_path.is_dir() {
        let count = manager.servers.len();
        if count > 0 {
            buf.ok(&format!("Servers dir: {} ({count} configs found)", servers_path.display()));
        } else {
            buf.warn(&format!("Servers dir exists but empty: {}", servers_path.display()));
            buf.dim("  Drop .conf (WireGuard) files into this directory");
        }
    } else {
        buf.err(&format!("Servers dir not found: {}", servers_path.display()));
        issues.push(format!("mkdir -p {}", servers_path.display()));
    }

    // ── Config and state paths ─────────────────────────────────────────

    let cfg_path = config::Config::path();
    if cfg_path.exists() {
        buf.ok(&format!("Config: {}", cfg_path.display()));
    } else {
        buf.warn(&format!("Config not found: {}", cfg_path.display()));
        buf.dim("  Will use defaults. Create config.toml to customize.");
    }

    let state = config::state_dir();
    if state.exists() {
        buf.ok(&format!("Log dir: {}", state.display()));
    } else {
        buf.warn(&format!("Log dir missing: {}", state.display()));
    }

    // ── systemd user session ───────────────────────────────────────────

    let (ok, _, _) = util::run_cmd("systemctl", &["--user", "status"]);
    if ok {
        buf.ok("systemd user session available");
    } else {
        buf.warn("systemd user session may not be available");
    }

    // ── WiFi backend ───────────────────────────────────────────────────

    buf.blank();
    buf.header("WiFi Backend");
    buf.kv("Current", &net::wifi_backend());

    // ── Security Checks ────────────────────────────────────────────────

    buf.blank();
    buf.header("Security Checks");
    let audit = crate::security::full_audit(session);
    for check in &audit {
        if check.passed {
            buf.ok(&format!("{}: {}", check.name, check.detail));
        } else {
            match check.severity {
                crate::security::Severity::Critical => {
                    buf.err(&format!("{}: {}", check.name, check.detail));
                }
                crate::security::Severity::Warning => {
                    buf.warn(&format!("{}: {}", check.name, check.detail));
                }
                crate::security::Severity::Info => {
                    buf.dim(&format!("{}: {}", check.name, check.detail));
                }
            }
        }
    }

    // ── Summary ────────────────────────────────────────────────────────

    buf.blank();

    if issues.is_empty() {
        buf.ok("All checks passed!");
    } else {
        buf.header("Fixes Needed");
        for fix in &issues {
            for line in fix.lines() {
                buf.plain(&format!("\x1b[33m$\x1b[0m {line}"));
            }
        }
    }

    buf
}
