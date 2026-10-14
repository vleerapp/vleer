#!/bin/sh
set -eu

API="https://api.vleer.app/update/v1/check"
PUBLIC_KEY="RWRh5nLZjKooTcjP0/fbHq6xIgU9c+QhLjaVPg6gqWv1MuNRV1RWgu28"

CHANNEL="nightly"
PREFIX="${HOME}/.local"
VERSION=""

die() {
  echo "error: $*" >&2
  exit 1
}

info() {
  echo "==> $*"
}

warn() {
  echo "warning: $*" >&2
}

usage() {
  cat <<'USAGE'
Install Vleer.

Usage: install.sh [options]

Options:
  --channel <stable|nightly>  Release channel (default: nightly)
  --version <version>         Install a specific version instead of the latest
  --prefix <dir>              Install prefix (default: ~/.local)
  --system                    Shorthand for --prefix /usr/local
  -h, --help                  Show this help
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --channel)
      [ $# -ge 2 ] || die "--channel requires a value"
      CHANNEL="$2"
      shift 2
      ;;
    --version)
      [ $# -ge 2 ] || die "--version requires a value"
      VERSION="$2"
      shift 2
      ;;
    --prefix)
      [ $# -ge 2 ] || die "--prefix requires a value"
      PREFIX="$2"
      shift 2
      ;;
    --system)
      PREFIX="/usr/local"
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

case "$CHANNEL" in
  stable | nightly) ;;
  *) die "unknown channel: $CHANNEL (expected stable or nightly)" ;;
esac

[ "$(uname -s)" = "Linux" ] || die "this script only supports Linux"

case "$(uname -m)" in
  x86_64 | amd64) ARCH="x86_64" ;;
  aarch64 | arm64) ARCH="aarch64" ;;
  *) die "unsupported architecture: $(uname -m)" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"

fetch() {
  curl -fsSL "$1"
}

json_field() {
  if command -v jq >/dev/null 2>&1; then
    printf '%s' "$1" | jq -r "$2 // empty"
  else
    printf '%s' "$1" | tr -d '\n ' | grep -o "$3" | head -n 1 | sed 's/.*"\([^"]*\)"$/\1/'
  fi
}

if [ "$CHANNEL" = "nightly" ]; then
  CHECK_URL="${API}?nightly=true"
else
  CHECK_URL="$API"
fi

info "resolving $CHANNEL build for linux-$ARCH"
MANIFEST=$(fetch "$CHECK_URL") || die "could not reach $CHECK_URL"

REMOTE_VERSION=$(json_field "$MANIFEST" '.version' '"version":"[^"]*"')
[ -n "$REMOTE_VERSION" ] || die "update manifest has no version"

if [ -n "$VERSION" ] && [ "$VERSION" != "$REMOTE_VERSION" ]; then
  die "only $REMOTE_VERSION is currently published on the $CHANNEL channel"
fi
VERSION="$REMOTE_VERSION"

URL=$(json_field "$MANIFEST" ".platforms[\"linux-${ARCH}\"].url" "\"linux-${ARCH}\":{\"url\":\"[^\"]*\"")
[ -n "$URL" ] || die "no linux-$ARCH build available on the $CHANNEL channel"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

info "downloading $VERSION"
curl -fsSL -o "$TMP/vleer.tar.gz" "$URL" || die "download failed: $URL"

if command -v minisign >/dev/null 2>&1; then
  if curl -fsSL -o "$TMP/vleer.tar.gz.minisig" "${URL}.minisig" 2>/dev/null; then
    info "verifying signature"
    minisign -Vqm "$TMP/vleer.tar.gz" -x "$TMP/vleer.tar.gz.minisig" -P "$PUBLIC_KEY" >/dev/null 2>&1 ||
      die "signature verification failed - refusing to install"
  else
    warn "no signature published for this build, skipping verification"
  fi
else
  warn "minisign not found, skipping signature verification"
fi

info "extracting"
mkdir -p "$TMP/extract"
tar -xzf "$TMP/vleer.tar.gz" -C "$TMP/extract"

TREE=$(find "$TMP/extract" -mindepth 1 -maxdepth 1 -type d | head -n 1)
[ -n "$TREE" ] || die "unexpected archive layout"
[ -f "$TREE/bin/vleer" ] || die "archive does not contain bin/vleer"

mkdir -p "$PREFIX" || die "cannot create $PREFIX"
[ -w "$PREFIX" ] || die "$PREFIX is not writable (use --prefix, or run with sudo for --system)"

info "installing to $PREFIX"
cp -R "$TREE/." "$PREFIX/"
chmod 0755 "$PREFIX/bin/vleer"

DESKTOP="$PREFIX/share/applications/vleer.desktop"
if [ -f "$DESKTOP" ]; then
  sed -e "s#^Exec=vleer\$#Exec=$PREFIX/bin/vleer#" \
    -e "s#^Exec=vleer\([[:space:]]\)#Exec=$PREFIX/bin/vleer\1#" \
    "$DESKTOP" > "$DESKTOP.tmp" && mv "$DESKTOP.tmp" "$DESKTOP"
fi

mkdir -p "$PREFIX/share/vleer"
cat > "$PREFIX/share/vleer/install-receipt.json" <<RECEIPT
{
  "prefix": "$PREFIX",
  "channel": "$CHANNEL",
  "version": "$VERSION",
  "arch": "$ARCH"
}
RECEIPT

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$PREFIX/share/applications" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi

info "installed Vleer $VERSION to $PREFIX"

case ":${PATH}:" in
  *":$PREFIX/bin:"*) ;;
  *) warn "$PREFIX/bin is not on your PATH - add it to your shell profile" ;;
esac
