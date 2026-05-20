# ADR-003: Native nftables kill switch in the `inet` family

**Status:** Accepted
**Date:** 2026-05-09
**Related:** ADR-001

## Context

A VPN client without a kill switch leaks. When the tunnel drops or fails to come up, packets that should ride the tunnel default-route through whatever the user's physical NIC is — leaking source IP, DNS queries, and traffic content. There are three viable implementations on Linux:

1. **Per-config `PostUp`/`PostDown` `iptables` rules** baked into the WireGuard `.conf` (the way many howtos teach it).
2. **A separate `iptables`/`iptables-restore` ruleset** managed independently of `wg-quick`.
3. **A separate `nftables` ruleset** managed independently of `wg-quick`.

We rejected option 1 because:

- It welds firewall policy into per-server config files. Every new `.conf` from a provider needs editing. Users won't.
- ADR-001 says `.conf` files come from providers untouched. PostUp shell hooks are the exact attack surface ADR-004 will close.
- Switching servers means tearing down rules and reinstating them per-config — kill-switch state should outlive any single connection.

We rejected option 2 because Arch and the wider Linux ecosystem are migrating off `iptables` to `nftables`. New systems may not have the `iptables` userspace at all (only `iptables-nft` shim). Building on a deprecated tool was a non-starter.

## Decision

tuinnel ships an opt-in nftables kill switch managed independently of any single connection.

- Single table: `inet tuinnel_killswitch` (`inet` family covers IPv4 and IPv6 in one ruleset).
- Two chains: `output` and `input`, both with priority 0 and policy `drop`.
- Allowed outbound traffic:
  - `oifname "lo"` (loopback)
  - `oifname "<session.interface>"` (the active tunnel)
  - `ip daddr <endpoint_ip>` (to the VPN endpoint, any port)
  - `udp dport { 67, 68 }` (DHCP — keep DHCP-bound networks usable)
  - `ct state established,related` (already-allowed flows)
- Inbound is the symmetric set.
- Endpoint resolution order: `wg show <iface> endpoints` (works while connected) → parse `Endpoint =` from the `.conf` (works pre-connect or while disconnected).
- Ruleset is rendered to a temp file and loaded via `nft -f`. Removal: `nft delete table inet tuinnel_killswitch`.
- Lifecycle is independent from connection. `tuinnel ks on` keeps blocking even if you disconnect — the user has to explicitly `ks off`. (This is the safer default for "I'm leaving the laptop overnight.")

## Consequences

- Provider configs stay untouched. Adding a new `.conf` is drag-and-drop.
- One ruleset per host instead of N (one per server).
- IPv4/IPv6 covered uniformly via `inet`.
- The kill switch will block traffic the user wants if their workflow needs LAN access (printer, NAS, dev VMs). The `LAN allow` option in `TODO.md` (allow `192.168.0.0/16`, `10.0.0.0/8`) addresses this when ready.
- Open issues we accept:
  - Loading via `/tmp/tuinnel-killswitch.nft` is a TOCTOU surface. Fix is to pipe via `nft -f -` stdin or use a root-owned `/run/tuinnel/` directory. Tracked in `TODO.md`.
  - The `kill_switch_on_connect` config flag exists but is not yet read by `commands::do_connect`. Until that's wired, kill switch is fully manual. Tracked in `TODO.md`.
  - The current ruleset only matches IPv4 endpoints (`ip daddr`). When a config uses an IPv6 endpoint we need to emit `ip6 daddr` instead. Tracked in `TODO.md`.
- The `nft` userspace must be installed; checked by `doctor.rs`. Listed as required (not optional) in `README.md` going forward.
