# tuinnel TODO

## Security hardening (priority — see `CLAUDE.md` and `docs/adr/004-privilege-model.md`)

- **[Critical] Replace `NOPASSWD: wg-quick, nft` with a root-owned helper.**
  - Ship `/usr/local/libexec/tuinnel-helper` with an enum of actions (`up`, `down`, `ks-up`, `ks-down`).
  - Helper rejects any `.conf` containing `PostUp`/`PostDown`/`PreUp`/`PreDown`/`Table`/`FwMark`/`SaveConfig`.
  - Helper canonicalizes the path and refuses anything outside the configured `servers_dir`.
  - Sudoers becomes `%wheel ALL=(root) NOPASSWD: /usr/local/libexec/tuinnel-helper`.
  - Or — preferred — switch to polkit (`pkexec`) so invocations are logged.
- **[High] Prevent IPv6 leaks at connect time** when `AllowedIPs` lacks `::/0` and the host has global IPv6. (Detection-only today in `security.rs`.)
- **[Medium] Use absolute paths for `sudo`/`wg`/`ip`/`nft`/`curl`/`which`** in `util.rs` to close `$PATH` hijack.
- **[Medium] Manager-level `Mutex`** to remove the probe/disconnect race.
- **[Low] DNS verification post-connect:** poll `resolvectl dns` until expected; `resolvectl flush-caches`.

## Product

- OpenVPN backend (`openvpn --config --daemon`, PID tracking, auth)
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
