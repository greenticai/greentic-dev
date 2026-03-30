#!/usr/bin/env bash
set -euo pipefail

export CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}"
export CARGO_NET_RETRY="${CARGO_NET_RETRY:-10}"
export CARGO_HTTP_CHECK_REVOKE="${CARGO_HTTP_CHECK_REVOKE:-false}"

SUCCESS_EXIT_CODE=0
POLICY_MISSING_EXIT_CODE=2
SETUP_FAILURE_EXIT_CODE=3
RUN_FAILURE_EXIT_CODE=4
POLICY_FAILURE_EXIT_CODE=5

POLICY_FILE="${COVERAGE_POLICY_FILE:-coverage-policy.json}"
REPORT_DIR="${COVERAGE_REPORT_DIR:-target/coverage}"
REPORT_FILE="${COVERAGE_REPORT_FILE:-${REPORT_DIR}/coverage.json}"
COVERAGE_SKIP_RUN="${COVERAGE_SKIP_RUN:-false}"

if command -v python3 >/dev/null 2>&1; then
  PYTHON_BIN="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON_BIN="python"
else
  echo "[coverage] python is required to evaluate ${POLICY_FILE}" >&2
  exit "${SETUP_FAILURE_EXIT_CODE}"
fi

OFFLINE_FLAG=""
LOCKED_FLAG="--locked"
if [[ "${CARGO_NET_OFFLINE:-false}" == "true" ]]; then
  OFFLINE_FLAG="--offline"
  LOCKED_FLAG=""
fi

log() {
  echo "[coverage] $*"
}

print_policy_missing_instructions() {
  cat <<EOF
[coverage] missing policy file: ${POLICY_FILE}
[coverage] Codex instructions:
Create ${POLICY_FILE} at the repository root with:
- a global line coverage minimum
- a default per-file line coverage minimum
- an explicit exclusion list for generated code or thin entrypoints
- per-file overrides for high-risk modules that need stricter targets
Suggested starting point:
{
  "version": 1,
  "global": { "line_coverage_min": 60.0 },
  "defaults": { "per_file_line_coverage_min": 60.0 },
  "exclusions": { "files": [] },
  "per_file": {}
}
EOF
}

ensure_binstall() {
  if command -v cargo-binstall >/dev/null 2>&1; then
    return 0
  fi
  if [[ -n "${OFFLINE_FLAG}" ]]; then
    echo "[coverage] cargo-binstall is required but offline mode is enabled" >&2
    return 1
  fi
  log "installing cargo-binstall"
  cargo install ${LOCKED_FLAG} ${OFFLINE_FLAG} cargo-binstall
}

ensure_tool() {
  local bin="$1"
  local package="$2"
  if command -v "${bin}" >/dev/null 2>&1; then
    return 0
  fi
  ensure_binstall
  if [[ -n "${OFFLINE_FLAG}" ]]; then
    echo "[coverage] missing ${package} but offline mode is enabled" >&2
    return 1
  fi
  log "installing ${package}"
  cargo binstall ${LOCKED_FLAG} ${OFFLINE_FLAG} -y "${package}"
}

ensure_llvm_tools() {
  if ! command -v rustup >/dev/null 2>&1; then
    echo "[coverage] rustup is required to add llvm-tools-preview" >&2
    return 1
  fi
  if rustup component list --installed | grep -q '^llvm-tools-preview'; then
    return 0
  fi
  if [[ -n "${OFFLINE_FLAG}" ]]; then
    echo "[coverage] llvm-tools-preview is missing and offline mode is enabled" >&2
    return 1
  fi
  log "installing llvm-tools-preview"
  rustup component add llvm-tools-preview
}

if [[ ! -f "${POLICY_FILE}" ]]; then
  print_policy_missing_instructions
  exit "${POLICY_MISSING_EXIT_CODE}"
fi

log "ensuring coverage tools are installed"
if [[ "${COVERAGE_SKIP_RUN}" != "true" ]]; then
  if ! ensure_tool cargo-llvm-cov cargo-llvm-cov; then
    exit "${SETUP_FAILURE_EXIT_CODE}"
  fi
  if ! ensure_tool cargo-nextest cargo-nextest; then
    exit "${SETUP_FAILURE_EXIT_CODE}"
  fi
  if ! ensure_llvm_tools; then
    exit "${SETUP_FAILURE_EXIT_CODE}"
  fi
fi

mkdir -p "${REPORT_DIR}"

if [[ "${COVERAGE_SKIP_RUN}" == "true" ]]; then
  log "skipping coverage run and reusing ${REPORT_FILE}"
else
  log "running cargo llvm-cov nextest"
  if ! cargo llvm-cov nextest --ignore-run-fail --json --output-path "${REPORT_FILE}" --workspace --all-features; then
    echo "[coverage] coverage command failed before policy evaluation" >&2
    exit "${RUN_FAILURE_EXIT_CODE}"
  fi
fi

if [[ ! -f "${REPORT_FILE}" ]]; then
  echo "[coverage] expected coverage report missing: ${REPORT_FILE}" >&2
  exit "${RUN_FAILURE_EXIT_CODE}"
fi

log "evaluating policy from ${POLICY_FILE}"
if ! "${PYTHON_BIN}" - "${POLICY_FILE}" "${REPORT_FILE}" <<'PY'
import json
import pathlib
import sys

policy_path = pathlib.Path(sys.argv[1])
report_path = pathlib.Path(sys.argv[2])

policy = json.loads(policy_path.read_text())
report = json.loads(report_path.read_text())

files = report.get("data", [{}])[0].get("files") or report.get("files") or []
totals = report.get("data", [{}])[0].get("totals") or report.get("totals") or {}

global_min = float(policy.get("global", {}).get("line_coverage_min", 0.0))
default_per_file_min = float(
    policy.get("defaults", {}).get("per_file_line_coverage_min", global_min)
)

excluded_entries = policy.get("exclusions", {}).get("files", [])
excluded_paths = set()
for entry in excluded_entries:
    if isinstance(entry, dict) and entry.get("path"):
        excluded_paths.add(entry["path"])
    elif isinstance(entry, str):
        excluded_paths.add(entry)

per_file = policy.get("per_file", {})

repo_root = pathlib.Path.cwd().resolve()

effective_line_count = 0
effective_line_covered = 0

violations = []

for file_entry in files:
    raw_filename = file_entry.get("filename", "")
    if not raw_filename:
      continue
    path = pathlib.Path(raw_filename)
    try:
        rel_path = path.resolve().relative_to(repo_root).as_posix()
    except Exception:
        rel_path = raw_filename

    if rel_path in excluded_paths:
        continue

    line_summary = file_entry.get("summary", {}).get("lines", {})
    actual = float(line_summary.get("percent", 0.0))
    effective_line_count += int(line_summary.get("count", 0))
    effective_line_covered += int(line_summary.get("covered", 0))
    expected = float(per_file.get(rel_path, {}).get("line_coverage_min", default_per_file_min))

    if actual < expected:
        violations.append(
            f"{rel_path} line coverage {actual:.2f}% is below required minimum {expected:.2f}%"
        )

if effective_line_count == 0:
    line_percent = float(totals.get("lines", {}).get("percent", 0.0))
else:
    line_percent = (effective_line_covered / effective_line_count) * 100.0

if line_percent < global_min:
    violations.insert(
        0,
        f"workspace line coverage {line_percent:.2f}% is below global minimum {global_min:.2f}%",
    )

if violations:
    print("[coverage] policy check failed")
    print("[coverage] Codex instructions:")
    print("Increase test coverage for the files below or update the exclusion list only for generated code, tooling entrypoints, or thin wiring layers.")
    print("Do not lower thresholds to make the report pass unless the team intentionally changes the policy.")
    print("[coverage] violations:")
    for violation in violations:
        print(f"- {violation}")
    sys.exit(1)

print("[coverage] policy check passed")
print(f"[coverage] workspace line coverage: {line_percent:.2f}%")
PY
then
  exit "${POLICY_FAILURE_EXIT_CODE}"
fi

log "success"
log "report written to ${REPORT_FILE}"
exit "${SUCCESS_EXIT_CODE}"
