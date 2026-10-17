#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo -e "\x1b[1;36m➜\x1b[0m Compiling mach release binary via cargo..."
cargo build --release --bin mach

RELEASE_BIN="$SCRIPT_DIR/target/release/mach"

if [[ ! -f "$RELEASE_BIN" ]]; then
    echo -e "\x1b[1;31m✖\x1b[0m Release binary not found at $RELEASE_BIN"
    exit 1
fi

echo -e "\x1b[1;36m➜\x1b[0m Signing $RELEASE_BIN with Apple Silicon Hypervisor entitlements..."
codesign -s - --entitlements "$SCRIPT_DIR/entitlements.plist" --force "$RELEASE_BIN"

# 1. Install to ~/.cargo/bin
CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$CARGO_BIN_DIR"
cp "$RELEASE_BIN" "$CARGO_BIN_DIR/mach"
codesign -s - --entitlements "$SCRIPT_DIR/entitlements.plist" --force "$CARGO_BIN_DIR/mach"
ln -sf "$CARGO_BIN_DIR/mach" "$CARGO_BIN_DIR/machctl"

# Install man pages to ~/.local/share/man/man1
MAN_DIR="${HOME}/.local/share/man/man1"
mkdir -p "$MAN_DIR"
cp "$SCRIPT_DIR/man/man1/mach.1" "$MAN_DIR/"
cp "$SCRIPT_DIR/man/man1/machctl.1" "$MAN_DIR/"

echo -e "\x1b[1;32m✓\x1b[0m Installed and signed mach at $CARGO_BIN_DIR/mach"

# 2. Package Homebrew distribution tarball
echo -e "\x1b[1;36m➜\x1b[0m Packaging distribution archive for Homebrew..."
mkdir -p "$SCRIPT_DIR/dist"
PKG_TMP="/tmp/mach-dist-pkg-$$"
rm -rf "$PKG_TMP"
mkdir -p "$PKG_TMP/assets" "$PKG_TMP/man/man1"

cp "$RELEASE_BIN" "$PKG_TMP/mach"
cp "$SCRIPT_DIR/entitlements.plist" "$PKG_TMP/"
cp -R "$SCRIPT_DIR/assets/"* "$PKG_TMP/assets/"
cp -R "$SCRIPT_DIR/man/man1/"* "$PKG_TMP/man/man1/"

TAR_FILE="$SCRIPT_DIR/dist/mach-0.1.0-aarch64-apple-darwin.tar.gz"
tar -czf "$TAR_FILE" -C "$PKG_TMP" .
rm -rf "$PKG_TMP"

NEW_SHA=$(shasum -a 256 "$TAR_FILE" | awk '{print $1}')
echo -e "  \x1b[1;34mArchive SHA256:\x1b[0m $NEW_SHA"

# 3. Update Formula/mach.rb in repo
sed -i '' "s/sha256 \".*\"/sha256 \"$NEW_SHA\"/" "$SCRIPT_DIR/Formula/mach.rb"

# 4. Update tapped formula if present
TAP_FORMULA="/opt/homebrew/Library/Taps/douglaslobo/homebrew-mach/Formula/mach.rb"
if [[ -f "$TAP_FORMULA" ]]; then
    echo -e "\x1b[1;36m➜\x1b[0m Updating Homebrew tap at $TAP_FORMULA..."
    cp "$SCRIPT_DIR/Formula/mach.rb" "$TAP_FORMULA"
    if [[ -d "/opt/homebrew/Library/Taps/douglaslobo/homebrew-mach/.git" ]]; then
        git -C "/opt/homebrew/Library/Taps/douglaslobo/homebrew-mach" add Formula/mach.rb
        git -C "/opt/homebrew/Library/Taps/douglaslobo/homebrew-mach" commit -m "update mach formula sha256 to $NEW_SHA" >/dev/null 2>&1 || true
    fi
fi

# 5. Clear cached Homebrew download so brew doesn't use stale archive
rm -f ~/Library/Caches/Homebrew/downloads/*mach* || true

echo -e "\x1b[1;32m✓\x1b[0m Successfully built, packaged, and synced mach"
echo ""
echo "Quick verification:"
echo "  mach --version"
echo "  machctl doctor"
