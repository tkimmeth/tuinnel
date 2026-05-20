# ADR-002: Persist active session state to disk

**Status:** Accepted
**Date:** 2026-05-09
**Supersedes:** N/A
**Related:** ADR-001

## Context

After ADR-001, `SessionInfo` is the canonical "what's connected" struct, owned in-memory by `VpnManager`. The original WireGuard backend (`probe_session`) reconstructed this struct at startup by:

1. Listing `wireguard`-type interfaces with `ip -o link show`.
2. Calling `wg show <iface> latest-handshakes` to verify the tunnel was active.
3. Returning a *minimal* `SessionInfo` — no provider, no country/city, no DNS, no geo, no original config path.

That broke three things:

- **Disconnect by config path failed across restarts.** `wg-quick down` works most reliably when given the original config file (it knows DNS, routes, hooks). After reboot we only had an interface name, so `wg-quick down <iface>` was best-effort and silently failed when `/etc/wireguard/<iface>.conf` did not exist.
- **The TUI globe and dashboard rendered blank** for any tunnel that survived a restart — no city, no coordinates, no provider name.
- **`security.rs` couldn't compare expected vs. actual DNS** because `SessionInfo.dns_servers` was empty.

We also can't put state under `/etc` (single-user TUI; no root-owned user state) or under `~/.config` (XDG forbids non-config there). `~/.local/state/tuinnel/` is the correct XDG location for "things that should survive a restart but aren't user-edited."

## Decision

On every successful `connect`, the WireGuard backend writes the full `SessionInfo` to `~/.local/state/tuinnel/session.toml`. On `probe_session`, the backend reads this file first and verifies the interface is still up via `ip link show`. If the file is missing or the interface is gone, the file is cleared and the backend falls back to the minimal interface-only detection. On `disconnect`, the file is removed.

Schema (TOML):

```
config_path = "..."
protocol    = "WireGuard"
interface   = "..."
dns_servers = ["10.2.0.1", "..."]
display_name = "..."
provider     = "..."
country      = "..."
city         = "..."
lat = 40.71
lon = -74.01
connected_at = 1747000000
```

The OpenVPN backend (when added) follows the same schema with `protocol = "OpenVPN"` and a populated `pid` field.

## Consequences

- `disconnect` after restart is reliable: we have the original `config_path`, so we hand it back to `wg-quick down`.
- TUI globe, security audit, and connection card all populate correctly across restarts.
- Failure modes: corrupt or attacker-edited `session.toml` can mislead `disconnect` and `probe_session`. Mitigations:
  - State directory and file should be `0700` / `0600` respectively (see ADR-004; not yet enforced — see `TODO.md`).
  - `load_session` should validate that `config_path` is inside the configured `servers_dir` and that `interface` matches `[a-zA-Z0-9_-]{1,15}` (Linux IFNAMSIZ).
  - Replace the hand-rolled line-based parser in `wireguard.rs:194-225` with `toml::from_str` into a `#[derive(Deserialize)]` struct.
- Adding any field to `SessionInfo` requires updating both `save_session` and `load_session` in lockstep. Switching to `serde` removes that coupling.
- We accept that an interface still being `UP` while `session.toml` is missing means we lose the rich metadata for that session — a small UX regression vs. always probing live, which we judge worth the simplicity.
