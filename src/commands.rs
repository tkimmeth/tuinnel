// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// commands.rs — Shared command implementations (CLI + TUI)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::backend::{ConnectTarget, SessionInfo, VpnManager};
use crate::config::Config;
use crate::killswitch;
use crate::net;
use crate::output::{LineKind, OutputBuffer};
use crate::servers;

/// Display VPN + network status.
pub fn do_status(manager: &VpnManager) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("VPN Status");

    match &manager.session {
        Some(s) => {
            buf.ok(&format!("Connected via {}", s.protocol));
            buf.kv("Server", &s.display_name);
            buf.kv("Interface", &s.interface);
            if !s.provider.is_empty() {
                buf.kv("Provider", &s.provider);
            }
            if !s.country.is_empty() {
                buf.kv("Country", &s.country);
            }
            if !s.city.is_empty() {
                buf.kv("City", &s.city);
            }
        }
        None => {
            buf.warn("Not connected");
        }
    }

    buf.header("Network");

    let route = net::default_route();
    buf.kv("Default route", &route);

    let (tunneled, _) = net::is_tunneled(manager.session.as_ref().map(|s| s.interface.as_str()));
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

/// Connect to a server.
pub fn do_connect(manager: &mut VpnManager, config: &Config, target: ConnectTarget) -> OutputBuffer {
    let mut buf = OutputBuffer::new();

    let label = match &target {
        ConnectTarget::First => "first available".to_string(),
        ConnectTarget::Random => "random".to_string(),
        ConnectTarget::Country(cc) => format!("country: {cc}"),
        ConnectTarget::City(city) => format!("city: {city}"),
        ConnectTarget::Server(s) => format!("server: {s}"),
        ConnectTarget::Preferred => "preferred".to_string(),
    };

    buf.header(&format!("Connecting ({label})"));

    match manager.connect(&target, config) {
        Ok(session) => {
            buf.ok("Connected!");
            buf.kv("Server", &session.display_name);
            buf.kv("Interface", &session.interface);
            buf.kv("Protocol", &session.protocol.to_string());
            buf.kv("Public IP", &net::public_ip());

            // Honor the kill_switch_on_connect config flag. The user has
            // documented they want kill switch armed for every session;
            // up to here the flag was dead config.
            if config.general.kill_switch_on_connect {
                match killswitch::enable(&session) {
                    Ok(msg) => buf.ok(&msg),
                    Err(e) => buf.err(&format!("Kill switch enable failed: {e}")),
                }
            } else if config.general.warn_kill_switch && !killswitch::is_active() {
                // Surface the leak window before they notice the wrong way.
                // Prepend so the warning sits above "Connected!" in output.
                buf.lines.insert(
                    0,
                    LineKind::Warn(
                        "Kill switch is OFF — traffic will leak if the tunnel drops".into(),
                    ),
                );
            }
        }
        Err(e) => buf.err(&format!("Connection failed: {e}")),
    }

    buf
}

/// Disconnect from VPN.
pub fn do_disconnect(manager: &mut VpnManager) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("Disconnecting");

    match manager.disconnect() {
        Ok(()) => buf.ok("Disconnected."),
        Err(e) => buf.err(&format!("Disconnect failed: {e}")),
    }

    buf
}

/// List available countries from discovered servers.
pub fn do_countries(manager: &VpnManager) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("Available Countries");

    let list = servers::countries(&manager.servers);
    if list.is_empty() {
        buf.warn("No servers found. Drop .conf files into ~/.config/tuinnel/servers/");
    } else {
        for (code, _) in &list {
            let count = servers::find_by_country(&manager.servers, code).len();
            buf.kv(code, &format!("{count} server(s)"));
        }
        buf.blank();
        buf.dim(&format!("{} countries available", list.len()));
        buf.dim("Use: tuinnel cities <CODE> to see cities");
    }

    buf
}

/// List available cities in a country.
pub fn do_cities(manager: &VpnManager, country: &str) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header(&format!("Cities in {}", country.to_uppercase()));

    let list = servers::cities(&manager.servers, country);
    if list.is_empty() {
        buf.warn("No cities found for this country");
    } else {
        for city in &list {
            buf.plain(city);
        }
        buf.blank();
        buf.dim(&format!("{} cities available", list.len()));
        buf.dim("Use: tuinnel go <city> to connect");
    }

    buf
}

/// Show network information.
pub fn do_net(session: Option<&SessionInfo>) -> OutputBuffer {
    let mut buf = OutputBuffer::new();
    buf.header("Network Information");

    let nm = net::nm_status();
    buf.kv("NetworkManager", "");
    for line in nm.lines() {
        buf.indent(line);
    }

    buf.blank();
    buf.kv("WiFi SSID", &net::wifi_ssid());

    buf.blank();
    buf.kv("Routes", "");
    for line in net::all_routes().lines() {
        buf.indent(line);
    }

    buf.blank();
    let (tunneled, _) = net::is_tunneled(session.map(|s| s.interface.as_str()));
    if tunneled {
        buf.ok("Traffic appears tunneled through VPN");
    } else {
        buf.warn("Traffic is NOT going through a VPN tunnel");
    }

    buf
}
