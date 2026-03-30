// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// security.rs — VPN security audit and leak detection
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Five checks that verify your VPN isn't leaking:
//
//   1. Tunnel Integrity  — is traffic actually routed through the VPN?
//   2. DNS Leak          — are DNS queries going to the expected servers?
//   3. IPv6 Leak         — is IPv6 bypassing the tunnel?
//   4. Kill Switch FW    — are firewall rules actually blocking?
//   5. Connectivity Leak — can traffic escape on the physical interface?

use crate::backend::SessionInfo;
use crate::net;
use crate::output::OutputBuffer;
use crate::util::run_cmd;

/// Result of a single security check.
#[derive(Clone, Debug)]
pub struct SecurityCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
    pub severity: Severity,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

// ── Check 1: Tunnel Integrity ───────────────────────────────────────────────

pub fn check_tunnel_integrity(session: Option<&SessionInfo>) -> SecurityCheck {
    let iface_name = session.map(|s| s.interface.as_str());
    let (tunneled, route_out) = net::is_tunneled(iface_name);

    if !tunneled {
        return SecurityCheck {
            name: "Tunnel Integrity".into(),
            passed: false,
            detail: format!(
                "Default route not through VPN: {}",
                route_out.lines().next().unwrap_or("?")
            ),
            severity: Severity::Critical,
        };
    }

    // If we have a session, verify its interface is UP
    if let Some(s) = session {
        let (ok, out, _) = run_cmd("ip", &["link", "show", &s.interface]);
        if ok && out.to_lowercase().contains("up") {
            return SecurityCheck {
                name: "Tunnel Integrity".into(),
                passed: true,
                detail: format!("Traffic routed through {} (UP)", s.interface),
                severity: Severity::Info,
            };
        }
        return SecurityCheck {
            name: "Tunnel Integrity".into(),
            passed: false,
            detail: format!("Interface {} exists but is not UP", s.interface),
            severity: Severity::Warning,
        };
    }

    // No session — generic check for any VPN-like interface
    let (ok, out, _) = run_cmd("ip", &["-o", "link", "show", "up"]);
    if ok {
        let has_vpn = out.lines().any(|l| {
            let lower = l.to_lowercase();
            lower.contains("tun") || lower.contains("wg")
        });
        if has_vpn {
            return SecurityCheck {
                name: "Tunnel Integrity".into(),
                passed: true,
                detail: "Traffic routed through VPN interface".into(),
                severity: Severity::Info,
            };
        }
    }

    SecurityCheck {
        name: "Tunnel Integrity".into(),
        passed: false,
        detail: "Route mentions VPN but no VPN interface is UP".into(),
        severity: Severity::Warning,
    }
}

// ── Check 2: DNS Leak ───────────────────────────────────────────────────────

/// Verify DNS resolvers match the expected servers from the VPN config.
/// If no expected servers are provided, just reports what's in use.
pub fn check_dns_leak(expected_dns: &[String]) -> SecurityCheck {
    let dns_raw = net::dns_servers();

    let mut all_servers: Vec<String> = Vec::new();
    for line in dns_raw.lines() {
        for word in line.split_whitespace() {
            let starts_with_digit = word.chars().next().is_some_and(|c| c.is_ascii_digit());
            if starts_with_digit && (word.contains('.') || word.contains(':')) {
                all_servers.push(word.to_string());
            }
        }
    }

    if all_servers.is_empty() {
        return SecurityCheck {
            name: "DNS Leak".into(),
            passed: false,
            detail: "No DNS servers detected".into(),
            severity: Severity::Warning,
        };
    }

    if expected_dns.is_empty() {
        // No expected DNS configured — can't definitively check, just report
        return SecurityCheck {
            name: "DNS Leak".into(),
            passed: true,
            detail: format!("DNS: {} (no expected DNS to verify against)", all_servers.join(", ")),
            severity: Severity::Info,
        };
    }

    let unexpected: Vec<&String> = all_servers
        .iter()
        .filter(|s| !expected_dns.contains(s))
        .collect();

    if unexpected.is_empty() {
        SecurityCheck {
            name: "DNS Leak".into(),
            passed: true,
            detail: format!("All DNS resolvers match VPN config ({})", expected_dns.join(", ")),
            severity: Severity::Info,
        }
    } else {
        SecurityCheck {
            name: "DNS Leak".into(),
            passed: false,
            detail: format!("Unexpected DNS servers: {}", unexpected.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")),
            severity: Severity::Critical,
        }
    }
}

// ── Check 3: IPv6 Leak ─────────────────────────────────────────────────────

pub fn check_ipv6_leak(session: Option<&SessionInfo>) -> SecurityCheck {
    let (ok, out, _) = run_cmd("ip", &["-6", "addr", "show", "scope", "global"]);

    if !ok || out.trim().is_empty() {
        return SecurityCheck {
            name: "IPv6 Leak".into(),
            passed: true,
            detail: "No global IPv6 addresses (IPv6 effectively disabled)".into(),
            severity: Severity::Info,
        };
    }

    let vpn_iface = session.map(|s| s.interface.as_str());

    let mut leaking_ifaces: Vec<String> = Vec::new();
    let mut current_iface = String::new();

    for line in out.lines() {
        if line.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            if let Some(name) = line.split(':').nth(1) {
                current_iface = name.trim().to_string();
            }
        }
        if line.contains("inet6") && !line.contains("fe80") {
            let is_vpn = vpn_iface.is_some_and(|vi| current_iface == vi)
                || current_iface.starts_with("tun")
                || current_iface.starts_with("wg");
            if !is_vpn && !leaking_ifaces.contains(&current_iface) {
                leaking_ifaces.push(current_iface.clone());
            }
        }
    }

    let (ok6, out6, _) = run_cmd("ip", &["-6", "route", "show", "default"]);
    let ipv6_route_on_physical = if ok6 && !out6.trim().is_empty() {
        out6.lines().any(|l| {
            let lower = l.to_lowercase();
            let is_vpn = vpn_iface.is_some_and(|vi| lower.contains(&vi.to_lowercase()))
                || lower.contains("tun")
                || lower.contains("wg");
            !is_vpn
        })
    } else {
        false
    };

    if leaking_ifaces.is_empty() && !ipv6_route_on_physical {
        SecurityCheck {
            name: "IPv6 Leak".into(),
            passed: true,
            detail: "No IPv6 routes bypassing VPN tunnel".into(),
            severity: Severity::Info,
        }
    } else {
        let mut detail = String::from("IPv6 may bypass VPN");
        if !leaking_ifaces.is_empty() {
            detail.push_str(&format!(" on: {}", leaking_ifaces.join(", ")));
        }
        if ipv6_route_on_physical {
            detail.push_str(" (IPv6 default route on physical interface)");
        }
        SecurityCheck {
            name: "IPv6 Leak".into(),
            passed: false,
            detail,
            severity: Severity::Critical,
        }
    }
}

// ── Check 4: Kill Switch Firewall ──────────────────────────────────────────

pub fn check_kill_switch_firewall() -> SecurityCheck {
    // Check for tuinnel's nftables table
    let (nft_ok, nft_out, nft_err) = run_cmd("nft", &["list", "table", "inet", "tuinnel_killswitch"]);

    if nft_ok && !nft_out.is_empty() {
        return SecurityCheck {
            name: "Kill Switch Firewall".into(),
            passed: true,
            detail: "nftables tuinnel_killswitch rules active".into(),
            severity: Severity::Info,
        };
    }

    // Fallback: check for any blocking rules
    let (ok, out, _) = run_cmd("iptables", &["-S"]);
    if ok {
        let blocking_rules: Vec<&str> = out
            .lines()
            .filter(|l| {
                let lower = l.to_lowercase();
                (lower.contains("drop") || lower.contains("reject"))
                    && lower.contains("killswitch")
            })
            .collect();

        if !blocking_rules.is_empty() {
            return SecurityCheck {
                name: "Kill Switch Firewall".into(),
                passed: true,
                detail: format!("{} iptables blocking rules active", blocking_rules.len()),
                severity: Severity::Info,
            };
        }
    }

    // Check permissions
    let nft_err_lower = nft_err.to_lowercase();
    if nft_err_lower.contains("permission denied") || nft_err_lower.contains("operation not permitted") {
        return SecurityCheck {
            name: "Kill Switch Firewall".into(),
            passed: false,
            detail: "Cannot verify (needs root). Run: sudo tuinnel doctor".into(),
            severity: Severity::Warning,
        };
    }

    SecurityCheck {
        name: "Kill Switch Firewall".into(),
        passed: false,
        detail: "No kill switch firewall rules detected".into(),
        severity: Severity::Critical,
    }
}

// ── Check 5: Connectivity Leak ─────────────────────────────────────────────

pub fn check_connectivity_leak() -> SecurityCheck {
    let (ok, out, _) = run_cmd("ip", &["-o", "link", "show", "up"]);
    if !ok {
        return SecurityCheck {
            name: "Connectivity Leak".into(),
            passed: false,
            detail: "Cannot enumerate interfaces".into(),
            severity: Severity::Warning,
        };
    }

    let physical_iface = out
        .lines()
        .filter_map(|l| {
            let name = l.split(':').nth(1)?.trim();
            let lower = name.to_lowercase();
            if lower.starts_with("wlan")
                || lower.starts_with("eth")
                || lower.starts_with("enp")
                || lower.starts_with("wlp")
            {
                Some(name.to_string())
            } else {
                None
            }
        })
        .next();

    let Some(iface) = physical_iface else {
        return SecurityCheck {
            name: "Connectivity Leak".into(),
            passed: true,
            detail: "No physical interface found to test".into(),
            severity: Severity::Info,
        };
    };

    let (leaked, _, _) = run_cmd(
        "curl",
        &["-s", "--max-time", "3", "--interface", &iface, "https://ip.me"],
    );

    if leaked {
        SecurityCheck {
            name: "Connectivity Leak".into(),
            passed: false,
            detail: format!("Traffic leaked through {iface} (bypassed VPN!)"),
            severity: Severity::Critical,
        }
    } else {
        SecurityCheck {
            name: "Connectivity Leak".into(),
            passed: true,
            detail: format!("Direct traffic on {iface} correctly blocked"),
            severity: Severity::Info,
        }
    }
}

// ── Aggregate ──────────────────────────────────────────────────────────────

pub fn full_audit(session: Option<&SessionInfo>) -> Vec<SecurityCheck> {
    let expected_dns = session
        .map(|s| s.dns_servers.as_slice())
        .unwrap_or(&[]);

    vec![
        check_tunnel_integrity(session),
        check_dns_leak(expected_dns),
        check_ipv6_leak(session),
        check_kill_switch_firewall(),
        check_connectivity_leak(),
    ]
}

pub fn audit_report(session: Option<&SessionInfo>) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("Security Audit");

    let checks = full_audit(session);
    let all_passed = checks.iter().all(|c| c.passed);

    for check in &checks {
        if check.passed {
            buf.ok(&format!("{}: {}", check.name, check.detail));
        } else {
            match check.severity {
                Severity::Critical => buf.err(&format!("{}: {}", check.name, check.detail)),
                Severity::Warning => buf.warn(&format!("{}: {}", check.name, check.detail)),
                Severity::Info => buf.dim(&format!("{}: {}", check.name, check.detail)),
            }
        }
    }

    buf.blank();
    if all_passed {
        buf.ok("All security checks passed");
    } else {
        buf.warn("Some checks failed — review above");
    }

    buf
}
