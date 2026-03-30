// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// doctor.rs — System diagnostics
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::{config, net, output::OutputBuffer, util, vpn};
use std::path::Path;

/// Run all diagnostic checks and return structured output.
pub fn run(vpn: &vpn::ProtonVPN, _config: &config::Config) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("pvpn doctor — System Check");
    buf.blank();

    let mut issues: Vec<String> = Vec::new();

    // ── ProtonVPN CLI ───────────────────────────────────────────────────

    if let Some(ref bin) = vpn.binary {
        buf.ok(&format!("ProtonVPN CLI: {bin}"));
        let gen_label = match vpn.generation {
            vpn::CliGeneration::New => "new (official v0.1.x+)",
            vpn::CliGeneration::Legacy => "legacy (protonvpn-cli)",
        };
        buf.indent(&format!("Generation: {gen_label}"));
        if let Some(ver) = vpn.version() {
            buf.indent(&format!("Version: {ver}"));
        }
    } else {
        buf.err("ProtonVPN CLI not found");
        issues.push(
            "Install from AUR:  yay -S protonvpn-cli\n  \
                    or:  sudo pacman -S protonvpn-cli  (if in repos)"
                .into(),
        );
    }

    // ── NetworkManager ──────────────────────────────────────────────────

    if util::binary_exists("nmcli") {
        buf.ok("nmcli found");
        let (ok, _, _) = util::run_cmd("systemctl", &["is-active", "NetworkManager"]);
        if ok {
            buf.ok("NetworkManager is running");
        } else {
            buf.err("NetworkManager is not running");
            issues.push("sudo systemctl enable --now NetworkManager".into());
        }
    } else {
        buf.err("nmcli not found");
        issues.push("sudo pacman -S networkmanager".into());
    }

    // ── Core tools ──────────────────────────────────────────────────────

    for (name, pkg) in &[("curl", "curl"), ("ip", "iproute2")] {
        if util::binary_exists(name) {
            buf.ok(&format!("{name} found"));
        } else {
            buf.err(&format!("{name} not found"));
            issues.push(format!("sudo pacman -S {pkg}"));
        }
    }

    // ── TUN device ──────────────────────────────────────────────────────

    if Path::new("/dev/net/tun").exists() {
        buf.ok("TUN device available");
    } else {
        buf.err("TUN device not found (/dev/net/tun)");
        issues.push("sudo modprobe tun".into());
    }

    // ── Keyring ─────────────────────────────────────────────────────────

    let keyring_ok = 'keyring: {
        for daemon in &["gnome-keyring-daemon", "kwalletd5", "kwalletd6"] {
            if util::binary_exists(daemon) {
                let (running, _, _) = util::run_cmd("pgrep", &["-x", daemon]);
                if running {
                    buf.ok(&format!("Keyring daemon running: {daemon}"));
                    break 'keyring true;
                }
            }
        }

        if util::binary_exists("secret-tool") {
            buf.ok("secret-tool available (libsecret)");
            break 'keyring true;
        }

        false
    };

    if !keyring_ok {
        buf.warn("No keyring daemon detected — ProtonVPN may not store credentials");
        issues.push(
            "sudo pacman -S gnome-keyring libsecret\n  \
             Then ensure gnome-keyring-daemon starts with your session"
                .into(),
        );
    }

    // ── Python (some ProtonVPN CLIs need it) ────────────────────────────

    if util::binary_exists("python3") || util::binary_exists("python") {
        buf.ok("Python available (some ProtonVPN CLIs require it)");
    } else {
        buf.warn("Python not found — may be needed by some ProtonVPN CLI versions");
    }

    // ── systemd user session ────────────────────────────────────────────

    let (ok, _, _) = util::run_cmd("systemctl", &["--user", "status"]);
    if ok {
        buf.ok("systemd user session available");
    } else {
        buf.warn("systemd user session may not be available");
    }

    // ── Config and state paths ──────────────────────────────────────────

    let cfg_path = config::Config::path();
    if cfg_path.exists() {
        buf.ok(&format!("Config: {}", cfg_path.display()));
    } else {
        buf.warn(&format!("Config not found: {}", cfg_path.display()));
        buf.dim("Run install.sh or create manually");
    }

    let state = config::state_dir();
    if state.exists() {
        buf.ok(&format!("Log dir: {}", state.display()));
    } else {
        buf.warn(&format!("Log dir missing: {}", state.display()));
    }

    // ── WiFi backend ────────────────────────────────────────────────────

    buf.blank();
    buf.header("WiFi Backend");
    let backend = net::wifi_backend();
    buf.kv("Current", &backend);

    if !backend.to_lowercase().contains("iwd") {
        buf.blank();
        buf.dim("To switch to iwd with NetworkManager:");
        buf.dim("  1. sudo pacman -S iwd");
        buf.dim("  2. Create /etc/NetworkManager/conf.d/wifi_backend.conf:");
        buf.dim("     [device]");
        buf.dim("     wifi.backend=iwd");
        buf.dim("  3. sudo systemctl enable --now iwd");
        buf.dim("  4. sudo systemctl restart NetworkManager");
    }

    // ── Capabilities ────────────────────────────────────────────────────

    if vpn.binary.is_some() {
        buf.blank();
        buf.header("ProtonVPN CLI Capabilities");

        for (name, available) in vpn.caps_summary() {
            if available {
                buf.ok(name);
            } else {
                buf.dim(&format!("– {name}"));
            }
        }
    }

    // ── Security Checks ────────────────────────────────────────────────

    buf.blank();
    buf.header("Security Checks");
    let audit = crate::security::full_audit();
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

    // ── Summary ─────────────────────────────────────────────────────────

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
