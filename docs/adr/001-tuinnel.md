# ADR-001: Refactor pvpn-bot into tuinnel — a provider-agnostic VPN TUI
   
    **Status:** Accepted
    **Date:** 2026-03-30
   
    ## Context
 pvpn-bot wraps ProtonVPN's Python CLI. The Python CLI breaks constantly across versions, causing hours of maintenance with no code change on our side. The TUI layer is solid and already mostly provider-agnostic.


 ## Decision
Rename to **tuinnel** (TUI + tunnel). Drop the ProtonVPN CLI dependency entirely. Use WireGuard (`wg-quick`) and OpenVPN (`openvpn`) directly.

- Users drop `.conf` / `.ovpn` files from any provider into `~/.config/tuinnel/servers/`
- All modules read from a canonical `SessionInfo` struct returned by the backend — no more hardcoded interface names or provider-specific patterns
- Kill switch via native nftables rules 
- v1 ships WireGuard-only; OpenVPN added after the session model is stable

## Consequences

 - Works with any VPN provider, not just Proton
 - No Python, no provider CLI; just standard Linux tools (`wg-quick`, `wg`, `nft`, `ip`)
 - Users do a one-time config download from their provider's website
 - Dynamic server list and load-based "fastest" selection are dropped
