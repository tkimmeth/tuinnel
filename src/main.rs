// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// main.rs — Entry point for tuinnel
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mod backend;
mod bandwidth;
mod commands;
mod config;
mod doctor;
mod geo;
mod globe;
mod import;
mod net;
mod output;
mod privilege;
mod security;
mod servers;
mod killswitch;
mod tui;
mod util;
mod wireguard;

use backend::{ConnectTarget, VpnManager};
use wireguard::WireGuardBackend;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

/// tuinnel — universal VPN TUI for WireGuard and OpenVPN.
///
/// Run with no arguments to launch the interactive TUI dashboard.
#[derive(Parser)]
#[command(name = "tuinnel", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show VPN connection status and network info.
    Status,

    /// Connect to a VPN server.
    Connect {
        /// Connect to the first available server.
        #[arg(short, long, conflicts_with_all = ["random", "country", "city", "server", "preferred"])]
        first: bool,

        /// Connect to a random server.
        #[arg(short, long, conflicts_with_all = ["first", "country", "city", "server", "preferred"])]
        random: bool,

        /// Connect by country code (e.g., US, NL, JP).
        #[arg(long, value_name = "CC", conflicts_with_all = ["first", "random", "city", "server", "preferred"])]
        country: Option<String>,

        /// Connect by city name (e.g., "New York").
        #[arg(long, conflicts_with_all = ["first", "random", "country", "server", "preferred"])]
        city: Option<String>,

        /// Connect to a specific server config (by filename without extension).
        #[arg(short, long, value_name = "NAME", conflicts_with_all = ["first", "random", "country", "city", "preferred"])]
        server: Option<String>,

        /// Use preferred settings from config file.
        #[arg(short, long, conflicts_with_all = ["first", "random", "country", "city", "server"])]
        preferred: bool,
    },

    /// Disconnect from VPN.
    Disconnect,

    /// Kill switch management (on | off | status).
    Ks {
        #[arg(value_parser = ["on", "off", "status"])]
        action: String,
    },

    /// Show network information.
    Net,

    /// List discovered server configs.
    Servers,

    /// List available countries.
    Countries,

    /// List available cities in a country.
    Cities {
        /// Country code (e.g., US, NL, JP).
        country: String,
    },

    /// Quick connect by city, country, or server name.
    Go {
        #[arg(num_args = 1..)]
        target: Vec<String>,
    },

    /// Import .conf / .ovpn configs from a file, directory, .zip, or https:// URL.
    Import {
        /// Source path or https:// URL.
        source: PathBuf,

        /// Overwrite existing configs with the same stem.
        #[arg(long)]
        force: bool,

        /// Preview without writing anything to disk.
        #[arg(long)]
        dry_run: bool,

        /// Provider name override (skips inference).
        #[arg(long)]
        provider: Option<String>,

        /// Country code override (2-letter, will be uppercased).
        #[arg(long)]
        country: Option<String>,

        /// City override.
        #[arg(long)]
        city: Option<String>,
    },

    /// Run system diagnostics and check dependencies.
    Doctor,

    /// Launch the interactive TUI dashboard.
    Menu,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Ensure directories exist
    let config_dir = config::config_dir();
    let state_dir = config::state_dir();
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&state_dir)?;

    // Tighten state dir to 0700; it holds session.toml (with config path
    // pointers) and the log file. Other-readable is unnecessary.
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&state_dir, std::fs::Permissions::from_mode(0o700));
    }
    // If we were launched via sudo, hand the state and config dirs back to the
    // invoking user so later user-mode runs can still read them.
    config::chown_to_invoking_user(&state_dir);
    config::chown_to_invoking_user(&config_dir);

    // Load config
    let config = Arc::new(config::Config::load()?);

    // Set up file logging
    setup_logging(&config)?;
    log::info!("tuinnel {} starting", env!("CARGO_PKG_VERSION"));

    // Discover servers
    let servers_path = config::servers_dir(&config);
    std::fs::create_dir_all(&servers_path)?;
    let server_list = servers::discover(&servers_path);

    // Initialize WireGuard backend
    let backend: Arc<dyn backend::VpnBackend> = Arc::new(WireGuardBackend);
    let mut manager = VpnManager::new(backend, server_list);

    match cli.command {
        None => {
            tui::run(Arc::clone(&config), &mut manager)?;
        }

        Some(Commands::Status) => {
            commands::do_status(&manager).print_all();
        }

        Some(Commands::Connect {
            first,
            random,
            country,
            city,
            server,
            preferred,
        }) => {
            let target = if random {
                ConnectTarget::Random
            } else if let Some(cc) = country {
                ConnectTarget::Country(cc)
            } else if let Some(name) = server {
                ConnectTarget::Server(name)
            } else if let Some(city_name) = city {
                ConnectTarget::City(city_name)
            } else if preferred {
                ConnectTarget::Preferred
            } else if first {
                ConnectTarget::First
            } else {
                match config.general.default_connect.as_str() {
                    "random" => ConnectTarget::Random,
                    "preferred" => ConnectTarget::Preferred,
                    _ => ConnectTarget::First,
                }
            };

            commands::do_connect(&mut manager, &config, target).print_all();
        }

        Some(Commands::Disconnect) => {
            commands::do_disconnect(&mut manager).print_all();
        }

        Some(Commands::Ks { action }) => {
            let mut buf = output::OutputBuffer::new();
            buf.header(&format!("Kill Switch ({action})"));
            match action.as_str() {
                "on" => {
                    match &manager.session {
                        Some(session) => match killswitch::enable(session) {
                            Ok(msg) => buf.ok(&msg),
                            Err(e) => buf.err(&e),
                        },
                        None => buf.err("Not connected — connect first, then enable kill switch"),
                    }
                }
                "off" => match killswitch::disable() {
                    Ok(msg) => buf.ok(&msg),
                    Err(e) => buf.err(&e),
                },
                "status" => buf.ok(&killswitch::status_string()),
                _ => buf.err(&format!("Unknown action: {action}")),
            }
            buf.print_all();
        }

        Some(Commands::Net) => {
            commands::do_net(manager.session.as_ref()).print_all();
        }

        Some(Commands::Servers) => {
            let mut buf = output::OutputBuffer::new();
            buf.header("Discovered Servers");
            if manager.servers.is_empty() {
                buf.warn("No servers found. Drop .conf files into:");
                buf.plain(&format!("  {}", servers_path.display()));
            } else {
                for s in &manager.servers {
                    let loc = if !s.city.is_empty() {
                        format!("{} - {}", s.country, s.city)
                    } else if !s.country.is_empty() {
                        s.country.clone()
                    } else {
                        "unknown".into()
                    };
                    buf.kv(&s.name, &format!("{} [{}] ({})", loc, s.protocol, s.provider));
                }
                buf.blank();
                buf.dim(&format!("{} server(s) found", manager.servers.len()));
            }
            buf.print_all();
        }

        Some(Commands::Countries) => {
            commands::do_countries(&manager).print_all();
        }

        Some(Commands::Cities { country }) => {
            commands::do_cities(&manager, &country).print_all();
        }

        Some(Commands::Go { target }) => {
            let query = target.join(" ");
            let target = if query.len() == 2 && query.chars().all(|c| c.is_ascii_alphabetic()) {
                ConnectTarget::Country(query.to_uppercase())
            } else if query.contains('#') || query.contains('.') {
                ConnectTarget::Server(query)
            } else {
                ConnectTarget::City(query)
            };
            commands::do_connect(&mut manager, &config, target).print_all();
        }

        Some(Commands::Import { source, force, dry_run, provider, country, city }) => {
            let args = import::ImportArgs {
                source,
                force,
                dry_run,
                provider,
                country,
                city,
            };
            import::run(args, &config).print_all();
        }

        Some(Commands::Doctor) => {
            doctor::run(&manager, &config, manager.session.as_ref()).print_all();
        }

        Some(Commands::Menu) => {
            tui::run(Arc::clone(&config), &mut manager)?;
        }
    }

    Ok(())
}

fn setup_logging(config: &config::Config) -> anyhow::Result<()> {
    use simplelog::{ConfigBuilder, LevelFilter, WriteLogger};
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    let level = match config.logging.level.to_uppercase().as_str() {
        "DEBUG" => LevelFilter::Debug,
        "INFO" => LevelFilter::Info,
        "WARN" | "WARNING" => LevelFilter::Warn,
        "ERROR" => LevelFilter::Error,
        _ => LevelFilter::Info,
    };

    let log_path = config::log_file();
    // 0600: log lines may include endpoint hostnames, interface names, and
    // (in DEBUG) full command arguments. Not secrets, but not for `getent`.
    // mode() applies on create; set_permissions enforces it on existing files
    // left over from older versions of tuinnel that wrote with default umask.
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&log_path, std::fs::Permissions::from_mode(0o600));
    }
    config::chown_to_invoking_user(&log_path);

    let log_config = ConfigBuilder::new()
        .set_time_format_rfc3339()
        .build();

    WriteLogger::init(level, log_config, file)?;

    log::debug!("Logging initialized: level={:?}, file={}", level, log_path.display());
    Ok(())
}
