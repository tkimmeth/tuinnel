# ADR-006: ProtonVPN Smart Routing is out of scope

**Status:** Accepted (informational)
**Date:** 2026-05-09
**Related:** ADR-001, ADR-005

## Context

A user may notice that connecting to a "Puerto Rico" ProtonVPN server (or UAE, Turkey, India, and other listed countries) results in traffic egressing through Miami (or Singapore, or the Netherlands). The endpoint IP in the `.conf` geolocates to the *physical* hosting location, not the *advertised* country. Example from a real `framework-PR-1.conf`:

- Advertised: Puerto Rico / San Juan
- `Endpoint = 138.199.50.97:51820`
- Geolocation: Miami, FL — DataCamp Limited, AS212238

This is **ProtonVPN Smart Routing**: for countries Proton judges hosting-risky, the listed location matches via WHOIS/registry IP attribution while the physical server lives in a "safe" jurisdiction (typically Miami, Singapore, or the Netherlands). It is documented behavior, not a leak: https://protonvpn.com/support/how-smart-routing-works.

The first time a user hits this, the symptom looks like:

- "tuinnel routes me through Miami no matter what."
- "My VPN is leaking — the country is wrong."

Neither is true. The `.conf` faithfully reports the endpoint Proton served. tuinnel does not modify or relabel endpoint IPs. The remedy, for users who care, is to pick a non–Smart-Routed country (most of Europe, Canada, Japan, Australia) — that's an upstream provider behavior tuinnel cannot and should not "fix."

## Decision

tuinnel does not detect, warn about, or "correct" Smart Routing. The endpoint IP, country, and city as configured are taken as ground truth. `geo.rs` resolves coordinates from the *labeled* city, not the physical IP, so the globe arc points at the country shown in the `.conf` — even when traffic egresses elsewhere.

This ADR exists so future changes don't accidentally introduce IP-geolocation lookups that "correct" Proton's labels — that would be hostile to providers' deliberate design choices and would require a network round-trip on every connect.

## Consequences

- Users who specifically want a non-Miami exit must select a non-Smart-Routed country. Documented in CLAUDE.md.
- The TUI globe arc may show a connection terminating in Puerto Rico while traffic is physically in Miami. We accept this as truthful to the `.conf`, not the physics.
- `security.rs` does not flag Smart-Routed connections as "leaks." The DNS server, tunnel interface, and `AllowedIPs` are still validated normally.
- If a future provider integration (e.g., a `tuinnel update <provider>` subcommand) chooses to display the physical egress location separately, that's allowed — but it should be additional information alongside the labeled location, not a replacement.
- Users who want the labeled-vs-physical mismatch surfaced can run `tuinnel net` after connecting; it shows the public IP, which they can geolocate themselves.
