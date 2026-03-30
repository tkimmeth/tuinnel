# tuinnel TODO

## Next up

- OpenVPN backend (`openvpn --config --daemon`, PID tracking, auth)
- `tuinnel import <path>` — guided config import with metadata prompts
- Populate `probe_session` with server metadata (match interface name to ServerEntry)
- AUR package (PKGBUILD)

## Improvements

- VecDeque for bandwidth history (currently Vec::remove(0) is O(n))
- Parse actual VPN uptime from wg show latest-handshakes
- Clear bandwidth history on reconnect
- Geo coordinate lookup from servers.toml lat/lon into globe marker
- LAN allow option in kill switch (allow 192.168.0.0/16, 10.0.0.0/8)

## Known issues

- `wg show` needs root — `probe_session` may fail without passwordless sudo
- Autoconnect service needs testing with the new binary name
