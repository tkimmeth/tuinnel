# ADR-005: Provider-agnostic config discovery via directory hierarchy

**Status:** Accepted
**Date:** 2026-05-09
**Related:** ADR-001

## Context

ADR-001 dropped the ProtonVPN CLI dependency. That removed the source of server metadata: country, city, and provider used to come from `protonvpn-cli`'s API surface. The replacement architecture uses static `.conf` files dropped into a directory tree by the user — but raw `.conf` files don't carry metadata. WireGuard's format only has `[Interface]` and `[Peer]` sections; OpenVPN has `remote <host> <port>` and a soup of TLS material. Neither tells you "this is a Mullvad NYC server."

Three options for how to attach metadata:

1. **Filename convention** (`mullvad-us-nyc-001.conf`) — fragile, every provider names files differently, parsers diverge.
2. **Sidecar metadata file per `.conf`** — one extra file per server, doubles the maintenance burden.
3. **Directory hierarchy with a sidecar override file for edge cases** — what we picked.

## Decision

Server discovery walks `~/.config/tuinnel/servers/` recursively (`servers.rs::walk_dir`) and infers metadata from path components relative to the servers root:

- 0 components below root → all metadata empty.
- 1 component → `provider`.
- 2 components → `provider`, `country` (uppercased).
- 3+ components → `provider`, `country`, `city` (component index 2; deeper paths are flattened into the city slot).

Sidecar overrides via `~/.config/tuinnel/servers/servers.toml`, keyed by filename stem:

```toml
[metadata."us-nyc-001"]
country = "US"
city = "New York"
provider = "Mullvad"
lat = 40.71
lon = -74.01
priority = 10
```

Any field present in the sidecar wins over the directory inference.

`ConnectTarget` resolution (`backend.rs:111-151`) uses these fields:

- `--country US` → match exactly on `country` (uppercase), pick first after stable sort.
- `--city "New York"` → case-insensitive `.contains()` on `city`, pick first.
- `--server <stem>` → exact lowercase match on filename stem.
- `tuinnel go <fuzzy>` (`main.rs:234-242`) — 2 alpha chars → country, contains `#`/`.` → server stem, else city.

## Consequences

- The user does not edit anything to add a provider. Drop the `.conf` in the right directory and it appears.
- The directory layout becomes part of the API. We document `<provider>/<country>/<city>/file.conf` in README and don't change it without a new ADR.
- Country codes need to be uppercased to match the way most providers' downloads label them (`US`, `NL`, `JP`). The walk_dir uppercases automatically.
- City names are case-insensitive on lookup but stored as-typed for display. "New York" and "NEW YORK" both match `--city "new york"`.
- The "first" strategy depends on the sort order in `servers.rs:sort_by` — `(priority, country, city, name)`. Sidecar `priority` is the only knob users have to control "which server is first."
- Edge case: a `.conf` placed directly in `~/.config/tuinnel/servers/` has no metadata at all and only matches `--server <stem>` or `--first`. We document this as expected, not a bug.
- The sidecar file is the only writeable-by-tuinnel file in the servers tree. Discovery is otherwise read-only — important for the privilege model in ADR-004.
