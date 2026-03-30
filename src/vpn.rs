// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// vpn.rs — ProtonVPN CLI backend
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::config::Config;
use crate::util;
use std::process::Command;

#[derive(Debug, Clone)]
pub enum ConnectMode {
    Fastest,
    Random,
    Country(String),
    City(String),
    Server(String),
    Preferred,
}

/// Which generation of ProtonVPN CLI is installed.
#[derive(Debug, Clone, PartialEq)]
pub enum CliGeneration {
    /// New official CLI (v0.1.x+): uses `-h`, `config set kill-switch`, no `status` cmd.
    New,
    /// Old protonvpn-cli (Python): uses `--help`, `ks`, has `status` cmd.
    Legacy,
}

/// Discovered capabilities of the installed ProtonVPN CLI.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub connect: bool,
    pub disconnect: bool,
    pub status_cmd: bool,
    pub kill_switch: bool,
    pub random: bool,
    pub country: bool,
    pub city: bool,
    pub p2p: bool,
    pub sc: bool,
    pub tor: bool,
}

/// Output from running an external command.
struct CmdOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Interface to the ProtonVPN CLI binary.
pub struct ProtonVPN {
    pub binary: Option<String>,
    pub generation: CliGeneration,
    pub caps: Capabilities,
    pub cli_version: Option<String>,
}

impl ProtonVPN {
    /// Create a new ProtonVPN backend, auto-detecting the CLI binary.
    pub fn new(config: &Config) -> Self {
        let binary = Self::find_binary(config);

        let (generation, caps, cli_version) = match &binary {
            Some(bin) => Self::discover(bin),
            None => (CliGeneration::Legacy, Capabilities::default(), None),
        };

        Self {
            binary,
            generation,
            caps,
            cli_version,
        }
    }

    // ── Binary Detection ────────────────────────────────────────────────

    fn find_binary(config: &Config) -> Option<String> {
        let configured = &config.general.cli_binary;
        if !configured.is_empty() {
            if which(configured) {
                return Some(configured.clone());
            }
            log::warn!("Configured binary '{}' not found in PATH", configured);
        }

        ["protonvpn-cli", "protonvpn"]
            .iter()
            .find(|name| which(name))
            .map(|name| {
                log::info!("Found ProtonVPN CLI: {}", name);
                name.to_string()
            })
    }

    // ── Capability Discovery ────────────────────────────────────────────

    fn discover(binary: &str) -> (CliGeneration, Capabilities, Option<String>) {
        let mut caps = Capabilities::default();
        let mut version = None;

        // Try -h first (new CLI), then --help (old CLI)
        let help_out = run(binary, &["-h"]);
        let help_text = format!("{}\n{}", help_out.stdout, help_out.stderr);
        let h = help_text.to_lowercase();

        // Detect generation:
        //   New CLI has "signin/signout" and "config" subcommands
        //   Old CLI has "login/logout" and "ks" subcommands
        let generation = if h.contains("signin") || h.contains("config") {
            CliGeneration::New
        } else {
            CliGeneration::Legacy
        };

        log::info!("Detected CLI generation: {:?}", generation);

        // Parse version from banner (new CLI shows it in the -h ASCII art)
        // Look for a version pattern like "0.1.6" at end of a line
        for line in help_text.lines() {
            let trimmed = line.trim();
            // The new CLI prints version at end of the ASCII art banner
            if let Some(ver) = trimmed.split_whitespace().last() {
                let starts_with_digit = ver.starts_with(|c: char| c.is_ascii_digit());
                if starts_with_digit && ver.contains('.') {
                    version = Some(ver.to_string());
                    break;
                }
            }
        }

        // If no version from banner, try --version (old CLI)
        if version.is_none() {
            let v = run(binary, &["--version"]);
            if v.success && !v.stdout.is_empty() {
                version = v.stdout.lines().next().map(|l| l.trim().to_string());
            }
        }

        // Detect main commands
        caps.connect = h.contains("connect");
        caps.disconnect = h.contains("disconnect");
        caps.status_cmd = h.contains("status"); // Old CLI only

        // Kill switch detection depends on generation
        match generation {
            CliGeneration::New => {
                // New CLI: check `config set -h` for kill-switch
                let cfg_help = run(binary, &["config", "set", "-h"]);
                let ch = format!("{}\n{}", cfg_help.stdout, cfg_help.stderr).to_lowercase();
                caps.kill_switch = ch.contains("kill-switch");
            }
            CliGeneration::Legacy => {
                for ks in &["ks", "killswitch", "kill-switch"] {
                    if h.contains(ks) {
                        caps.kill_switch = true;
                        break;
                    }
                }
            }
        }

        // Detect connect options
        let connect_help = match generation {
            CliGeneration::New => run(binary, &["connect", "-h"]),
            CliGeneration::Legacy => {
                let out = run(binary, &["connect", "--help"]);
                if out.success {
                    out
                } else {
                    run(binary, &["c", "--help"])
                }
            }
        };
        let ch = format!("{}\n{}", connect_help.stdout, connect_help.stderr).to_lowercase();

        caps.random = ch.contains("--random");
        caps.country = ch.contains("--country") || ch.contains("--cc");
        caps.city = ch.contains("--city");
        caps.p2p = ch.contains("--p2p");
        caps.sc = ch.contains("--sc") || ch.contains("--securecore");
        caps.tor = ch.contains("--tor");

        log::info!("Capabilities: {:?}", caps);

        (generation, caps, version)
    }

    // ── Status ──────────────────────────────────────────────────────────
    //
    // The new CLI (v0.1.x) has no `status` command. We detect connection
    // state by checking nmcli for active VPN connections.

    pub fn status(&self) -> Result<String, String> {
        let bin = self.require_binary()?;

        // Old CLI: try `status` / `s` commands directly
        if self.generation == CliGeneration::Legacy {
            for sub in &["status", "s"] {
                let out = run(&bin, &[sub]);
                if out.success && !out.stdout.is_empty() {
                    return Ok(out.stdout);
                }
            }
        }

        // Both generations: build status from nmcli VPN connection info
        self.build_status_from_nmcli()
    }

    fn build_status_from_nmcli(&self) -> Result<String, String> {
        // Check for active VPN connections via nmcli
        let (ok, out, _) =
            util::run_cmd("nmcli", &["-t", "-f", "TYPE,NAME,DEVICE", "connection", "show", "--active"]);

        if !ok {
            return Err("Could not query NetworkManager".into());
        }

        // Look for VPN or tun/wg connections
        let mut vpn_lines = Vec::new();
        for line in out.lines() {
            let lower = line.to_lowercase();
            if lower.contains("vpn")
                || lower.contains("tun")
                || lower.contains("wg")
                || lower.contains("proton")
            {
                vpn_lines.push(line.to_string());
            }
        }

        if vpn_lines.is_empty() {
            Ok("Status: Disconnected\nNo active VPN connection".to_string())
        } else {
            let mut status = String::from("Status: Connected\n");
            for line in &vpn_lines {
                // Format: TYPE:NAME:DEVICE
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 3 {
                    status.push_str(&format!(
                        "  Type: {}  Name: {}  Device: {}\n",
                        parts[0], parts[1], parts[2]
                    ));
                } else {
                    status.push_str(&format!("  {line}\n"));
                }
            }
            Ok(status)
        }
    }

    /// One-line status for display in TUI header.
    pub fn status_oneline(&self) -> String {
        match self.status() {
            Ok(text) => {
                text.lines()
                    .find(|l| {
                        let lo = l.to_lowercase();
                        lo.contains("status") || lo.contains("connect")
                    })
                    .unwrap_or_else(|| text.lines().next().unwrap_or("Unknown"))
                    .trim()
                    .to_string()
            }
            Err(e) => e,
        }
    }

    // ── Connect ─────────────────────────────────────────────────────────

    pub fn connect_full(
        &self,
        mode: &ConnectMode,
        config: &Config,
    ) -> Result<String, String> {
        let bin = self.require_binary()?;

        let resolved = match mode {
            ConnectMode::Preferred => ConnectMode::Country(config.preferred.country.clone()),
            other => other.clone(),
        };

        let mut last_err = String::from("Connection failed");

        // Build command based on CLI generation
        let attempts = match self.generation {
            CliGeneration::New => self.new_cli_connect_cmds(&resolved),
            CliGeneration::Legacy => self.legacy_connect_cmds(&resolved),
        };

        for cmd_args in &attempts {
            let str_args: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            log::info!("Connect: {} {}", bin, str_args.join(" "));
            let out = run(&bin, &str_args);

            if out.success {
                return Ok(if out.stdout.is_empty() {
                    "Connected".into()
                } else {
                    out.stdout
                });
            }

            last_err = if !out.stderr.is_empty() {
                out.stderr
            } else if !out.stdout.is_empty() {
                out.stdout
            } else {
                "Connection failed".into()
            };
            log::debug!("Attempt failed: {}", last_err);
        }

        Err(last_err)
    }

    /// Build connect commands for the new official CLI (v0.1.x+).
    fn new_cli_connect_cmds(&self, mode: &ConnectMode) -> Vec<Vec<String>> {
        let mut cmds = Vec::new();
        let s = |s: &str| s.to_string(); // shorthand

        match mode {
            // New CLI: bare `protonvpn connect` connects to fastest server.
            ConnectMode::Fastest => {
                cmds.push(vec![s("connect")]);
            }
            ConnectMode::Random => {
                cmds.push(vec![s("connect"), s("--random")]);
            }
            ConnectMode::Country(cc) => {
                cmds.push(vec![s("connect"), s("--country"), cc.clone()]);
            }
            ConnectMode::City(city) => {
                cmds.push(vec![s("connect"), s("--city"), city.clone()]);
            }
            ConnectMode::Server(name) => {
                cmds.push(vec![s("connect"), name.clone()]);
            }
            ConnectMode::Preferred => unreachable!(), // resolved before this
        }

        cmds
    }

    /// Build connect commands for the old Python CLI (protonvpn-cli).
    fn legacy_connect_cmds(&self, mode: &ConnectMode) -> Vec<Vec<String>> {
        let mut cmds = Vec::new();
        let s = |s: &str| s.to_string();

        // Try both long and short subcommand forms
        for sub in &["connect", "c"] {
            let mut args = vec![s(sub)];
            match mode {
                ConnectMode::Fastest => args.push(s("--fastest")),
                ConnectMode::Random => args.push(s("--random")),
                ConnectMode::Country(cc) => {
                    args.push(s("--cc"));
                    args.push(cc.clone());
                }
                ConnectMode::City(_) => {
                    // Old CLI doesn't support --city; fall back to fastest
                    args.push(s("--fastest"));
                }
                ConnectMode::Server(name) => {
                    args.push(name.clone());
                }
                ConnectMode::Preferred => unreachable!(),
            }
            cmds.push(args);
        }

        cmds
    }

    // ── Disconnect ──────────────────────────────────────────────────────

    pub fn disconnect(&self) -> Result<String, String> {
        let bin = self.require_binary()?;

        for sub in &["disconnect", "d"] {
            let out = run(&bin, &[sub]);
            if out.success {
                return Ok(if out.stdout.is_empty() {
                    "Disconnected".into()
                } else {
                    out.stdout
                });
            }
        }
        Err("Disconnect failed".into())
    }

    // ── Kill Switch ─────────────────────────────────────────────────────

    pub fn kill_switch(&self, action: &str) -> Result<String, String> {
        let bin = self.require_binary()?;

        if !self.caps.kill_switch {
            return Err(
                "Kill switch not detected in your ProtonVPN CLI.\n  \
                 Run 'pvpn doctor' to check capabilities."
                    .to_string(),
            );
        }

        match self.generation {
            CliGeneration::New => self.new_cli_kill_switch(&bin, action),
            CliGeneration::Legacy => self.legacy_kill_switch(&bin, action),
        }
    }

    /// Kill switch for new CLI: `protonvpn config set kill-switch {standard|off}`
    ///
    /// Status is determined by checking iptables for actual blocking rules,
    /// NOT the nmcli pvpn-killswitch profile (which can linger after disable).
    fn new_cli_kill_switch(&self, bin: &str, action: &str) -> Result<String, String> {
        let state_file = crate::config::state_dir().join("ks-state");

        match action {
            "on" => {
                let out = run(bin, &["config", "set", "kill-switch", "standard"]);
                if out.success {
                    let _ = std::fs::write(&state_file, "on");
                    Ok("Kill switch enabled (standard mode)".into())
                } else {
                    Err(out.stderr)
                }
            }
            "off" => {
                let out = run(bin, &["config", "set", "kill-switch", "off"]);
                if out.success {
                    let _ = std::fs::write(&state_file, "off");
                    Ok("Kill switch disabled".into())
                } else {
                    Err(out.stderr)
                }
            }
            "status" => {
                // Primary: check iptables for actual DROP/REJECT rules referencing VPN
                let (ipt_ok, ipt_out, _) = util::run_cmd("iptables", &["-S"]);
                if ipt_ok {
                    let has_blocks = ipt_out.lines().any(|l| {
                        let lo = l.to_lowercase();
                        (lo.contains("drop") || lo.contains("reject"))
                            && (lo.contains("proton") || lo.contains("pvpn"))
                    });
                    if has_blocks {
                        return Ok("Kill switch: ON (firewall rules active)".into());
                    }
                }

                // Fallback: check our own state file (tracks last set action)
                if let Ok(state) = std::fs::read_to_string(&state_file) {
                    if state.trim() == "on" {
                        return Ok("Kill switch: ON".into());
                    }
                }

                Ok("Kill switch: OFF".into())
            }
            _ => Err(format!("Unknown kill switch action: {action}")),
        }
    }

    /// Kill switch for old CLI: tries `ks --on/--off` etc.
    fn legacy_kill_switch(&self, bin: &str, action: &str) -> Result<String, String> {
        let ks_cmds = ["ks", "killswitch", "kill-switch"];
        let flags = match action {
            "on" => &["--on", "--enable", "enable", "on"][..],
            "off" => &["--off", "--disable", "disable", "off"][..],
            "status" => &["--status", "status", ""][..],
            _ => return Err(format!("Unknown kill switch action: {action}")),
        };

        for ks in &ks_cmds {
            for flag in flags {
                let args: Vec<&str> = if flag.is_empty() {
                    vec![ks]
                } else {
                    vec![ks, flag]
                };
                let out = run(bin, &args);
                if out.success {
                    let msg = if out.stdout.is_empty() {
                        format!("Kill switch {action}")
                    } else {
                        out.stdout
                    };
                    return Ok(msg);
                }
            }
        }

        Err(format!("Kill switch '{action}' failed"))
    }

    /// Get the CLI version string.
    pub fn version(&self) -> Option<String> {
        self.cli_version.clone()
    }

    /// Return a summary of capabilities for display.
    pub fn caps_summary(&self) -> Vec<(&str, bool)> {
        vec![
            ("connect", self.caps.connect),
            ("disconnect", self.caps.disconnect),
            ("status (native)", self.caps.status_cmd),
            ("kill_switch", self.caps.kill_switch),
            ("random", self.caps.random),
            ("country", self.caps.country),
            ("city", self.caps.city),
            ("p2p", self.caps.p2p),
            ("secure core", self.caps.sc),
            ("tor", self.caps.tor),
        ]
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn require_binary(&self) -> Result<String, String> {
        self.binary.clone().ok_or_else(|| {
            "ProtonVPN CLI not found. Run 'pvpn doctor' for help.".to_string()
        })
    }

    /// List available countries from `protonvpn countries`.
    /// Returns Vec of (display_name, country_code).
    pub fn list_countries(&self) -> Result<Vec<(String, String)>, String> {
        let bin = self.require_binary()?;
        let out = run(&bin, &["countries", "list"]);
        if !out.success {
            return Err(format!("countries command failed: {}", out.stderr));
        }
        let mut results = Vec::new();
        for line in out.stdout.lines().skip(2) {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('-') {
                continue;
            }
            // Format: "Country Name            CC"
            let parts: Vec<&str> = trimmed.rsplitn(2, char::is_whitespace).collect();
            if parts.len() == 2 {
                let code = parts[0].trim().to_string();
                let name = parts[1].trim().to_string();
                if code.len() == 2 && code.chars().all(|c| c.is_ascii_uppercase()) {
                    results.push((name, code));
                }
            }
        }
        Ok(results)
    }

    /// List available cities for a country from `protonvpn cities --country <cc>`.
    /// Returns Vec of unique city names.
    pub fn list_cities(&self, country_code: &str) -> Result<Vec<String>, String> {
        let bin = self.require_binary()?;
        let out = run(&bin, &["cities", "list", country_code]);
        if !out.success {
            return Err(format!("cities command failed: {}", out.stderr));
        }
        let mut cities = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for line in out.stdout.lines().skip(2) {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('-') {
                continue;
            }
            // Format: "City Name       Features"
            // Take everything before two+ spaces (the column gap)
            let city = if let Some(idx) = trimmed.find("  ") {
                trimmed[..idx].trim()
            } else {
                trimmed
            };
            if !city.is_empty() && seen.insert(city.to_string()) {
                cities.push(city.to_string());
            }
        }
        cities.sort();
        Ok(cities)
    }
}

// ── Free Functions ──────────────────────────────────────────────────────────

/// Stderr patterns to suppress — harmless noise from the ProtonVPN CLI's
/// LocalAgent system and Python logging internals.
const STDERR_NOISE: &[&str] = &[
    "localagenterror", "local_agent", "localagent",
    "invalidcertificate", "notvalidforname", "tokio(custom",
    "concurrent.futures", "traceback (most recent",
    "file \"/usr/lib/python", "raise self._exception",
    "future.result()", "__get_result", "_invoke_callbacks",
    "_handle_future_result", "_start_local_agent",
    "__attempt_to_connect", "await self._agent", "await self.__attempt",
];

/// Stdout lines to suppress — informational noise from the CLI.
const STDOUT_NOISE: &[&str] = &[
    "server list is outdated", "this may take a moment",
];

fn filter_lines(raw: &str, patterns: &[&str]) -> String {
    raw.lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            !patterns.iter().any(|pat| lower.contains(pat))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Run an external command and capture output.
fn run(program: &str, args: &[&str]) -> CmdOutput {
    log::debug!("exec: {} {}", program, args.join(" "));

    match Command::new(program).args(args).output() {
        Ok(output) => {
            let raw_stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let raw_stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            CmdOutput {
                success: output.status.success(),
                stdout: filter_lines(&raw_stdout, STDOUT_NOISE),
                stderr: filter_lines(&raw_stderr, STDERR_NOISE),
            }
        }
        Err(e) => CmdOutput {
            success: false,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

/// Check if a binary exists in PATH.
fn which(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

