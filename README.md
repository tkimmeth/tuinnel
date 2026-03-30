# pvpn — ProtonVPN Terminal Bot

A terminal-first tool to control ProtonVPN on Arch Linux. Single binary, no runtime dependencies.

Built with Rust. Uses the official `protonvpn` CLI as the backend and `nmcli` for network awareness. Supports both the new official CLI (v0.1.x+) and the legacy `protonvpn-cli`.

## Install

```bash
git clone <this-repo> ~/Projects/pvpn-bot
cd ~/Projects/pvpn-bot
./install.sh
```

This builds the release binary and installs it to `~/.local/bin/pvpn`.

Make sure `~/.local/bin` is in your `PATH`:

```bash
export PATH="${HOME}/.local/bin:${PATH}"  # add to .zshrc or .bashrc
```

## Usage

### CLI Commands

```bash
pvpn status                    # VPN status, public IP, DNS, tunnel check
pvpn connect                   # Connect (uses default strategy from config)
pvpn connect --fastest         # Connect to fastest server
pvpn connect --random          # Connect to random server
pvpn connect --country US      # Connect by country code
pvpn connect --city "New York" # Connect by city name
pvpn connect --server US-NY#1  # Connect to specific server
pvpn connect --preferred       # Connect using preferred settings from config
pvpn disconnect                # Disconnect
pvpn ks on                     # Enable kill switch
pvpn ks off                    # Disable kill switch
pvpn ks status                 # Show kill switch state
pvpn net                       # Show network info (SSID, routes, DNS)
pvpn doctor                    # Run system diagnostics
pvpn menu                      # Launch interactive TUI
```

### Interactive TUI

`pvpn menu` launches a curses-style menu with vim keybindings (`j`/`k` to navigate, Enter to select, `q` to quit). Works over SSH.

## Configuration

Config file: `~/.config/pvpn-bot/config.toml`

```toml
[general]
cli_binary = ""              # auto-detect (protonvpn-cli or protonvpn)
default_connect = "fastest"  # fastest | random | preferred
kill_switch_on_connect = false
warn_kill_switch = true

[preferred]
country = "US"
# servers = ["US-NY#1"]

[autoconnect]
strategy = "fastest"
nm_wait_timeout = 30

[logging]
level = "INFO"               # DEBUG | INFO | WARN | ERROR
```

## Autoconnect on Login

A systemd user service is installed to `~/.config/systemd/user/pvpn-autoconnect.service`.

Enable it:

```bash
systemctl --user daemon-reload
systemctl --user enable --now pvpn-autoconnect.service
```

It waits for NetworkManager connectivity, then runs `pvpn connect --preferred`.

Edit `~/.config/pvpn-bot/config.toml` to change the autoconnect strategy and preferred country.

## Logs

Logs are written to `~/.local/state/pvpn-bot/pvpn.log`.

Set the log level in config:

```toml
[logging]
level = "DEBUG"
```

## Kill Switch

Kill switch management adapts to your CLI version:

- **New CLI (v0.1.x+):** Uses `protonvpn config set kill-switch standard/off`
- **Legacy CLI:** Tries `ks --on/--off` and common variants

`pvpn ks status` detects kill switch state by checking for the `pvpn-killswitch` NetworkManager interface. No guessing.

## WiFi Backend (iwd)

If you plan to switch from `wpa_supplicant` to `iwd`, `pvpn doctor` will print the exact steps. `pvpn` itself does not modify system files — it only reads network state.

## Test Plan

After installing, verify everything works:

```bash
# 1. Check dependencies and CLI detection
pvpn doctor

# 2. Check current state (should show "Disconnected")
pvpn status

# 3. Connect
pvpn connect --fastest

# 4. Verify connection (IP should change, tunnel should be detected)
pvpn status

# 5. Check kill switch
pvpn ks status

# 6. Disconnect
pvpn disconnect

# 7. Verify disconnected
pvpn status
```

## Roadmap

### v2.0 — Full Dashboard TUI

The current `pvpn menu` is a simple action list. The long-term vision is a persistent, full-screen TUI dashboard:

```
 ┌─ pvpn ──────────────────────────────────────────────────────────────────┐
 │                                                                         │
 │         . _ . _ .  _  . _ .        ┌─ Connection ──────────────────┐    │
 │      . '  _  o _ .' '. _  '.      │ Server:  US-NY#563            │    │
 │    .'  .'  '.'  '. '.  '. '.      │ City:    New York             │    │
 │   /  .'  ____     \   '.  \ \     │ IP:      31.13.189.232        │    │
 │  |  / ,-'    '-. *[NY]  \  | |    │ Proto:   WireGuard            │    │
 │  | | /  .--.  \ \  |  |  | | |    │ Uptime:  1h 23m               │    │
 │  |  \  |    |  / /  |  / |  |     │ Kill SW: ON                   │    │
 │   \  '-.____.-' /  .'  / /  /     └───────────────────────────────┘    │
 │    '.    ''''   .' .'  .'.'        ┌─ Bandwidth ───────────────────┐    │
 │      '. _ . _ .'  .' _ .'         │ ▁▂▃▅▇▅▃▂▁▂▃▅▇█▇▅▃  12.4 MB/s│    │
 │         ' _ '  . '_ '             │ ▁▁▂▂▃▃▂▁▁▂▂▃▃▅▅▃▂   4.1 MB/s│    │
 │                                    └───────────────────────────────┘    │
 │ ┌─ Actions ───────┐  ┌─ Network ────────────────────────────────────┐  │
 │ │ > Reconnect      │  │ SSID:    Tommyphone                        │  │
 │ │   Change Server  │  │ Route:   wlan0 -> pvpnksintrf0 -> proton0  │  │
 │ │   Disconnect     │  │ DNS:     10.2.0.1 (ProtonDNS)              │  │
 │ │   Kill Switch    │  │ Leak:    None detected                     │  │
 │ │   Settings       │  └────────────────────────────────────────────┘  │
 │ └──────────────────┘                                                   │
 │  j/k navigate  Enter select  r refresh  q quit           pvpn v2.0.0  │
 └─────────────────────────────────────────────────────────────────────────┘
```

**Planned features:**

- **Spinning ASCII globe** rendered with braille characters (`⠁⠂⠄⡀⢀`), rotating in real time, with a marker on your connected server location and a traced path from your real location to the exit node
- **Live bandwidth graph** — sparkline-style throughput monitor reading from `/sys/class/net/proton0/statistics/`
- **Server browser** — fuzzy-searchable list of all available countries/cities with latency indicators, pulled from `protonvpn countries` / `protonvpn cities`
- **DNS leak test** — built-in leak detection (query multiple DNS leak test endpoints, verify all responses route through ProtonDNS)
- **Connection history** — persistent log of past sessions (server, duration, bandwidth) stored in SQLite
- **Split-pane layout** — status dashboard always visible at top, action panel below, resizable
- **Notifications** — optional desktop notifications via `notify-send` on connect/disconnect/drop events
- **Multi-hop visualization** — when using Secure Core, show the full path (you -> entry country -> exit country) on the globe

### Implementation Notes

The globe rendering would use an equirectangular projection mapped to braille Unicode characters for ~2x4 sub-character resolution. Server coordinates can be derived from city names via a bundled lookup table (no network dependency). Rotation is frame-based at ~10 FPS using ratatui's event loop.

Bandwidth monitoring reads TX/RX bytes from the kernel's sysfs interface (`/sys/class/net/<iface>/statistics/rx_bytes`), diffs over time, and renders as a scrolling sparkline widget.

## Project Structure

```
pvpn-bot/
├── src/
│   ├── main.rs       # CLI parsing (clap), entry point
│   ├── config.rs     # Config struct, TOML loading, defaults
│   ├── vpn.rs        # ProtonVPN CLI backend, capability discovery
│   ├── net.rs        # Network utilities (nmcli, ip, curl)
│   ├── commands.rs   # Shared command implementations
│   ├── doctor.rs     # System diagnostics
│   ├── tui.rs        # ratatui interactive TUI
│   └── output.rs     # Colored terminal output helpers
├── config/
│   └── config.toml   # Default config template
├── systemd/
│   └── pvpn-autoconnect.service
├── install.sh
├── Cargo.toml
└── README.md
```

## Dependencies

Runtime: None (single static binary).

Build-time (Rust crates):
- `clap` — CLI argument parsing
- `ratatui` + `crossterm` — TUI rendering
- `serde` + `toml` — config deserialization
- `anyhow` — error handling
- `log` + `simplelog` — file logging
- `dirs` — XDG directory paths

## License

MIT
