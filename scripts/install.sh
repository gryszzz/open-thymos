#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${THYMOS_INSTALL_PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
CONFIG_DIR="$HOME/.config/thymos"
CONFIG_FILE="$CONFIG_DIR/thymos.env"

blue='\033[38;2;119;169;255m'
green='\033[38;2;52;211;153m'
dim='\033[2m'
bold='\033[1m'
reset='\033[0m'

if [[ -n "${NO_COLOR:-}" || "${TERM:-}" == "dumb" ]]; then
  blue=''
  green=''
  dim=''
  bold=''
  reset=''
fi

printf '%b\n' "${blue}${bold}THYMOS INSTALLER${reset} ${dim}governed runtime binaries${reset}"
printf '%b\n' "${dim}repo${reset}      $ROOT"
printf '%b\n' "${dim}prefix${reset}    $PREFIX"
printf '\n'

command -v cargo >/dev/null 2>&1 || {
  printf 'cargo is required. Install Rust first: https://rustup.rs\n' >&2
  exit 1
}

cd "$ROOT/thymos"
cargo build --release -p thymos-cli -p thymos-server -p thymos-worker

mkdir -p "$BIN_DIR" "$CONFIG_DIR"
cp target/release/thymos "$BIN_DIR/thymos"
cp target/release/thymos-server "$BIN_DIR/thymos-server"
cp target/release/thymos-worker "$BIN_DIR/thymos-worker"
chmod +x "$BIN_DIR/thymos" "$BIN_DIR/thymos-server" "$BIN_DIR/thymos-worker"

if [[ ! -f "$CONFIG_FILE" ]]; then
  cat > "$CONFIG_FILE" <<'EOF'
# Thymos terminal defaults
export THYMOS_URL="http://localhost:3001"

# Optional authenticated gateway key
# export THYMOS_API_KEY=""

# Optional cognition providers
# export OPENAI_API_KEY=""
# export ANTHROPIC_API_KEY=""
# export HF_TOKEN=""
# export OPENAI_BASE_URL="http://localhost:1234/v1"
EOF
fi

printf '\n%b\n' "${green}${bold}Installed${reset}"
printf '  %s/thymos\n' "$BIN_DIR"
printf '  %s/thymos-server\n' "$BIN_DIR"
printf '  %s/thymos-worker\n' "$BIN_DIR"
printf '\n%b\n' "${blue}${bold}Config${reset}"
printf '  %s\n' "$CONFIG_FILE"
printf '\n%b\n' "${blue}${bold}Next commands${reset}"
printf '  export PATH="%s:$PATH"\n' "$BIN_DIR"
printf '  source "%s"\n' "$CONFIG_FILE"
printf '  thymos-server\n'
printf '  thymos doctor\n'
printf '  thymos shell\n'
