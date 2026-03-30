// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// security.rs — VPN security audit and leak detection
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Five checks that verify your VPN isn't leaking:
//
//   1. Tunnel Integrity  — is traffic actually routed through the VPN?
//   2. DNS Leak          — are DNS queries going to ProtonDNS (10.2.0.1)?
//   3. IPv6 Leak         — is IPv6 bypassing the tunnel?
//   4. Kill Switch FW    — are iptables/nftables rules actually blocking?
//   5. Connectivity Leak — can traffic escape on the physical interface?

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

/// Verify that the default route goes through a VPN tunnel interface
/// AND that the interface is actually UP.
pub fn check_tunnel_integrity() -> SecurityCheck {
    let (tunneled, route_out) = net::is_tunneled();

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

    // Verify VPN interface exists and is UP
    let vpn_ifaces = ["proton0", "wg0", "tun0"];
    let mut active_vpn = None;

    for iface in &vpn_ifaces {
        let (ok, out, _) = run_cmd("ip", &["link", "show", iface]);
        if ok && out.to_lowercase().contains("up") {
            active_vpn = Some((*iface).to_string());
            break;
        }
    }

    match active_vpn {
        Some(iface) => SecurityCheck {
            name: "Tunnel Integrity".into(),
            passed: true,
            detail: format!("Traffic routed through {iface} (UP)"),
            severity: Severity::Info,
        },
        None => SecurityCheck {
            name: "Tunnel Integrity".into(),
            passed: false,
            detail: "Route mentions VPN but no VPN interface is UP".into(),
            severity: Severity::Warning,
        },
    }
}

// ── Check 2: DNS Leak ───────────────────────────────────────────────────────

/// Verify all configured DNS resolvers are ProtonDNS (10.2.0.1).
/// Non-Proton DNS servers mean your queries could be visible to your ISP.
pub fn check_dns_leak() -> SecurityCheck {
    let dns_raw = net::dns_servers();

    let mut all_servers: Vec<String> = Vec::new();
    let mut non_proton: Vec<String> = Vec::new();

    for line in dns_raw.lines() {
        for word in line.split_whitespace() {
            // Look for things that resemble IP addresses
            let starts_with_digit = word.chars().next().is_some_and(|c| c.is_ascii_digit());
            if starts_with_digit && (word.contains('.') || word.contains(':')) {
                all_servers.push(word.to_string());
                // ProtonVPN routes DNS through 10.2.0.1
                if word != "10.2.0.1" {
                    non_proton.push(word.to_string());
                }
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

    if non_proton.is_empty() {
        SecurityCheck {
            name: "DNS Leak".into(),
            passed: true,
            detail: "All DNS resolvers are ProtonDNS (10.2.0.1)".into(),
            severity: Severity::Info,
        }
    } else {
        SecurityCheck {
            name: "DNS Leak".into(),
            passed: false,
            detail: format!("Non-ProtonDNS servers: {}", non_proton.join(", ")),
            severity: Severity::Critical,
        }
    }
}

// ── Check 3: IPv6 Leak ─────────────────────────────────────────────────────

/// Detect IPv6 addresses and routes on non-VPN interfaces.
/// IPv6 traffic can bypass an IPv4-only VPN tunnel entirely.
pub fn check_ipv6_leak() -> SecurityCheck {
    // Check for global IPv6 addresses on non-VPN interfaces
    let (ok, out, _) = run_cmd("ip", &["-6", "addr", "show", "scope", "global"]);

    if !ok || out.trim().is_empty() {
        return SecurityCheck {
            name: "IPv6 Leak".into(),
            passed: true,
            detail: "No global IPv6 addresses (IPv6 effectively disabled)".into(),
            severity: Severity::Info,
        };
    }

    // Parse which interfaces have global IPv6
    let mut leaking_ifaces: Vec<String> = Vec::new();
    let mut current_iface = String::new();

    for line in out.lines() {
        // Lines starting with a digit indicate a new interface section
        if line.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            if let Some(name) = line.split(':').nth(1) {
                current_iface = name.trim().to_string();
            }
        }
        // inet6 line (not link-local fe80) on a non-VPN interface = leak
        if line.contains("inet6") && !line.contains("fe80") {
            let lower = current_iface.to_lowercase();
            let is_vpn = lower.contains("proton")
                || lower.contains("tun")
                || lower.contains("wg")
                || lower.contains("pvpn");
            if !is_vpn && !leaking_ifaces.contains(&current_iface) {
                leaking_ifaces.push(current_iface.clone());
            }
        }
    }

    // Also check for IPv6 default routes on non-VPN interfaces
    let (ok6, out6, _) = run_cmd("ip", &["-6", "route", "show", "default"]);
    let ipv6_route_on_physical = if ok6 && !out6.trim().is_empty() {
        out6.lines().any(|l| {
            let lower = l.to_lowercase();
            !lower.contains("proton") && !lower.contains("tun") && !lower.contains("wg")
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

/// Verify that actual iptables/nftables rules exist to block non-VPN traffic.
/// The NM profile "pvpn-killswitch" alone doesn't guarantee the rules are
/// loaded — this checks the kernel's actual firewall state.
///
/// Note: requires root for iptables -S / nft list ruleset. If unprivileged,
/// reports a warning rather than a false failure.
pub fn check_kill_switch_firewall() -> SecurityCheck {
    // Try iptables first
    let (ok, out, _) = run_cmd("iptables", &["-S"]);

    if ok {
        let blocking_rules: Vec<&str> = out
            .lines()
            .filter(|l| {
                let lower = l.to_lowercase();
                (lower.contains("drop") || lower.contains("reject"))
                    && (lower.contains("proton")
                        || lower.contains("pvpn")
                        || lower.contains("killswitch"))
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

        // iptables worked but no VPN-related rules — fall through to nftables check
    }

    // Try nftables (either as fallback after iptables found no rules, or because iptables failed)
    let (nft_ok, nft_out, nft_err) = run_cmd("nft", &["list", "ruleset"]);

    if nft_ok {
        let lower = nft_out.to_lowercase();
        if lower.contains("proton") || lower.contains("pvpn") || lower.contains("killswitch") {
            return SecurityCheck {
                name: "Kill Switch Firewall".into(),
                passed: true,
                detail: "nftables rules active for kill switch".into(),
                severity: Severity::Info,
            };
        }
    } else {
        let nft_err_lower = nft_err.to_lowercase();
        if (nft_err_lower.contains("permission denied")
            || nft_err_lower.contains("operation not permitted"))
            && !ok
        {
            // Both iptables and nftables need root
            return SecurityCheck {
                name: "Kill Switch Firewall".into(),
                passed: false,
                detail: "Cannot verify (needs root). Run: sudo pvpn doctor".into(),
                severity: Severity::Warning,
            };
        }
    }

    SecurityCheck {
        name: "Kill Switch Firewall".into(),
        passed: false,
        detail: "No kill switch firewall rules detected".into(),
        severity: Severity::Critical,
    }
}

// ── Check 5: Connectivity Leak ─────────────────────────────────────────────

/// Practical test: try to reach the internet directly on the physical
/// interface (bypassing the VPN). If the kill switch is working, this
/// request should be blocked by firewall rules.
pub fn check_connectivity_leak() -> SecurityCheck {
    // Find the physical network interface (wlan/eth/enp/wlp)
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
            // Format: "2: wlan0: <BROADCAST..."
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

    // Try to reach an endpoint directly on the physical interface
    // If kill switch is working, this should fail (blocked by firewall)
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

/// Run all security checks and return results.
pub fn full_audit() -> Vec<SecurityCheck> {
    vec![
        check_tunnel_integrity(),
        check_dns_leak(),
        check_ipv6_leak(),
        check_kill_switch_firewall(),
        check_connectivity_leak(),
    ]
}

/// Produce an OutputBuffer from a full security audit.
pub fn audit_report() -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("Security Audit");

    let checks = full_audit();
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
