#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "\x1b[1;36m➜\x1b[0m Installing mach-cli via cargo..."
cargo install --path crates/mach-cli

CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/mach"

if [[ -f "$CARGO_BIN" ]]; then
    echo -e "\x1b[1;36m➜\x1b[0m Signing $CARGO_BIN with Apple Silicon Hypervisor entitlements..."
    codesign -s - --entitlements "$SCRIPT_DIR/entitlements.plist" --force "$CARGO_BIN"
    MACHCTL_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/machctl"
    ln -sf "$CARGO_BIN" "$MACHCTL_BIN"

    # Also keep legacy symlinks so existing muscle memory never breaks
    NORB_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/norb"
    NORBCTL_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/norbctl"
    ln -sf "$CARGO_BIN" "$NORB_BIN"
    ln -sf "$CARGO_BIN" "$NORBCTL_BIN"

    # Install man pages to ~/.local/share/man/man1 if directory exists or /usr/local/share/man
    MAN_DIR="${HOME}/.local/share/man/man1"
    mkdir -p "$MAN_DIR"
    cp "$SCRIPT_DIR/man/man1/mach.1" "$MAN_DIR/"
    cp "$SCRIPT_DIR/man/man1/machctl.1" "$MAN_DIR/"

    echo -e "\x1b[1;32m✓\x1b[0m Successfully installed and signed \x1b[1mmach\x1b[0m and \x1b[1mmachctl\x1b[0m"
    echo ""
    echo "You can now run 'mach' and 'machctl' directly from anywhere in your terminal:"
    echo "  mach                # Drop into default Linux shell"
    echo "  mach uname -a       # Transparently run command in Linux"
    echo "  machctl list        # Manage machines and settings"
    echo "  machctl doctor      # Verify system health"
else
    echo -e "\x1b[1;31m✖\x1b[0m Could not locate installed binary at $CARGO_BIN"
    exit 1
fi
