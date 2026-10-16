#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "\x1b[1;36m➜\x1b[0m Installing norb-cli via cargo..."
cargo install --path crates/norb-cli

CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/norb"

if [[ -f "$CARGO_BIN" ]]; then
    echo -e "\x1b[1;36m➜\x1b[0m Signing $CARGO_BIN with Hypervisor entitlement..."
    codesign -s - --entitlements "$SCRIPT_DIR/entitlements.plist" --force "$CARGO_BIN"
    NORBCTL_BIN="${CARGO_HOME:-$HOME/.cargo}/bin/norbctl"
    ln -sf "$CARGO_BIN" "$NORBCTL_BIN"
    echo -e "\x1b[1;32m✓\x1b[0m Successfully installed and signed \x1b[1mnorb\x1b[0m and \x1b[1mnorbctl\x1b[0m"
    echo ""
    echo "You can now run 'norb' and 'norbctl' directly from anywhere in your terminal:"
    echo "  norb                # Drop into default Linux shell"
    echo "  norb uname -a       # Transparently run command in Linux"
    echo "  norbctl list        # Manage machines and settings"
    echo "  norbctl doctor      # Verify system health"
else
    echo -e "\x1b[1;31m✖\x1b[0m Could not locate installed binary at $CARGO_BIN"
    exit 1
fi
