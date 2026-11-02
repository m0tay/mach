#!/usr/bin/env bash
# Build a signed local install and the portable Homebrew release archive.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"
SYNC_TAP=false
case "${1:-}" in
    "") ;;
    --sync-tap) SYNC_TAP=true ;;
    *) echo "Usage: $0 [--sync-tap]" >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { echo "Too many arguments" >&2; exit 2; }
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || {
    echo "Mach release packaging requires macOS on Apple Silicon." >&2; exit 1;
}
VERSION=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
REPOSITORY="https://github.com/m0tay/mach"
ARCHIVE_NAME="mach-$VERSION-aarch64-apple-darwin.tar.gz"
RELEASE_URL="$REPOSITORY/releases/download/v$VERSION/$ARCHIVE_NAME"
PKG_TMP=$(mktemp -d "${TMPDIR:-/tmp}/mach-dist.XXXXXX")
trap 'rm -rf "$PKG_TMP"' EXIT

echo "Building mach v$VERSION..."
cargo build --release --locked --bin mach
RELEASE_BIN="$SCRIPT_DIR/target/release/mach"
codesign --force --entitlements entitlements.plist --sign - "$RELEASE_BIN"
codesign --verify --strict "$RELEASE_BIN"

mkdir -p "$PKG_TMP/assets" "$PKG_TMP/man/man1" "$PKG_TMP/completions"
cp "$RELEASE_BIN" "$PKG_TMP/mach"
ln -s mach "$PKG_TMP/machctl"
cp entitlements.plist "$PKG_TMP/"
cp -R assets/. "$PKG_TMP/assets/"
cp man/man1/mach.1 man/man1/machctl.1 "$PKG_TMP/man/man1/"
# The generator registers both command trees. Never ship empty completions.
"$PKG_TMP/machctl" completion bash > "$PKG_TMP/completions/mach.bash"
"$PKG_TMP/machctl" completion zsh > "$PKG_TMP/completions/_mach"
"$PKG_TMP/machctl" completion fish > "$PKG_TMP/completions/mach.fish"
for script in "$PKG_TMP/completions/"*; do
    [[ -s "$script" ]] || { echo "Empty completion: $script" >&2; exit 1; }
done
# Both names are discoverable by zsh's completion loader.
cp "$PKG_TMP/completions/_mach" "$PKG_TMP/completions/_machctl"
mkdir -p dist
# Normalize ownership, ordering and timestamps; gzip omits source filename/time.
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
export SOURCE_DATE_EPOCH
python3 - "$PKG_TMP" "$SCRIPT_DIR/dist/$ARCHIVE_NAME" <<'PY'
import gzip, os, pathlib, sys, tarfile
root, output = pathlib.Path(sys.argv[1]), sys.argv[2]
epoch = int(os.environ['SOURCE_DATE_EPOCH'])
with open(output, 'wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as gz:
    with tarfile.open(fileobj=gz, mode='w', format=tarfile.USTAR_FORMAT) as archive:
        for path in sorted(root.rglob('*')):
            item = archive.gettarinfo(str(path), str(path.relative_to(root)))
            item.uid = item.gid = 0
            item.uname = item.gname = ''
            item.mtime = epoch
            if item.isfile():
                with path.open('rb') as data:
                    archive.addfile(item, data)
            else:
                archive.addfile(item)
PY
NEW_SHA=$(shasum -a 256 "dist/$ARCHIVE_NAME" | awk '{print $1}')
python3 - "$VERSION" "$NEW_SHA" "$RELEASE_URL" <<'PY'
import pathlib, re, sys
version, sha, url = sys.argv[1:]
for name in ('Formula/mach.rb', 'Casks/mach.rb'):
    path = pathlib.Path(name)
    text = path.read_text()
    text = re.sub(r'(?m)^  (?:url|sha256|version) ".*"', lambda m: '  '+m[0].strip().split()[0]+' "'+{'url':url,'sha256':sha,'version':version}[m[0].strip().split()[0]]+'"', text)
    path.write_text(text)
PY

CARGO_BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$CARGO_BIN_DIR" "$HOME/.local/share/man/man1"
cp "$RELEASE_BIN" "$CARGO_BIN_DIR/mach"
ln -sf mach "$CARGO_BIN_DIR/machctl"
codesign --verify --strict "$CARGO_BIN_DIR/mach"
cp man/man1/mach.1 man/man1/machctl.1 "$HOME/.local/share/man/man1/"
if $SYNC_TAP; then
    # Resolve the actual tap location; never commit unrelated tap changes.
    TAP_NAME="${MACH_HOMEBREW_TAP:-m0tay/mach}"
    TAP_DIR=$(brew --repository "$TAP_NAME")
    [[ -d "$TAP_DIR/.git" ]] || {
        echo "Tap $TAP_NAME is not installed. Run: brew tap $TAP_NAME $REPOSITORY" >&2
        exit 1
    }
    mkdir -p "$TAP_DIR/Casks" "$TAP_DIR/Formula"
    cp Casks/mach.rb "$TAP_DIR/Casks/mach.rb"
    cp Formula/mach.rb "$TAP_DIR/Formula/mach.rb"
    echo "Synced Homebrew definitions at $TAP_DIR (no commit or push)."
fi
printf 'Built and installed mach v%s\nArchive: dist/%s\nSHA256: %s\nPublish archive at: %s\n' "$VERSION" "$ARCHIVE_NAME" "$NEW_SHA" "$RELEASE_URL"
