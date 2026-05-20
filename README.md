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

1. Download WireGuard `.conf` files from your VPN provider's website.

2. Drop them into `~/.config/tuinnel/servers/`:
```
~/.config/tuinnel/servers/
  mullvad/US/NewYork/us-nyc-001.conf
  protonvpn/NL/Amsterdam/nl-ams-01.conf
  selfhosted/US/Home/home.conf
```

Directory structure is optional but gives you country/city metadata:
`servers/<provider>/<country>/<city>/file.conf`

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

## Passwordless sudo (optional)

`wg-quick` needs root. To avoid password prompts:

```
sudo tee /etc/sudoers.d/tuinnel <<< '%wheel ALL=(root) NOPASSWD: /usr/bin/wg-quick, /usr/bin/wg, /usr/bin/nft'
```

## License

MIT
