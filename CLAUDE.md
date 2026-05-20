# CLAUDE.md

Guidance for Claude when working in `/home/tommy/Projects/tuinnel`.

## What this project is

**tuinnel** is a universal VPN TUI written in Rust. It wraps `wg-quick` (and, eventually, `openvpn`) and presents a ratatui dashboard with a spinning ASCII globe, live bandwidth, security audit, and one-key connect/disconnect. Users drop provider `.conf` files into `~/.config/tuinnel/servers/<provider>/<country>/<city>/` and tuinnel does the rest.

It is the **Rust rewrite of `pvpn-bot`**. The old project wrapped ProtonVPN's Python CLI; the rewrite drops that dependency entirely. See `docs/adr/001-tuinnel.md` for the rationale. A frozen snapshot of the predecessor lives at `/home/tommy/Projects/pvpn-bot-backup-stepA/` — historical only, do not modify unless asked.

## Build, run, test

```
cargo build --release            # release binary → target/release/tuinnel
cargo run -- status              # quick smoke check
cargo run -- doctor              # dependency + leak self-test
./install.sh                     # full install to ~/.local/bin + systemd user unit
```

No test suite exists yet (`cargo test` runs zero). When adding tests, prefer integration tests under `tests/` that exercise the `commands::do_*` functions with a fake `VpnBackend` — those are the seams designed for it.

Logs go to `~/.local/state/tuinnel/tuinnel.log`. Set `[logging] level = "DEBUG"` in `~/.config/tuinnel/config.toml` for verbose tracing.

## Architecture (one-line per module)

| Module | Owns |
|---|---|
| `main.rs` | clap CLI, startup sequence, subcommand dispatch, TUI launch. |
| `backend.rs` | `Protocol`, `SessionInfo`, `VpnBackend` trait, `ConnectTarget`, `VpnManager`. |
| `wireguard.rs` | Only real backend impl; calls `wg-quick`; persists session to disk via serde+toml. Exposes `validate_wg_config(path)` (rejects `PostUp`/`PostDown`/`PreUp`/`PreDown`/`Table`/`FwMark`/`SaveConfig`). |
| `servers.rs` | Recursive `.conf`/`.ovpn` discovery, dir-structure metadata, `servers.toml` overrides, all selection helpers. |
| `commands.rs` | Shared `do_*` functions called identically from CLI and TUI. Reads `kill_switch_on_connect` / `warn_kill_switch` after a successful connect. |
| `import.rs` | `tuinnel import <source>` — single file, dir, `.zip`, or `https://` URL. Validates each `.conf` via `wireguard::validate_wg_config` before writing. |
| `config.rs` | TOML config loader + canonical paths (`config_dir`, `state_dir`, `servers_dir`, `log_file`). |
| `tui.rs` | ratatui dashboard, event loop, overlay state machine, background mpsc threads. |
| `killswitch.rs` | nftables ruleset (`inet tuinnel_killswitch` table) install/remove. Ruleset is piped through `nft -f -` stdin; no temp file. |
| `security.rs` | Five leak/integrity checks (tunnel, DNS leak, IPv6 leak, KS firewall, connectivity leak). |
| `privilege.rs` | Thin `sudo` wrapper around `util::run_cmd`. Adds `run_privileged_stdin` for ruleset piping. **No argument validation.** |
| `net.rs` | `curl`, `ip`, `nmcli`, `resolvectl`, `/etc/resolv.conf` probes. |
| `bandwidth.rs` | `/sys/class/net/<iface>/statistics/{rx,tx}_bytes` rolling history. |
| `globe.rs` | Pre-computed orthographic-projection world map; great-circle arc renderer. |
| `geo.rs` | ~140-city lat/lon lookup table; ProtonVPN `CC-CITY#N` parser (cosmetic). |
| `output.rs` | `OutputBuffer` rendered to ANSI stdout (CLI) or `Vec<Line>` (TUI overlay). |
| `doctor.rs` | Binary presence checks + dir checks + sudo-passwordless test + `security::full_audit`. |
| `util.rs` | `run_cmd` and `binary_exists`. **Uses bare program names — relies on `$PATH`.** |

## Connection lifecycle (canonical path)

```
tuinnel connect [--country US]
  main.rs:148-168     → ConnectTarget::Country("US")
  commands::do_connect → manager.connect()
  backend.rs:154-165   → resolve_target → servers::find_by_country → first match
  WireGuardBackend::connect (wireguard.rs:22-68)
    privilege::run_privileged("wg-quick", ["down", path])    # idempotent
    privilege::run_privileged("wg-quick", ["up",   path])
    parse_dns_from_config(path)
    save_session(&session)                                   # ~/.local/state/tuinnel/session.toml
  manager.session = Some(session)
```

`probe_session` (called at startup) tries `load_session()` first, verifies the interface is `UP`, and falls back to detecting any `wg`-type interface. The fallback returns minimal metadata (no provider/country/city/lat/lon).

## Server selection

`ConnectTarget` resolution in `backend.rs:111-151`:

| Target | Strategy |
|---|---|
| `First` | `servers.first()` after sort by (priority, country, city, name). |
| `Random` | Index by `SystemTime::now().subsec_nanos() % len`. |
| `Country(cc)` | Exact uppercase match → first. |
| `City(name)` | Case-insensitive `.contains()` → first. |
| `Server(name)` | Exact lowercase match on filename stem. |
| `Preferred` | `config.preferred.server` → `.city` → `.country` in order. |

`tuinnel go <fuzzy>`: 2 alpha chars → country, contains `#`/`.` → server name, else → city.

## State and config paths

| Path | Purpose |
|---|---|
| `~/.config/tuinnel/config.toml` | User config (general / preferred / autoconnect / logging sections). |
| `~/.config/tuinnel/servers/` | Server config tree (`<provider>/<country>/<city>/file.conf`). |
| `~/.config/tuinnel/servers/servers.toml` | Optional sidecar metadata overrides keyed by filename stem. |
| `~/.local/state/tuinnel/session.toml` | Active session (config_path, interface, DNS, geo, connected_at). Mode 0600. |
| `~/.local/state/tuinnel/tuinnel.log` | Application log. Mode 0600. |
| `/sys/class/net/<iface>/statistics/{rx,tx}_bytes` | Bandwidth counters. |

## Security model — read this before changing anything privileged

The threat model has one dominant edge: **`wg-quick` invoked as root with a user-controllable config path is a root-RCE primitive**. `wg-quick` is a bash script that interprets `PostUp = `, `PostDown = `, `PreUp = `, and `PreDown = ` as shell commands run as root. The README and `install.sh` currently recommend `%wheel ALL=(root) NOPASSWD: /usr/bin/wg-quick, /usr/bin/wg, /usr/bin/nft`. With that line in place, **any process running as the user that can drop a file in `~/.config/tuinnel/servers/`** (a malicious AUR package, a compromised browser extension, a cargo build script, a vendor `.conf` bundle with a backdoor) gets root with no password prompt.

Other concrete issues to keep in mind when editing:

| Severity | File:line | Issue |
|---|---|---|
| Critical | `wireguard.rs`, `README.md`, `install.sh` | `wg-quick` PostUp/PostDown shell hooks executed as root via NOPASSWD sudo. **Partially mitigated:** `wireguard.rs::validate_wg_config` rejects forbidden directives in-process; long-term fix is the root-owned helper from ADR-004. |
| Critical | `README.md`, `killswitch.rs` | Blanket `NOPASSWD: nft` allows `nft flush ruleset` (whole-host firewall wipe) and arbitrary ruleset load. |
| High | `killswitch.rs` | **Closed.** Ruleset is now piped via `nft -f -` stdin; no `/tmp/tuinnel-killswitch.nft` file. |
| High | `wireguard.rs::save_session` | **Closed.** `session.toml` written with `OpenOptions::mode(0o600)`; `set_permissions` on existing files. State dir set to `0700` in `main.rs`. |
| High | `wireguard.rs` (formerly hand-rolled) | **Closed.** `save_session` / `load_session` use `toml::to_string_pretty` / `toml::from_str` over a `#[derive(Serialize, Deserialize)]` struct. |
| High | `commands.rs:do_connect` | **Closed.** `config.general.kill_switch_on_connect` is now read; `warn_kill_switch` prepends a warning when KS is off. |
| High | (whole crate) | IPv6 leak is **detected** by `security.rs` but not **prevented** at connect time when the config lacks `AllowedIPs = ::/0`. |
| Medium | `util.rs:10,23` | Bare-name `Command::new` and `which` lookups → `$PATH` hijack. |
| Medium | `wireguard.rs:99-140`, `backend.rs:168-171` | Probe/disconnect race; manager has no `Mutex`. |
| Low | `main.rs::setup_logging` | **Closed.** Log file opened with `mode(0o600)` and `set_permissions` to enforce. |

When proposing fixes, the right shape is a small **root-owned helper** (`/usr/local/libexec/tuinnel-helper`) with an enum of allowed actions, called from a tightened sudoers entry — or polkit. The helper should refuse any `.conf` containing `PostUp/PostDown/PreUp/PreDown/Table/FwMark/SaveConfig` lines and confine the path to a known root-owned directory.

External CVEs that touch this stack:

- **CVE-2024-1086** (`nf_tables` UAF, "Flipping Pages") — actively exploited, full local root in 5.14–6.6. Patched in 5.15.149+ / 6.1.76+ / 6.6.15+ / 6.8+. Verify host kernel is current.
- **AUR supply-chain campaigns** — recurring (CHAOS RAT in `librewolf-fix-bin` / `firefox-patch-bin` / `zen-browser-patched-bin`, July 2025). If tuinnel is ever distributed via AUR, that becomes a relevant vector.
- No open advisories against `wireguard-tools`, `openresolv`, or `nftables` userspace at last check.

## Surprising behavior to know about

- **VPN routes through Miami even when the config says "Puerto Rico"** — that's ProtonVPN's [Smart Routing](https://protonvpn.com/support/how-smart-routing-works). PR/UAE/TR/IN servers physically egress through Miami/Singapore/NL. Not a leak, but US jurisdiction applies. Don't try to "fix" this in tuinnel.
- **`default_connect = "first"` + one config = all traffic to that one server.** The user must drop more `.conf` files to use other regions.
- **Kill switch auto-enable on connect** is now wired to `general.kill_switch_on_connect`. With the flag false but `warn_kill_switch` true, a warning is prepended to the connect output instead.
- **`probe_session` returns minimal metadata** if `session.toml` is missing — globe/geo/provider fields are blank.
- **OpenVPN backend is stubbed only.** `Protocol::OpenVPN` exists, the trait is generic, but no impl. Don't claim OpenVPN works.

## Cosmetic ProtonVPN references that are intentional

These are not bugs — leave them unless explicitly asked to remove:

- `src/geo.rs:5,25,149` — comments about ProtonVPN naming.
- `src/geo.rs:155-174` — `lookup_server` parses `CC-CITY#N` (a Proton convention) as a coordinate-lookup convenience.
- `src/backend.rs:50` — doc-comment example listing `"protonvpn"` as a provider directory.

## Conventions for edits in this repo

- Cite `file_path:line_number` when referencing code.
- Don't add error handling for impossible cases — internal code trusts internal code; only validate at boundaries (`.conf` files, `session.toml`, network responses, env / `$PATH`).
- Don't add comments that just restate the code. Add a comment only when the *why* is non-obvious (privilege boundaries, leak windows, surprising wg-quick semantics).
- Prefer extending the `OutputBuffer` typed-line system over raw `println!` so output works in both CLI and TUI overlay.
- New shelled-out binaries should go through `privilege::run_privileged` (if root needed) or `util::run_cmd` (if not). Use absolute paths.
- The `SessionInfo` struct is the single source of truth for "what's connected." Don't reach around it with ad-hoc `wg show` parsing.

## Where to look first for a given question

| Question | Module |
|---|---|
| "Why doesn't the killswitch turn on automatically?" | `commands.rs:do_connect` reads `kill_switch_on_connect` after `manager.connect()`; if false but `warn_kill_switch` is true a warn line is prepended. |
| "Why is bandwidth flat?" | `bandwidth.rs` — verify `/sys/class/net/<iface>/statistics/` is readable for the active iface |
| "Why does the globe show no arc?" | `tui.rs` (globe rendering) → `geo::lookup_city_name` → `SessionInfo.{lat,lon}` |
| "Why does disconnect fail after reboot?" | `wireguard.rs:probe_session` → `session.toml` (the persistence work on this branch) |
| "How does the security audit decide pass/fail?" | `security.rs:check_*` functions |
| "Where are the nftables rules built?" | `killswitch.rs:build_ruleset` |
| "How do I get configs onto disk without doing it by hand?" | `tuinnel import <path|zip|https-url>` → `import.rs::run`, validates each `.conf` via `wireguard::validate_wg_config`. |

## Active branch state

Branch `refactor/tuinnelRemake`: session persistence now uses serde+toml,
files write at 0600, state dir is 0700, log file is 0600. The kill-switch
ruleset is piped via `nft -f -` stdin (no `/tmp` file). `commands::do_connect`
honours `kill_switch_on_connect` and `warn_kill_switch`. New `tuinnel import`
subcommand (ADR-007) accepts `.conf`, `.ovpn`, directory, `.zip`, or
`https://` URL; every `.conf` is validated via `wireguard::validate_wg_config`
before being copied into `~/.config/tuinnel/servers/`.

## TODOs that matter for any future agent

`TODO.md` lists product TODOs (OpenVPN backend, AUR packaging, etc.). The security hardening list above is **not** in TODO.md yet — surface those before product work if security is the user's focus.
