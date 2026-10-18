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
VERSION=$(grep '^version = ' "$SCRIPT_DIR/Cargo.toml" | head -n 1 | cut -d '"' -f 2)
echo -e "\x1b[1;36m➜\x1b[0m Packaging distribution archive for Homebrew (v$VERSION)..."
mkdir -p "$SCRIPT_DIR/dist"
PKG_TMP="/tmp/mach-dist-pkg-$$"
rm -rf "$PKG_TMP"
mkdir -p "$PKG_TMP/assets" "$PKG_TMP/man/man1" "$PKG_TMP/completions"

cp "$RELEASE_BIN" "$PKG_TMP/mach"
ln -sf "mach" "$PKG_TMP/machctl"
cp "$SCRIPT_DIR/entitlements.plist" "$PKG_TMP/"
cp -R "$SCRIPT_DIR/assets/"* "$PKG_TMP/assets/"
cp -R "$SCRIPT_DIR/man/man1/"* "$PKG_TMP/man/man1/"

# Generate shell completions for package distribution
"$CARGO_BIN_DIR/mach" completion bash > "$PKG_TMP/completions/mach.bash" 2>/dev/null || true
"$CARGO_BIN_DIR/mach" completion zsh > "$PKG_TMP/completions/_mach" 2>/dev/null || true
cp "$PKG_TMP/completions/_mach" "$PKG_TMP/completions/_machctl"
"$CARGO_BIN_DIR/mach" completion fish > "$PKG_TMP/completions/mach.fish" 2>/dev/null || true

TAR_FILE="$SCRIPT_DIR/dist/mach-$VERSION-aarch64-apple-darwin.tar.gz"
tar -czf "$TAR_FILE" -C "$PKG_TMP" .
rm -rf "$PKG_TMP"

NEW_SHA=$(shasum -a 256 "$TAR_FILE" | awk '{print $1}')
echo -e "  \x1b[1;34mArchive SHA256:\x1b[0m $NEW_SHA"

# 3. Update Formula/mach.rb & Casks/mach.rb in repo
sed -i '' -E "s|url \"file://.*/dist/mach-.*-aarch64-apple-darwin.tar.gz\"|url \"file://$SCRIPT_DIR/dist/mach-$VERSION-aarch64-apple-darwin.tar.gz\"|" "$SCRIPT_DIR/Formula/mach.rb"
sed -i '' "s/sha256 \".*\"/sha256 \"$NEW_SHA\"/" "$SCRIPT_DIR/Formula/mach.rb"

sed -i '' "s/version \".*\"/version \"$VERSION\"/" "$SCRIPT_DIR/Casks/mach.rb"
sed -i '' -E "s|url \"file://.*/dist/mach-.*-aarch64-apple-darwin.tar.gz\"|url \"file://$SCRIPT_DIR/dist/mach-$VERSION-aarch64-apple-darwin.tar.gz\"|" "$SCRIPT_DIR/Casks/mach.rb"
sed -i '' "s/sha256 \".*\"/sha256 \"$NEW_SHA\"/" "$SCRIPT_DIR/Casks/mach.rb"

# 4. Update tapped Cask (and clear legacy formula in tap to prevent conflicts)
TAP_DIR="/opt/homebrew/Library/Taps/douglaslobo/homebrew-mach"
if [[ -d "$TAP_DIR" ]]; then
    echo -e "\x1b[1;36m➜\x1b[0m Updating Homebrew tap at $TAP_DIR..."
    mkdir -p "$TAP_DIR/Casks"
    cp "$SCRIPT_DIR/Casks/mach.rb" "$TAP_DIR/Casks/mach.rb"
    # Remove formula in tap if present to ensure clean Cask resolution and clean package-manager uninstall hooks
    rm -f "$TAP_DIR/Formula/mach.rb" "$TAP_DIR/Formula/mach.rb.bak" || true
    if [[ -d "$TAP_DIR/.git" ]]; then
        git -C "$TAP_DIR" add -A
        git -C "$TAP_DIR" commit -m "update mach cask to $VERSION ($NEW_SHA)" >/dev/null 2>&1 || true
    fi
fi

# 5. Clear cached Homebrew download so brew doesn't use stale archive
rm -f ~/Library/Caches/Homebrew/downloads/*mach* || true

echo -e "\x1b[1;32m✓\x1b[0m Successfully built, packaged, and synced mach"
echo ""
echo "Quick verification:"
echo "  mach --version"
echo "  machctl doctor"
