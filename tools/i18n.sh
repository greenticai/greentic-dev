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
I18N_TRANSLATOR_MANIFEST="${I18N_TRANSLATOR_MANIFEST:-}"
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
  I18N_TRANSLATOR_MANIFEST=...    Optional manifest path for greentic-i18n-translator workspace
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

translator_cmd() {
  if [[ -n "$I18N_TRANSLATOR_MANIFEST" ]]; then
    require_path "$I18N_TRANSLATOR_MANIFEST" "translator Cargo.toml"
    echo "cargo run --manifest-path \"$I18N_TRANSLATOR_MANIFEST\" -p greentic-i18n-translator --"
    return 0
  fi

  if command -v greentic-i18n-translator >/dev/null 2>&1; then
    echo "greentic-i18n-translator"
    return 0
  fi

  echo "Missing translator: set I18N_TRANSLATOR_MANIFEST or install greentic-i18n-translator on PATH" >&2
  exit 2
}

require_locale_files() {
  require_path "$EN_PATH" "English source file"
  require_path "$LOCALES_PATH" "locale list"
  python3 - <<'PY' "$I18N_DIR" "$LOCALES_PATH"
import json
import sys
from pathlib import Path

i18n_dir = Path(sys.argv[1])
locales_path = Path(sys.argv[2])
locales = json.loads(locales_path.read_text(encoding="utf-8"))
missing = [locale for locale in locales if not (i18n_dir / f"{locale}.json").exists()]
if missing:
    print("Missing locale files:", ", ".join(missing), file=sys.stderr)
    raise SystemExit(2)
PY
}

incomplete_locales() {
  python3 - <<'PY' "$EN_PATH" "$I18N_DIR" "$LOCALES_PATH"
import json
import re
import sys
from collections import Counter
from pathlib import Path

en_path = Path(sys.argv[1])
i18n_dir = Path(sys.argv[2])
locales_path = Path(sys.argv[3])

PLACEHOLDER_RE = re.compile(r"\{[A-Za-z0-9_.-]+\}")
BACKTICK_RE = re.compile(r"`[^`]*`")

def placeholders(text):
    return Counter(PLACEHOLDER_RE.findall(text))

def literal_backticks(text):
    spans = []
    for span in BACKTICK_RE.findall(text):
        inner = span[1:-1]
        if PLACEHOLDER_RE.fullmatch(inner):
            continue
        spans.append(span)
    return spans

def same_literal_backticks(lhs, rhs):
    return Counter(literal_backticks(lhs)) == Counter(literal_backticks(rhs))

english = json.loads(en_path.read_text(encoding="utf-8"))
if not isinstance(english, dict):
    print(f"{en_path}: expected a top-level JSON object", file=sys.stderr)
    raise SystemExit(2)

locales = json.loads(locales_path.read_text(encoding="utf-8"))
for locale in locales:
    path = i18n_dir / f"{locale}.json"
    try:
        catalog = json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        print(f"{locale}: invalid JSON in {path}: {exc}", file=sys.stderr)
        print(locale)
        continue
    if not isinstance(catalog, dict):
        print(f"{locale}: expected a top-level JSON object in {path}", file=sys.stderr)
        print(locale)
        continue
    broken = False
    missing = len(set(english) - set(catalog))
    extra = len(set(catalog) - set(english))
    if missing or extra:
        broken = True
    for key, source in english.items():
        target = catalog.get(key)
        if not isinstance(target, str):
            broken = True
            continue
        if placeholders(source) != placeholders(target):
            broken = True
        if source.count("\n") != target.count("\n"):
            broken = True
        if not same_literal_backticks(source, target):
            broken = True
    if broken:
        print(locale)
PY
}

collect_incomplete_locales() {
  pending=()
  local line
  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    pending+=("$line")
  done < <(incomplete_locales)
}

run_translate() {
  require_locale_files
  local cmd
  cmd="$(translator_cmd)"
  local -a pending=()
  collect_incomplete_locales

  if [[ "${#pending[@]}" -eq 0 ]]; then
    echo "All locales are complete."
    return 0
  fi

  echo "Resuming translation from first incomplete locale: ${pending[0]}"
  echo "Pending locales: ${pending[*]}"

  local failures=()
  local locale
  for locale in "${pending[@]}"; do
    [[ "$locale" == "en" ]] && continue
    echo "Translating locale: $locale"
    if ! eval "$cmd --locale \"$LOCALE\" translate --langs \"$locale\" --en \"$EN_PATH\" --auth-mode \"$AUTH_MODE\" --batch-size \"$BATCH_SIZE\""; then
      failures+=("$locale")
      echo "Translation failed for locale: $locale" >&2
    fi
  done

  if [[ "${#failures[@]}" -gt 0 ]]; then
    echo "Translation failed for locales: ${failures[*]}" >&2
    return 1
  fi

  pending=()
  collect_incomplete_locales
  if [[ "${#pending[@]}" -gt 0 ]]; then
    echo "Translation incomplete; remaining locales: ${pending[*]}" >&2
    return 1
  fi
}

run_validate() {
  require_path "ci/i18n_check.py" "i18n checker"
  require_locale_files
  python3 ci/i18n_check.py validate
}

run_status() {
  require_path "ci/i18n_check.py" "i18n checker"
  require_locale_files
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
