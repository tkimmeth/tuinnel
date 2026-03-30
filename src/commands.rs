// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// commands.rs — Shared command implementations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// These functions are called from both main.rs (CLI) and tui.rs (TUI menu).

use crate::config::Config;
use crate::output::OutputBuffer;
use crate::vpn::{ConnectMode, ProtonVPN};
use crate::net;

/// Display full VPN + network status.
pub fn do_status(vpn: &ProtonVPN) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("ProtonVPN Status");

    match vpn.status() {
        Ok(text) => {
            for line in text.lines() {
                if !line.trim().is_empty() {
                    buf.plain(line);
                }
            }
        }
        Err(e) => buf.err(&format!("Could not get VPN status: {e}")),
    }

    buf.header("Network");

    let route = net::default_route();
    buf.kv("Default route", &route);

    let (tunneled, _) = net::is_tunneled();
    if tunneled {
        buf.ok("Traffic appears tunneled through VPN");
    } else {
        buf.warn("Traffic does NOT appear tunneled");
    }

    buf.kv("Public IP", &net::public_ip());

    buf.kv("DNS servers", "");
    for line in net::dns_servers().lines() {
        buf.indent(line);
    }

    buf
}

/// Connect to ProtonVPN.
pub fn do_connect(vpn: &ProtonVPN, config: &Config, mode: ConnectMode) -> OutputBuffer {
    let mut buf = OutputBuffer::new();

    if vpn.binary.is_none() {
        buf.err("ProtonVPN CLI not found. Run 'pvpn doctor' for help.");
        return buf;
    }

    // Kill switch warning
    if config.general.warn_kill_switch && vpn.caps.kill_switch {
        if let Ok(ks_out) = vpn.kill_switch("status") {
            let lo = ks_out.to_lowercase();
            if lo.contains("off") || lo.contains("disabled") || lo.contains("inactive") {
                buf.warn("Kill switch is OFF. Your IP may leak if VPN drops.");
                buf.dim("Enable with: pvpn ks on");
            }
        }
    }

    // Auto-enable kill switch if configured
    if config.general.kill_switch_on_connect && vpn.caps.kill_switch {
        buf.dim("Enabling kill switch...");
        let _ = vpn.kill_switch("on");
    }

    let label = match &mode {
        ConnectMode::Fastest => "fastest".to_string(),
        ConnectMode::Random => "random".to_string(),
        ConnectMode::Country(cc) => format!("country: {cc}"),
        ConnectMode::City(city) => format!("city: {city}"),
        ConnectMode::Server(s) => format!("server: {s}"),
        ConnectMode::Preferred => format!("preferred ({})", config.preferred.country),
    };

    buf.header(&format!("Connecting ({label})"));

    match vpn.connect_full(&mode, config) {
        Ok(text) => {
            buf.ok("Connected!");
            for line in text.lines() {
                if !line.trim().is_empty() {
                    buf.plain(line);
                }
            }
            buf.kv("Public IP", &net::public_ip());
        }
        Err(e) => buf.err(&format!("Connection failed: {e}")),
    }

    buf
}

/// Disconnect from ProtonVPN.
pub fn do_disconnect(vpn: &ProtonVPN) -> OutputBuffer {
    let mut buf = OutputBuffer::new();

    if vpn.binary.is_none() {
        buf.err("ProtonVPN CLI not found. Run 'pvpn doctor' for help.");
        return buf;
    }

    buf.header("Disconnecting");

    match vpn.disconnect() {
        Ok(text) => {
            buf.ok("Disconnected.");
            for line in text.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && trimmed.to_lowercase() != "disconnected" {
                    buf.plain(line);
                }
            }
        }
        Err(e) => buf.err(&format!("Disconnect failed: {e}")),
    }

    buf
}

/// Manage kill switch.
pub fn do_ks(vpn: &ProtonVPN, action: &str) -> OutputBuffer {
    let mut buf = OutputBuffer::new();

    if vpn.binary.is_none() {
        buf.err("ProtonVPN CLI not found. Run 'pvpn doctor' for help.");
        return buf;
    }

    buf.header(&format!("Kill Switch ({action})"));

    match vpn.kill_switch(action) {
        Ok(msg) => buf.ok(&msg),
        Err(e) => buf.err(&e),
    }

    buf
}

/// List available countries.
pub fn do_countries(vpn: &ProtonVPN) -> OutputBuffer {
    let mut buf = OutputBuffer::new();

    if vpn.binary.is_none() {
        buf.err("ProtonVPN CLI not found. Run 'pvpn doctor' for help.");
        return buf;
    }

    buf.header("Available Countries");

    match vpn.list_countries() {
        Ok(countries) => {
            if countries.is_empty() {
                buf.warn("No countries returned");
            } else {
                for (name, code) in &countries {
                    buf.kv(code, name);
                }
                buf.blank();
                buf.dim(&format!("{} countries available", countries.len()));
                buf.dim("Use: pvpn cities <CODE> to see cities");
            }
        }
        Err(e) => buf.err(&format!("Could not list countries: {e}")),
    }

    buf
}

/// List available cities in a country.
pub fn do_cities(vpn: &ProtonVPN, country: &str) -> OutputBuffer {
    let mut buf = OutputBuffer::new();

    if vpn.binary.is_none() {
        buf.err("ProtonVPN CLI not found. Run 'pvpn doctor' for help.");
        return buf;
    }

    buf.header(&format!("Cities in {}", country.to_uppercase()));

    match vpn.list_cities(country) {
        Ok(cities) => {
            if cities.is_empty() {
                buf.warn("No cities found");
            } else {
                for city in &cities {
                    buf.plain(city);
                }
                buf.blank();
                buf.dim(&format!("{} cities available", cities.len()));
                buf.dim("Use: pvpn go <city> to connect");
            }
        }
        Err(e) => buf.err(&format!("Could not list cities: {e}")),
    }

    buf
}

/// Show network information.
pub fn do_net() -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("Network Information");

    let nm = net::nm_status();
    buf.kv("NetworkManager", "");
    for line in nm.lines() {
        buf.indent(line);
    }

    buf.blank();
    buf.kv("WiFi SSID", &net::wifi_ssid());
    buf.kv("WiFi backend", &net::wifi_backend());

    buf.blank();
    buf.kv("Routes", "");
    for line in net::all_routes().lines() {
        buf.indent(line);
    }

    buf.blank();
    let (tunneled, _) = net::is_tunneled();
    if tunneled {
        buf.ok("Traffic appears tunneled through VPN");
    } else {
        buf.warn("Traffic is NOT going through a VPN tunnel");
    }

    buf
}
