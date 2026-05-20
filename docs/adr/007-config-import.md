# ADR-007: Config import via `tuinnel import <source>`

**Status:** Accepted
**Date:** 2026-05-09
**Related:** ADR-001, ADR-004, ADR-005

## Context

Today the only way to add a server is manual: download a `.conf` from the provider's website, figure out where it goes in `~/.config/tuinnel/servers/<provider>/<country>/<city>/`, drop it there, fix permissions if you remembered. Most providers ship configs as zip archives containing tens or hundreds of files; doing this by hand is friction the user wants gone.

The simplest fix is a one-shot `import` subcommand that takes whatever the provider gave you and lands it on disk in the layout `servers.rs::discover` expects. A more involved fix would be per-provider OAuth integrations (Mullvad device login, ProtonVPN session token via WebDriver, etc.). The latter is out of scope for this ADR — see "Out of scope" below.

## Decision

Add a `tuinnel import <source>` subcommand. The argument can be:

| Form | Behavior |
|---|---|
| `path/to/file.conf` | Validate, copy to `<servers_dir>/<provider>/<country>/<city>/<stem>.conf` (mode 0600). |
| `path/to/file.ovpn` | Same path, OpenVPN extension preserved. |
| `path/to/dir/` | Walk recursively, import every `.conf`/`.ovpn` found. |
| `path/to/configs.zip` | Extract entries to a 0700 temp dir, walk, import. Reject zip-slip names. |
| `https://provider.example/configs.zip` | Fetch (10 MB cap, https only), then dispatch as zip / single config. |
| `http://...` | Refused. https is the only supported scheme. |

Every entry, regardless of source, passes through `wireguard::validate_wg_config` (in the same Rust binary) before being written. That function rejects any line whose key (case-insensitive, after trimming) is `postup`, `postdown`, `preup`, `predown`, `table`, `fwmark`, or `saveconfig`. This is the same list documented in ADR-004 and is the privilege boundary that prevents a hostile config from reaching `wg-quick`.

Metadata inference, in order:

1. Explicit CLI overrides: `--provider`, `--country`, `--city`. If all three are supplied the import is fully non-interactive.
2. Filename heuristics:
   - Stem matches `mullvad-XX-...` → `provider=mullvad`, `country=XX`.
   - Stem contains `#` and starts with `XX-` → `provider=protonvpn`, `country=XX`.
   - Stem starts with `XX-` (any 2 alpha chars) → `country=XX`, provider blank.
3. Fallback default: `provider=imported`, country and city blank. The user can fix this later either by moving the file or by adding an entry to `servers.toml` (ADR-005). We deliberately do **not** prompt interactively on every miss — `tuinnel` is a single-shot CLI, not a wizard, and the directory layout is already self-documenting.

Flags:

- `--force` — overwrite an existing same-stemmed file.
- `--dry-run` — show what would be written without touching disk.
- `--provider`, `--country`, `--city` — non-interactive metadata override; useful for shell scripts (`for f in ./*.conf; do tuinnel import "$f" --provider mullvad --country US; done`).

Files are written with mode `0600` via `OpenOptions::mode`. WireGuard configs embed a private key; world-readable is unacceptable.

## Out of scope

- **Per-provider authentication.** No OAuth, no API tokens, no scraping the provider's account portal. Pulling that in resurrects exactly the dependency we removed in ADR-001 (a Python or browser-driven CLI per provider). Users continue to download the zip themselves; we just take the friction out of unzipping and placing the files.
- **Server selection within a provider's catalog.** `import` ingests whatever the user hands us. It does not re-query the provider for "the latest list."
- **Modifying or rewriting `.conf` files** to remove forbidden directives. Reject is the only action — silently mutating a config the user got from a provider would mask a hostile change made upstream.
- **Importing keys from a non-WireGuard format** (e.g., ProtonVPN's account-level UI export). Out of scope; users still use the provider's "download config" button.

## Consequences

- The user-facing onboarding becomes: download zip → `tuinnel import ~/Downloads/configs.zip`. No directory-tree manual labor.
- Defense in depth: even with the eventual root-owned helper from ADR-004, naked `cargo run` is also safe because `validate_wg_config` runs in-process before the file is ever passed to `wg-quick`.
- The CLI grows one subcommand. README adds one usage line. CLAUDE.md adds one row to "Where to look first."
- New deps: `ureq` (~50 KB compiled, sync, dependency-light) for HTTPS, `zip` for archive extraction. No `tokio`, no `reqwest`.
- The 10 MB download cap is a guard, not a policy. Provider config bundles are typically a few hundred KB; a 10 MB ceiling catches both runaway redirects and adversarial servers without rejecting legitimate use.
- We accept that the metadata inference heuristics will miss for less-common providers. Users in that case can either (a) pass `--provider`/`--country`/`--city`, (b) move the file after import, or (c) add a sidecar entry. None of these are worse than the manual flow they replace.
- `import` does not read or modify `servers.toml`. The sidecar override path stays exactly as ADR-005 defined it.
