#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

MODE="${1:-all}"
AUTH_MODE="${AUTH_MODE:-auto}"
LOCALE="${LOCALE:-en}"
I18N_DIR="${I18N_DIR:-i18n}"
EN_PATH="${EN_PATH:-$I18N_DIR/en.json}"
LOCALES_PATH="${LOCALES_PATH:-$I18N_DIR/locales.json}"
I18N_TRANSLATOR_MANIFEST="${I18N_TRANSLATOR_MANIFEST:-../greentic-i18n/Cargo.toml}"
BATCH_SIZE="${BATCH_SIZE:-200}"

usage() {
  cat <<'EOF'
Usage: tools/i18n.sh [translate|validate|status|all]

Environment overrides:
  I18N_DIR=...                    i18n directory path (default: i18n)
  EN_PATH=...                     English source file path (default: i18n/en.json)
  LOCALES_PATH=...                Locale list path (default: i18n/locales.json)
  AUTH_MODE=...                   Translator auth mode for translate (default: auto)
  LOCALE=...                      CLI locale used for translator output (default: en)
  I18N_TRANSLATOR_MANIFEST=...    Path to greentic-i18n Cargo.toml
  BATCH_SIZE=...                  Keys per translation request (default: 200)

Examples:
  tools/i18n.sh all
  AUTH_MODE=api-key tools/i18n.sh translate
  I18N_DIR=i18n tools/i18n.sh validate
EOF
}

require_path() {
  local path="$1"
  local label="$2"
  if [[ ! -e "$path" ]]; then
    echo "Missing $label: $path" >&2
    exit 2
  fi
}

run_translate() {
  require_path "$EN_PATH" "English source file"
  require_path "$LOCALES_PATH" "locale list"
  require_path "$I18N_TRANSLATOR_MANIFEST" "translator Cargo.toml"
  cargo run --manifest-path "$I18N_TRANSLATOR_MANIFEST" -p greentic-i18n-translator -- \
    --locale "$LOCALE" \
    translate --langs all --en "$EN_PATH" --auth-mode "$AUTH_MODE" --batch-size "$BATCH_SIZE"
}

run_validate() {
  require_path "ci/i18n_check.py" "i18n checker"
  require_path "$EN_PATH" "English source file"
  require_path "$LOCALES_PATH" "locale list"
  python3 ci/i18n_check.py validate
}

run_status() {
  require_path "ci/i18n_check.py" "i18n checker"
  require_path "$EN_PATH" "English source file"
  require_path "$LOCALES_PATH" "locale list"
  python3 ci/i18n_check.py status
}

if [[ "${MODE}" == "-h" || "${MODE}" == "--help" ]]; then
  usage
  exit 0
fi

case "$MODE" in
  translate) run_translate ;;
  validate) run_validate ;;
  status) run_status ;;
  all)
    run_translate
    run_validate
    run_status
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    usage
    exit 2
    ;;
esac
