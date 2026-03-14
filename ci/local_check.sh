#!/usr/bin/env bash
set -euo pipefail

export CARGO_TERM_COLOR=always
export CARGO_NET_RETRY=10
export CARGO_HTTP_CHECK_REVOKE=false

TOOLCHAIN_FILE="rust-toolchain.toml"
TOOLCHAIN_CHANNEL_DEFAULT="1.91.0"
TOOLCHAIN_CHANNEL="$TOOLCHAIN_CHANNEL_DEFAULT"
TOOLCHAIN_COMPONENTS=""
PYTHON_BIN="${PYTHON_BIN:-}"
if [[ -z "$PYTHON_BIN" ]]; then
  if command -v python3 >/dev/null 2>&1; then
    PYTHON_BIN="python3"
  elif command -v python >/dev/null 2>&1; then
    PYTHON_BIN="python"
  fi
fi
if [[ -r "${TOOLCHAIN_FILE}" ]]; then
  if [[ -n "$PYTHON_BIN" ]] && TOOLCHAIN_INFO=$("$PYTHON_BIN" - <<'PY'
import pathlib, re

path = pathlib.Path("rust-toolchain.toml")
text = path.read_text()
channel = ""
components = []
for line in text.splitlines():
    line = line.strip()
    if line.startswith("channel") and "=" in line:
        channel = line.split("=", 1)[1].strip().strip('"')
    elif line.startswith("components") and "=" in line:
        components = re.findall(r'"([^"]+)"', line)
print(channel)
print(" ".join(components))
PY
  ); then
    TOOLCHAIN_CHANNEL=$(printf '%s' "$TOOLCHAIN_INFO" | sed -n '1p' | tr -d '\r')
    TOOLCHAIN_COMPONENTS=$(printf '%s' "$TOOLCHAIN_INFO" | sed -n '2p' | tr -d '\r')
    if [[ -z "$TOOLCHAIN_CHANNEL" ]]; then
      TOOLCHAIN_CHANNEL="$TOOLCHAIN_CHANNEL_DEFAULT"
    fi
  elif [[ -z "$PYTHON_BIN" ]]; then
    echo "[check_local] python not found; using default rust toolchain ${TOOLCHAIN_CHANNEL}"
  fi
fi

if [[ "${CARGO_NET_OFFLINE:-false}" != "true" ]]; then
  echo "[check_local] ensuring rust toolchain ${TOOLCHAIN_CHANNEL} is installed"
  rustup toolchain install "$TOOLCHAIN_CHANNEL"
  if [[ -n "$TOOLCHAIN_COMPONENTS" ]]; then
    rustup component add --toolchain "$TOOLCHAIN_CHANNEL" $TOOLCHAIN_COMPONENTS
  fi
else
  echo "[check_local] offline; skipping rust toolchain install for ${TOOLCHAIN_CHANNEL}"
fi
export RUSTUP_TOOLCHAIN="$TOOLCHAIN_CHANNEL"

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  export CARGO_TARGET_DIR="$(pwd)/.target-local"
fi

OFFLINE_FLAG=""
LOCKED_FLAG="--locked"
if [[ "${CARGO_NET_OFFLINE:-false}" == "true" ]]; then
  OFFLINE_FLAG="--offline"
  LOCKED_FLAG=""
fi

echo "[check_local] toolchain:"
rustup --version || true
cargo --version

ensure_bin() {
  local bin="$1"
  if command -v "$bin" >/dev/null 2>&1; then
    return 0
  fi
  if [[ -n "${OFFLINE_FLAG}" ]]; then
    echo "[check_local] missing $bin but offline; please install it manually" >&2
    return 1
  fi
  echo "[check_local] installing $bin via cargo binstall"
  cargo binstall ${LOCKED_FLAG} ${OFFLINE_FLAG} -y "$bin"
}

echo "[check_local] ensuring required binaries (greentic-flow, greentic-component, greentic-pack, greentic-runner-cli)"
ensure_bin greentic-flow
ensure_bin greentic-component
ensure_bin greentic-pack
if command -v greentic-runner-cli >/dev/null 2>&1; then
  :
else
  ensure_bin greentic-runner
fi

if [[ -z "${OFFLINE_FLAG}" ]]; then
  echo "[check_local] fetch (locked)"
  if ! cargo fetch --locked; then
    echo "[check_local] cargo fetch failed (offline?). Continuing with existing cache."
    export CARGO_NET_OFFLINE=true
    OFFLINE_FLAG="--offline"
    LOCKED_FLAG=""
  fi
fi

echo "[check_local] fmt + clippy"
cargo fmt --all -- --check
cargo clippy --all --all-features ${LOCKED_FLAG} ${OFFLINE_FLAG} -- -D warnings

echo "[check_local] i18n validate"
tools/i18n.sh validate

echo "[check_local] build (locked)"
cargo build --workspace --all-features ${LOCKED_FLAG} ${OFFLINE_FLAG}

echo "[check_local] test (locked)"
cargo test --workspace --all-features ${LOCKED_FLAG} ${OFFLINE_FLAG} -- --nocapture

echo "[check_local] OK"
