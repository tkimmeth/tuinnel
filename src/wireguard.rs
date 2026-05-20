// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// wireguard.rs — WireGuard backend via wg-quick
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//
// Manages WireGuard tunnels using wg-quick (from wireguard-tools).
//
// The kernel interface name is always `tuinnel0`, regardless of which server
// .conf the user picked. We achieve this by *staging* the chosen .conf to a
// fixed path (`runtime_config_path()`) before calling `wg-quick up`, since
// wg-quick derives the interface name from the .conf filename stem. This
// decouples "which server am I on" (recorded in `SessionInfo.config_path`)
// from "what's the kernel calling the tunnel" — killswitch rules, bandwidth
// probes, and TUI status all reference a stable name.
//
// Session state is persisted to ~/.local/state/tuinnel/session.toml so that
// disconnect works after restart and probe_session has full metadata.

use crate::backend::{Protocol, SessionInfo, VpnBackend};
use crate::privilege::run_privileged;
use crate::servers::ServerEntry;
use crate::util::run_cmd;
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Fixed kernel interface name for every tuinnel WireGuard tunnel. The
/// staged-config trick in `stage_config` ensures `wg-quick` always creates
/// an interface of this name.
pub const RUNTIME_IFACE: &str = "tuinnel0";

pub struct WireGuardBackend;

impl VpnBackend for WireGuardBackend {
    fn connect(&self, server: &ServerEntry) -> Result<SessionInfo, String> {
        let source = &server.config_path;

        // Defense in depth: even though the long-term goal is a root-owned
        // helper (ADR-004), the cargo-run path goes straight from a user
        // file to root via wg-quick. Reject configs that would let wg-quick
        // execute arbitrary shell as root.
        validate_wg_config(source).map_err(|e| format!("Refusing unsafe config: {e}"))?;

        // Stage the chosen .conf at runtime_config_path() so wg-quick names
        // the interface `tuinnel0` rather than echoing the filename stem.
        let staged = stage_config(source)?;

        // Idempotent: tear down anything left on `tuinnel0` from a previous run.
        let _ = run_privileged("wg-quick", &["down", &staged.to_string_lossy()]);

        log::info!(
            "wg-quick up {} (source: {})",
            staged.display(),
            source.display()
        );
        let (ok, stdout, stderr) =
            run_privileged("wg-quick", &["up", &staged.to_string_lossy()]);

        if !ok {
            // Leave no private-key copy on disk if we never came up.
            let _ = unstage_config();
            let err = if !stderr.is_empty() { stderr } else { stdout };
            return Err(format!("wg-quick up failed: {err}"));
        }

        let dns_servers = parse_dns_from_config(source);

        let display_name = if !server.city.is_empty() {
            format!("{} - {}", server.country, server.city)
        } else if !server.country.is_empty() {
            server.country.clone()
        } else {
            server.name.clone()
        };

        let session = SessionInfo {
            // config_path tracks the *source* the user picked — used for
            // display, geo lookup, and "what server am I on" answers.
            // The actual file wg-quick reads is runtime_config_path().
            config_path: source.clone(),
            protocol: Protocol::WireGuard,
            interface: RUNTIME_IFACE.to_string(),
            pid: None,
            dns_servers,
            display_name,
            provider: server.provider.clone(),
            country: server.country.clone(),
            city: server.city.clone(),
            lat: server.lat,
            lon: server.lon,
            connected_at: SystemTime::now(),
        };

        save_session(&session);
        Ok(session)
    }

    fn disconnect(&self, session: &SessionInfo) -> Result<(), String> {
        let staged = crate::config::runtime_config_path();

        // Prefer the staged path — that's what brought the tunnel up, and
        // wg-quick down with a path is the most reliable form. Only fall
        // back to the interface name if the staged file was wiped out of
        // band (manual cleanup, abandoned run, etc.).
        let result = if staged.exists() {
            log::info!("wg-quick down {}", staged.display());
            run_privileged("wg-quick", &["down", &staged.to_string_lossy()])
        } else {
            log::info!(
                "wg-quick down {} (staged config missing)",
                session.interface
            );
            run_privileged("wg-quick", &["down", &session.interface])
        };

        let (ok, stdout, stderr) = result;

        if !ok {
            log::warn!("wg-quick down failed, forcing cleanup");
            let (del_ok, _, del_err) =
                run_privileged("ip", &["link", "delete", &session.interface]);
            let _ = run_privileged("resolvconf", &["-d", &session.interface]);

            if !del_ok {
                let err = if !stderr.is_empty() {
                    stderr
                } else if !del_err.is_empty() {
                    del_err
                } else {
                    stdout
                };
                clear_session();
                let _ = unstage_config();
                return Err(format!("Disconnect failed: {err}"));
            }
        }

        clear_session();
        let _ = unstage_config();
        Ok(())
    }

    fn probe_session(&self) -> Option<SessionInfo> {
        // First try loading persisted session from disk
        if let Some(session) = load_session() {
            // Verify the interface is actually still up
            let (ok, out, _) = run_cmd("ip", &["link", "show", &session.interface]);
            if ok && out.to_lowercase().contains("up") {
                return Some(session);
            }
            // Interface is gone — clean up stale session and any leftover
            // staged config (which still holds the previous tunnel's private key).
            clear_session();
            let _ = unstage_config();
        }

        // Only claim our own interface name. Any other wg-type tunnel on the
        // host (Proton's daemon, NetworkManager, a hand-run `wg-quick up`)
        // belongs to someone else — tuinnel won't try to manage or tear it
        // down. The previous behavior of grabbing "any wg interface" caused
        // disconnect to silently fail on foreign tunnels.
        let (ok, out, _) = run_cmd("ip", &["link", "show", RUNTIME_IFACE]);
        if !ok || !out.to_lowercase().contains("up") {
            return None;
        }

        Some(SessionInfo {
            config_path: PathBuf::new(),
            protocol: Protocol::WireGuard,
            interface: RUNTIME_IFACE.to_string(),
            pid: None,
            dns_servers: Vec::new(),
            display_name: RUNTIME_IFACE.to_string(),
            provider: String::new(),
            country: String::new(),
            city: String::new(),
            lat: None,
            lon: None,
            connected_at: SystemTime::now(),
        })
    }

    fn protocol(&self) -> Protocol {
        Protocol::WireGuard
    }
}

// ── Config staging ────────────────────────────────────────────────────────
//
// wg-quick names the kernel interface after the .conf's filename stem. To
// give every tunnel the same name (`tuinnel0`) regardless of which server the
// user picked, we copy the chosen .conf to a fixed runtime path before
// invoking wg-quick. The staged file is wiped on disconnect so the private
// key doesn't sit on disk between sessions.

/// Copy `source` to `runtime_config_path()`. Creates the runtime dir at 0700
/// if missing; writes the staged file at 0600. Returns the staged path on
/// success.
fn stage_config(source: &Path) -> Result<PathBuf, String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let runtime_dir = crate::config::runtime_dir();
    fs::create_dir_all(&runtime_dir)
        .map_err(|e| format!("create {}: {e}", runtime_dir.display()))?;
    let _ = fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700));
    crate::config::chown_to_invoking_user(&runtime_dir);

    let content = fs::read_to_string(source)
        .map_err(|e| format!("read {}: {e}", source.display()))?;

    let staged = crate::config::runtime_config_path();
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);

    let mut f = opts
        .open(&staged)
        .map_err(|e| format!("open {}: {e}", staged.display()))?;
    f.write_all(content.as_bytes())
        .map_err(|e| format!("write {}: {e}", staged.display()))?;

    // mode() only applies on create; re-enforce 0600 if the file already
    // existed with a wider mode.
    let _ = fs::set_permissions(&staged, fs::Permissions::from_mode(0o600));
    crate::config::chown_to_invoking_user(&staged);

    Ok(staged)
}

/// Best-effort delete of the staged config. Errors are returned for callers
/// that want to log them; the success path is `Ok(())` even if the file was
/// already absent.
fn unstage_config() -> std::io::Result<()> {
    let staged = crate::config::runtime_config_path();
    if staged.exists() {
        fs::remove_file(&staged)?;
    }
    Ok(())
}

// ── Config validation ─────────────────────────────────────────────────────
//
// Rejects WireGuard configs containing directives that wg-quick interprets
// as shell commands run as root. This is the privilege boundary that
// turns "drop a .conf and you get root" into "drop a .conf and tuinnel
// refuses to use it." See ADR-004.

const FORBIDDEN_KEYS: &[&str] = &[
    "postup",
    "postdown",
    "preup",
    "predown",
    "table",
    "fwmark",
    "saveconfig",
];

/// Validate a WireGuard `.conf` for directives that would let wg-quick run
/// arbitrary shell as root, or rewrite the user's main routing table.
///
/// Returns `Ok(())` if safe, or `Err(detail)` on the first forbidden line
/// found. The detail includes the line number and the offending key for
/// operator triage.
pub fn validate_wg_config(path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    for (lineno, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }

        let key = match line.split_once('=') {
            Some((k, _)) => k.trim().to_lowercase(),
            None => continue,
        };

        if FORBIDDEN_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "{}:{}: forbidden directive `{}` (wg-quick would interpret this)",
                path.display(),
                lineno + 1,
                key,
            ));
        }
    }

    Ok(())
}

// ── Session persistence ────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct PersistedSession {
    config_path: String,
    protocol: String,
    interface: String,
    #[serde(default)]
    dns_servers: Vec<String>,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    city: String,
    #[serde(default)]
    lat: Option<f64>,
    #[serde(default)]
    lon: Option<f64>,
    #[serde(default)]
    connected_at: u64,
}

fn session_path() -> PathBuf {
    crate::config::state_dir().join("session.toml")
}

fn save_session(session: &SessionInfo) {
    let epoch = session
        .connected_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let record = PersistedSession {
        config_path: session.config_path.to_string_lossy().into_owned(),
        protocol: "WireGuard".into(),
        interface: session.interface.clone(),
        dns_servers: session.dns_servers.clone(),
        display_name: session.display_name.clone(),
        provider: session.provider.clone(),
        country: session.country.clone(),
        city: session.city.clone(),
        lat: session.lat,
        lon: session.lon,
        connected_at: epoch,
    };

    let content = match toml::to_string_pretty(&record) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to serialize session: {e}");
            return;
        }
    };

    // session.toml contains hostnames, IPs, and the path to a config that
    // holds the WireGuard private key. 0600 keeps it out of `getent`-able
    // home directories on shared hosts.
    let path = session_path();
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true).mode(0o600);

    match opts.open(&path) {
        Ok(mut f) => {
            use std::io::Write;
            if let Err(e) = f.write_all(content.as_bytes()) {
                log::warn!("Failed to save session: {e}");
            }
        }
        Err(e) => log::warn!("Failed to open session file: {e}"),
    }

    // mode() applies only on create; enforce 0600 on existing files written
    // by older tuinnel versions with default umask.
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    crate::config::chown_to_invoking_user(&path);
}

fn load_session() -> Option<SessionInfo> {
    let path = session_path();
    let content = fs::read_to_string(&path).ok()?;

    let record: PersistedSession = match toml::from_str(&content) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("Failed to parse session.toml: {e}");
            return None;
        }
    };

    if record.interface.is_empty() {
        return None;
    }

    Some(SessionInfo {
        config_path: PathBuf::from(record.config_path),
        protocol: Protocol::WireGuard,
        interface: record.interface,
        pid: None,
        dns_servers: record.dns_servers,
        display_name: record.display_name,
        provider: record.provider,
        country: record.country,
        city: record.city,
        lat: record.lat,
        lon: record.lon,
        connected_at: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(record.connected_at),
    })
}

fn clear_session() {
    let _ = fs::remove_file(session_path());
}

// ── Config parsing ─────────────────────────────────────────────────────────

fn parse_dns_from_config(path: &std::path::Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(dns_value) = trimmed.strip_prefix("DNS") {
            let value = dns_value.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
            return value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    Vec::new()
}
