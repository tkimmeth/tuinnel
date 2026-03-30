// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// main.rs — Entry point for pvpn
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Module tree:
//   pvpn (crate root = main.rs)
//   ├── output     → terminal color helpers
//   ├── config     → config loading
//   ├── vpn        → ProtonVPN CLI backend
//   ├── net        → network utilities
//   ├── commands   → shared command implementations
//   ├── doctor     → system diagnostics
//   ├── globe      → braille globe rendering + world map
//   ├── geo        → city coordinate lookup
//   ├── bandwidth  → live bandwidth monitoring
//   └── tui        → full-screen dashboard TUI

mod bandwidth;
mod commands;
mod config;
mod doctor;
mod geo;
mod globe;
mod net;
mod output;
mod privilege;
mod security;
mod tui;
mod util;
mod vpn;

use clap::{Parser, Subcommand};
use std::sync::Arc;

/// ProtonVPN Terminal Bot — control ProtonVPN from the terminal.
///
/// Run with no arguments to launch the interactive TUI dashboard.
#[derive(Parser)]
#[command(name = "pvpn", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show VPN connection status and network info.
    Status,

    /// Connect to ProtonVPN.
    Connect {
        /// Connect to the fastest available server.
        #[arg(short, long, conflicts_with_all = ["random", "country", "city", "server", "preferred"])]
        fastest: bool,

        /// Connect to a random server.
        #[arg(short, long, conflicts_with_all = ["fastest", "country", "city", "server", "preferred"])]
        random: bool,

        /// Connect by country code (e.g., US, NL, JP).
        #[arg(long, value_name = "CC", conflicts_with_all = ["fastest", "random", "city", "server", "preferred"])]
        country: Option<String>,

        /// Connect by city name (e.g., "New York").
        #[arg(long, conflicts_with_all = ["fastest", "random", "country", "server", "preferred"])]
        city: Option<String>,

        /// Connect to a specific server (e.g., US-NY#1).
        #[arg(short, long, value_name = "NAME", conflicts_with_all = ["fastest", "random", "country", "city", "preferred"])]
        server: Option<String>,

        /// Use preferred settings from config file.
        #[arg(short, long, conflicts_with_all = ["fastest", "random", "country", "city", "server"])]
        preferred: bool,
    },

    /// Disconnect from ProtonVPN.
    Disconnect,

    /// Kill switch management (on | off | status).
    Ks {
        /// Action to perform.
        #[arg(value_parser = ["on", "off", "status"])]
        action: String,
    },

    /// Show network information (SSID, routes, DNS, tunnel status).
    Net,

    /// List available countries.
    Countries,

    /// List available cities in a country.
    Cities {
        /// Country code (e.g., US, ES, JP) or full name.
        country: String,
    },

    /// Quick connect by city, country, or server name (fuzzy).
    ///
    /// Examples: pvpn go barcelona, pvpn go tokyo, pvpn go US
    Go {
        /// City name, country code, or server name.
        #[arg(num_args = 1..)]
        target: Vec<String>,
    },

    /// Run system diagnostics and check dependencies.
    Doctor,

    /// Launch the interactive TUI dashboard (same as running pvpn with no args).
    Menu,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Ensure directories exist
    let config_dir = config::config_dir();
    let state_dir = config::state_dir();
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&state_dir)?;

    // Load config
    let config = Arc::new(config::Config::load()?);

    // Set up file logging
    setup_logging(&config)?;
    log::info!("pvpn {} starting", env!("CARGO_PKG_VERSION"));

    // Initialize the VPN backend
    let vpn = Arc::new(vpn::ProtonVPN::new(&config));

    match cli.command {
        // No subcommand → launch TUI dashboard
        None => {
            tui::run(Arc::clone(&config), Arc::clone(&vpn))?;
        }

        Some(Commands::Status) => {
            commands::do_status(&vpn).print_all();
        }

        Some(Commands::Connect {
            fastest,
            random,
            country,
            city,
            server,
            preferred,
        }) => {
            let mode = if random {
                vpn::ConnectMode::Random
            } else if let Some(cc) = country {
                vpn::ConnectMode::Country(cc)
            } else if let Some(name) = server {
                vpn::ConnectMode::Server(name)
            } else if let Some(city_name) = city {
                vpn::ConnectMode::City(city_name)
            } else if preferred {
                vpn::ConnectMode::Preferred
            } else if fastest {
                vpn::ConnectMode::Fastest
            } else {
                match config.general.default_connect.as_str() {
                    "random" => vpn::ConnectMode::Random,
                    "preferred" => vpn::ConnectMode::Preferred,
                    _ => vpn::ConnectMode::Fastest,
                }
            };

            commands::do_connect(&vpn, &config, mode).print_all();
        }

        Some(Commands::Disconnect) => {
            commands::do_disconnect(&vpn).print_all();
        }

        Some(Commands::Ks { action }) => {
            commands::do_ks(&vpn, &action).print_all();
        }

        Some(Commands::Net) => {
            commands::do_net().print_all();
        }

        Some(Commands::Countries) => {
            commands::do_countries(&vpn).print_all();
        }

        Some(Commands::Cities { country }) => {
            commands::do_cities(&vpn, &country).print_all();
        }

        Some(Commands::Go { target }) => {
            let query = target.join(" ");
            let mode = if query.len() == 2 && query.chars().all(|c| c.is_ascii_uppercase()) {
                vpn::ConnectMode::Country(query)
            } else if query.contains('#') {
                vpn::ConnectMode::Server(query)
            } else {
                vpn::ConnectMode::City(query)
            };
            commands::do_connect(&vpn, &config, mode).print_all();
        }

        Some(Commands::Doctor) => {
            doctor::run(&vpn, &config).print_all();
        }

        Some(Commands::Menu) => {
            tui::run(Arc::clone(&config), Arc::clone(&vpn))?;
        }
    }

    Ok(())
}

/// Configure file-based logging.
fn setup_logging(config: &config::Config) -> anyhow::Result<()> {
    use simplelog::{ConfigBuilder, LevelFilter, WriteLogger};
    use std::fs::OpenOptions;

    let level = match config.logging.level.to_uppercase().as_str() {
        "DEBUG" => LevelFilter::Debug,
        "INFO" => LevelFilter::Info,
        "WARN" | "WARNING" => LevelFilter::Warn,
        "ERROR" => LevelFilter::Error,
        _ => LevelFilter::Info,
    };

    let log_path = config::log_file();
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;

    let log_config = ConfigBuilder::new()
        .set_time_format_rfc3339()
        .build();

    WriteLogger::init(level, log_config, file)?;

    log::debug!("Logging initialized: level={:?}, file={}", level, log_path.display());
    Ok(())
}
