#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────────
# tuinnel installer
# ──────────────────────────────────────────────────────────────────────────────

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.config/tuinnel"
SERVERS_DIR="${CONFIG_DIR}/servers"
STATE_DIR="${HOME}/.local/state/tuinnel"
SYSTEMD_DIR="${HOME}/.config/systemd/user"

info()  { printf '  \033[32m✓\033[0m %s\n' "$1"; }
warn()  { printf '  \033[33m⚠\033[0m %s\n' "$1"; }
err()   { printf '  \033[31m✗\033[0m %s\n' "$1"; exit 1; }

echo ""
echo "  tuinnel installer"
echo "  ═════════════════"
echo ""

# ── Dependencies ────────────────────────────────────────────────────────────

echo "  Checking dependencies..."
MISSING=()
for cmd in wg-quick wg; do
    command -v "$cmd" &>/dev/null || MISSING+=("wireguard-tools")
done
command -v nft &>/dev/null || MISSING+=("nftables")
command -v resolvconf &>/dev/null || MISSING+=("openresolv")
command -v curl &>/dev/null || MISSING+=("curl")

if [[ ${#MISSING[@]} -gt 0 ]]; then
    # Deduplicate
    UNIQUE=($(printf '%s\n' "${MISSING[@]}" | sort -u))
    warn "Missing packages: ${UNIQUE[*]}"
    echo "     Install with: sudo pacman -S ${UNIQUE[*]}"
    echo ""
    read -rp "  Install now? [Y/n] " answer
    if [[ "${answer:-y}" =~ ^[Yy]$ ]]; then
        sudo pacman -S --needed "${UNIQUE[@]}"
        info "Dependencies installed"
    fi
else
    info "All dependencies found"
fi

# ── Build ───────────────────────────────────────────────────────────────────

if ! command -v cargo &>/dev/null; then
    err "Rust/cargo not found. Install: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
fi

echo "  Building release binary..."
cargo build --release --manifest-path "${SCRIPT_DIR}/Cargo.toml" 2>&1 | tail -1
info "Build complete"

# ── Install binary ──────────────────────────────────────────────────────────

mkdir -p "${BIN_DIR}"
cp "${SCRIPT_DIR}/target/release/tuinnel" "${BIN_DIR}/tuinnel"
chmod +x "${BIN_DIR}/tuinnel"
info "Installed tuinnel to ${BIN_DIR}/tuinnel"

# ── Config ──────────────────────────────────────────────────────────────────

mkdir -p "${CONFIG_DIR}"
mkdir -p "${SERVERS_DIR}"

if [[ ! -f "${CONFIG_DIR}/config.toml" ]]; then
    cp "${SCRIPT_DIR}/config/config.toml" "${CONFIG_DIR}/config.toml"
    info "Created default config at ${CONFIG_DIR}/config.toml"
else
    warn "Config already exists (not overwritten)"
fi

# ── State directory ─────────────────────────────────────────────────────────

mkdir -p "${STATE_DIR}"
info "Log directory: ${STATE_DIR}"

# ── Systemd user service ───────────────────────────────────────────────────

mkdir -p "${SYSTEMD_DIR}"
cp "${SCRIPT_DIR}/systemd/tuinnel-autoconnect.service" "${SYSTEMD_DIR}/tuinnel-autoconnect.service"
info "Installed systemd service"

# ── PATH check ──────────────────────────────────────────────────────────────

if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
    warn "${BIN_DIR} is not in your PATH"
    echo "     Add to your shell rc:"
    echo "       export PATH=\"\${HOME}/.local/bin:\${PATH}\""
fi

# ── Done ────────────────────────────────────────────────────────────────────

echo ""
info "Installation complete!"
echo ""
echo "  Next steps:"
echo "    1. Drop WireGuard .conf files into ${SERVERS_DIR}/"
echo "       (download from your VPN provider's website)"
echo "    2. tuinnel doctor            (verify system)"
echo "    3. tuinnel connect            (connect to VPN)"
echo "    4. tuinnel                    (interactive TUI)"
echo ""
echo "  Optional — autoconnect on login:"
echo "    systemctl --user daemon-reload"
echo "    systemctl --user enable --now tuinnel-autoconnect.service"
echo ""
echo "  Optional — passwordless sudo for wg-quick:"
echo "    sudo tee /etc/sudoers.d/tuinnel <<< '%wheel ALL=(root) NOPASSWD: /usr/bin/wg-quick, /usr/bin/wg, /usr/bin/nft'"
echo ""
warn "Security caveat: wg-quick interprets PostUp/PostDown shell hooks as root."
echo "     Granting NOPASSWD on wg-quick is effectively NOPASSWD on bash for"
echo "     any process that can drop a .conf into ~/.config/tuinnel/servers/."
echo "     Read the Security Model section in README.md before enabling."
echo ""
