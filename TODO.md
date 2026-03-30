# pvpn TODO

## Bugs

1. **`pvpn go us` broken** — lowercase country codes fall through to city mode and fail. The check requires uppercase (`query.chars().all(|c| c.is_ascii_uppercase())`). Fix: normalize to uppercase before checking length.

2. **Kill switch toggle in TUI does nothing** — `MenuAction::KillSwitch` hits the `_ => return` catch-all in `handle_selection` and silently cancels. The spinner shows but no background thread is spawned. Needs its own match arm.

3. **TUI hangs 0-5s on startup** — `gather_connection_info` calls `net::public_ip()` (blocking 5s curl) synchronously before the TUI draws. Fix: move initial gather to a background thread, show placeholder until ready.

4. **DNS leak check false-positives** — `check_dns_leak` hardcodes `10.2.0.1` as the only valid ProtonDNS address. systemd-resolved's stub `127.0.0.53` always triggers a false "DNS Leak" warning. Also fails for OpenVPN DNS or alternate Proton addresses. Fix: also accept `127.0.0.53` (when resolved forwards to Proton), `10.2.0.1`, and the IPv6 Proton DNS `2a07:b944::2:1`.

5. **Geo lookup returns wrong cities** — duplicate 2-letter codes ("MA" = Manchester/Madrid/Marseille/Manila) cause `find_map` to always return the first match. Globe marker shows wrong location for most of these. Fix: `lookup_city_code` must match both country AND city code, not just city code (it already does, but duplicate entries for the same country+code still collide — need to deduplicate the table).

## Missing Features

6. **No NetShield control** — `protonvpn config set netshield {off|malware-only|malware-ads-trackers}` exists. Add `pvpn netshield` subcommand and TUI menu option to cycle through modes.

7. **No P2P/SecureCore/Tor connect modes** — `caps.p2p`, `caps.sc`, `caps.tor` are detected during capability discovery but never wired to `ConnectMode` variants, CLI flags, or TUI options. Add `pvpn connect --p2p`, `--sc`, `--tor` and expose in TUI picker.

8. **No settings viewer/editor** — `protonvpn config list` shows 8 settings (VPN Accelerator, IPv6, Moderate NAT, port forwarding, custom DNS, anonymous crash reports, NetShield, kill switch). `protonvpn config set` can change them all. Add `pvpn config` to list and `pvpn config set <key> <value>` to change. Surface in TUI as a settings overlay.

9. **No account info** — `protonvpn info` shows username and plan. Add `pvpn info` and show plan/username in TUI connection panel.

10. **Autoconnect service ignores config** — `~/.config/systemd/user/pvpn-autoconnect.service` hardcodes `pvpn connect --preferred`. The `autoconnect.strategy` and `autoconnect.nm_wait_timeout` config keys are dead code. Fix: generate the service file from config, or have the service call a script that reads config.

## Minor

- `bandwidth.rs` uses `Vec::remove(0)` for rolling buffer — O(n) shift every second. Use `VecDeque`.
- Uptime shows "time since TUI opened", not actual VPN uptime. Could parse nmcli activation timestamp.
- `preferred.servers` config key exists but is never read anywhere.
- Legacy CLI city connect silently falls back to `--fastest` with no warning to the user.
- Bandwidth history not cleared on reconnect — sparkline shows stale data from previous session.
- Picker `scroll_offset` field is never written back after render — computed locally each frame.
