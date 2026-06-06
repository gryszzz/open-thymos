#!/usr/bin/env sh
# OpenThymos prebuilt installer — no Rust, no clone, no compile.
#
#   curl -fsSL https://raw.githubusercontent.com/gryszzz/open-thymos/main/scripts/get.sh | sh
#
# Downloads the right prebuilt `thymos` + `thymos-server` for your OS/arch from
# the GitHub Release that .github/workflows/release.yml publishes, and installs
# them to ~/.local/bin (override with THYMOS_INSTALL_PREFIX).
#
# Pin a version:  THYMOS_VERSION=v0.5.0 sh get.sh
#
# Privacy: this fetches ONLY from github.com (the release assets). No telemetry,
# no phone-home — same posture as the runtime itself.
set -eu

REPO="${THYMOS_REPO:-gryszzz/open-thymos}"
PREFIX="${THYMOS_INSTALL_PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"

blue='\033[38;2;124;92;255m'; green='\033[38;2;70;211;154m'
dim='\033[2m'; bold='\033[1m'; reset='\033[0m'
if [ -n "${NO_COLOR:-}" ] || [ "${TERM:-}" = "dumb" ] || [ ! -t 1 ]; then
  blue=''; green=''; dim=''; bold=''; reset=''
fi
say() { printf '%b\n' "$1"; }
die() { printf '%b\n' "error: $1" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die "curl is required."
command -v tar  >/dev/null 2>&1 || die "tar is required."

# --- detect target triple (must match release.yml's matrix) ---
os="$(uname -s)"; arch="$(uname -m)"
case "$os" in
  Darwin)
    case "$arch" in
      arm64|aarch64) target="aarch64-apple-darwin" ;;
      x86_64)        target="x86_64-apple-darwin" ;;
      *) die "unsupported macOS arch: $arch" ;;
    esac ;;
  Linux)
    case "$arch" in
      x86_64) target="x86_64-unknown-linux-gnu" ;;
      *) die "unsupported Linux arch: $arch (build from source, or open an issue for a $arch release)" ;;
    esac ;;
  MINGW*|MSYS*|CYGWIN*)
    die "On Windows use the .msi installer from the Releases page (or Scoop), not this script." ;;
  *) die "unsupported OS: $os" ;;
esac

# --- resolve version (default: latest published release) ---
ver="${THYMOS_VERSION:-}"
if [ -z "$ver" ]; then
  # Follow the /releases/latest redirect; the final URL ends in the tag.
  latest_url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
    "https://github.com/$REPO/releases/latest" 2>/dev/null || true)"
  ver="${latest_url##*/tag/}"
  case "$ver" in
    v*) : ;;
    *) die "no published release found for $REPO yet.
   The runtime is real, but binaries ship with the first tagged release.
   Until then: build from source — https://github.com/$REPO#quick-start" ;;
  esac
fi

asset="thymos-${ver}-${target}.tar.gz"
url="https://github.com/$REPO/releases/download/${ver}/${asset}"

say "${blue}${bold}OpenThymos${reset} ${dim}prebuilt installer${reset}"
say "${dim}target${reset}   $target"
say "${dim}version${reset}  $ver"
say "${dim}prefix${reset}   $PREFIX"
printf '\n'

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
say "downloading ${dim}$url${reset}"
curl -fSL --proto '=https' --tlsv1.2 "$url" -o "$tmp/$asset" \
  || die "download failed. Asset for $target may not exist in $ver — check https://github.com/$REPO/releases/$ver"
tar -xzf "$tmp/$asset" -C "$tmp"

mkdir -p "$BIN_DIR"
for b in thymos thymos-server; do
  [ -f "$tmp/$b" ] || die "archive missing $b (unexpected release layout)"
  install -m 0755 "$tmp/$b" "$BIN_DIR/$b"
done

say "\n${green}${bold}Installed${reset}"
say "  $BIN_DIR/thymos"
say "  $BIN_DIR/thymos-server"

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "\n${blue}${bold}Add to PATH${reset}"
     say "  export PATH=\"$BIN_DIR:\$PATH\"   ${dim}# add to your shell rc${reset}" ;;
esac

say "\n${blue}${bold}Next${reset}"
say "  thymos-server &     ${dim}# start the runtime (mock until you set a provider key)${reset}"
say "  thymos doctor       ${dim}# check readiness${reset}"
say "  thymos shell        ${dim}# interactive governed shell${reset}"
