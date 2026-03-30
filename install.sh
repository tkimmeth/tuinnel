#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────────
# pvpn-bot installer for Arch Linux
# ──────────────────────────────────────────────────────────────────────────────
#
# BASH LESSON (for Rust people):
#   set -e  → exit on any error (like Rust's unwrap — fail fast)
#   set -u  → error on undefined variables (like Rust's compiler checks)
#   set -o pipefail → pipe fails if ANY command in the pipe fails
#
# This is called "strict mode" and every bash script should start with it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.config/pvpn-bot"
STATE_DIR="${HOME}/.local/state/pvpn-bot"
SYSTEMD_DIR="${HOME}/.config/systemd/user"

info()  { printf '  \033[32m✓\033[0m %s\n' "$1"; }
warn()  { printf '  \033[33m⚠\033[0m %s\n' "$1"; }
err()   { printf '  \033[31m✗\033[0m %s\n' "$1"; exit 1; }

echo ""
echo "  pvpn-bot installer"
echo "  ══════════════════"
echo ""

# ── Build ────────────────────────────────────────────────────────────────────

if ! command -v cargo &>/dev/null; then
    err "Rust/cargo not found. Install with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
fi

echo "  Building release binary..."
cargo build --release --manifest-path "${SCRIPT_DIR}/Cargo.toml" 2>&1 | tail -1
info "Build complete"

# ── Install binary ───────────────────────────────────────────────────────────

mkdir -p "${BIN_DIR}"
cp "${SCRIPT_DIR}/target/release/pvpn" "${BIN_DIR}/pvpn"
chmod +x "${BIN_DIR}/pvpn"
info "Installed pvpn to ${BIN_DIR}/pvpn"

# ── Config ───────────────────────────────────────────────────────────────────

mkdir -p "${CONFIG_DIR}"
if [[ ! -f "${CONFIG_DIR}/config.toml" ]]; then
    cp "${SCRIPT_DIR}/config/config.toml" "${CONFIG_DIR}/config.toml"
    info "Created default config at ${CONFIG_DIR}/config.toml"
else
    warn "Config already exists at ${CONFIG_DIR}/config.toml (not overwritten)"
fi

# ── State directory ──────────────────────────────────────────────────────────

mkdir -p "${STATE_DIR}"
info "Log directory: ${STATE_DIR}"

# ── Systemd user service ────────────────────────────────────────────────────

mkdir -p "${SYSTEMD_DIR}"
cp "${SCRIPT_DIR}/systemd/pvpn-autoconnect.service" "${SYSTEMD_DIR}/pvpn-autoconnect.service"
info "Installed systemd service to ${SYSTEMD_DIR}/pvpn-autoconnect.service"

# ── PATH check ───────────────────────────────────────────────────────────────

if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
    warn "${BIN_DIR} is not in your PATH"
    echo "     Add to your shell rc:"
    echo "       export PATH=\"\${HOME}/.local/bin:\${PATH}\""
fi

# ── Done ─────────────────────────────────────────────────────────────────────

echo ""
info "Installation complete!"
echo ""
echo "  Next steps:"
echo "    1. pvpn doctor              (verify system)"
echo "    2. pvpn status              (check VPN state)"
echo "    3. pvpn menu                (interactive TUI)"
echo ""
echo "  To enable autoconnect on login:"
echo "    systemctl --user daemon-reload"
echo "    systemctl --user enable --now pvpn-autoconnect.service"
echo ""
