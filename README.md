# tuinnel

A universal VPN TUI for WireGuard and OpenVPN. No Python, no provider CLIs — just `wg-quick` and a terminal.

Drop `.conf` files from any VPN provider into a directory, and tuinnel gives you a dashboard with a spinning globe, live bandwidth, security audit, and one-key connect/disconnect.

## Requirements

- `wireguard-tools` — `wg-quick`, `wg`
- `openresolv` — DNS management for `wg-quick`
- `nftables` — kill switch (optional)
- `curl`, `iproute2` — network checks

**Arch Linux:**
```
sudo pacman -S wireguard-tools openresolv nftables curl
```

**Debian/Ubuntu:**
```
sudo apt install wireguard-tools openresolv nftables curl
```

## Install

```
git clone https://github.com/tkimmeth/tuinnel
cd tuinnel
./install.sh
```

Or manually:
```
cargo build --release
cp target/release/tuinnel ~/.local/bin/
```

## Setup

1. Download WireGuard `.conf` files (or a zip of them) from your VPN provider's website.

2. Either let `tuinnel import` place them for you:
```
tuinnel import ~/Downloads/wireguard-configs.zip
tuinnel import ~/Downloads/us-nyc-001.conf --provider mullvad --country US --city "New York"
tuinnel import ~/configs/             # walk a directory recursively
tuinnel import https://example.com/configs.zip   # https only, 10 MB cap
```
…or drop them into `~/.config/tuinnel/servers/` by hand:
```
~/.config/tuinnel/servers/
  mullvad/US/NewYork/us-nyc-001.conf
  protonvpn/NL/Amsterdam/nl-ams-01.conf
  selfhosted/US/Home/home.conf
```

Directory structure is optional but gives you country/city metadata:
`servers/<provider>/<country>/<city>/file.conf`

`tuinnel import` rejects any config containing `PostUp`, `PostDown`, `PreUp`,
`PreDown`, `Table`, `FwMark`, or `SaveConfig` — these are the directives
`wg-quick` interprets as shell commands run as root. Use `--dry-run` to
preview the destination paths before writing.

3. Run `tuinnel doctor` to verify everything is set up.

## Usage

```
tuinnel                          # Launch TUI dashboard
tuinnel status                   # Show connection status
tuinnel connect                  # Connect (first available)
tuinnel connect --country US     # Connect to a US server
tuinnel connect --city "New York"
tuinnel connect --random
tuinnel connect --preferred      # Use config preferences
tuinnel disconnect               # Disconnect
tuinnel ks on|off|status         # Kill switch (nftables)
tuinnel servers                  # List discovered configs
tuinnel countries                # List available countries
tuinnel cities US                # List cities in a country
tuinnel go tokyo                 # Quick connect (fuzzy match)
tuinnel import <path|zip|url>    # Import .conf / .ovpn configs
tuinnel doctor                   # System diagnostics
```

## TUI

The dashboard shows:
- Spinning ASCII globe with connection arc
- Live bandwidth (RX/TX sparklines)
- Connection info (server, IP, protocol, uptime, kill switch)
- Network status (SSID, DNS, leak detection)
- Security audit (tunnel integrity, DNS leak, IPv6 leak, kill switch firewall)

Keybindings: `j/k` navigate, `Enter` select, `r` refresh, `q` quit.

## Server metadata

Country and city are inferred from directory structure. Override with `servers.toml`:

```toml
# ~/.config/tuinnel/servers/servers.toml

[metadata."us-nyc-001"]
country = "US"
city = "New York"
provider = "Mullvad"
lat = 40.71
lon = -74.01
```

## Kill switch

`tuinnel ks on` installs nftables rules that block all traffic except through the VPN tunnel. Traffic to the VPN endpoint IP and DHCP are allowed so the connection stays alive.

## Autoconnect

```
systemctl --user enable --now tuinnel-autoconnect.service
```

## Security model — read before granting `NOPASSWD`

`wg-quick` needs root because it brings up network interfaces, sets routes, and edits `/etc/resolv.conf`. The convenient way to skip password prompts is a `NOPASSWD` sudoers entry, but **`wg-quick` is a bash script that interprets `PostUp`, `PostDown`, `PreUp`, and `PreDown` lines as shell commands run as root**. Granting `NOPASSWD: /usr/bin/wg-quick` is therefore equivalent to granting `NOPASSWD: bash` to anything that can write a `.conf` file into `~/.config/tuinnel/servers/`. The same caveat applies to `NOPASSWD: /usr/bin/nft` — it permits `nft flush ruleset` (whole-host firewall wipe) and arbitrary ruleset loads.

**Threat model summary:**

- Configs in `~/.config/tuinnel/servers/` are a **privilege boundary**, not user data. Treat them like `/etc/sudoers.d/`.
- Only drop `.conf` files you obtained directly from your VPN provider's official portal.
- Inspect new `.conf` files for `PostUp`/`PostDown`/`PreUp`/`PreDown` directives before connecting. Legitimate provider configs do not need them.
- Keep your kernel current — CVE-2024-1086 (`nf_tables` UAF) is actively exploited and patched in Linux 5.15.149+ / 6.1.76+ / 6.6.15+ / 6.8+.

A safer alternative — a root-owned helper script that validates configs and confines `nft` to a fixed ruleset — is on the roadmap. See `docs/adr/004-privilege-model.md`.

### Passwordless sudo (optional, with caveats)

If you accept the trade-off above:

```
sudo tee /etc/sudoers.d/tuinnel <<< '%wheel ALL=(root) NOPASSWD: /usr/bin/wg-quick, /usr/bin/wg, /usr/bin/nft'
```

## License

MIT
