#!/usr/bin/env bash
set -euo pipefail

# Developer helper (not run in CI) to refresh real component artifacts for realism ladder tests.
# It clones/fetches pinned repos, checks out the requested commit (if the repo is clean),
# builds if needed, then copies artifacts into tests/fixtures/real_components.
# Examples:
# - Refresh everything (git + OCI): ./tools/refresh_real_components.sh
# - Refresh only weatherapi via OCI (skip git): ONLY_OCI=1 ONLY_COMPONENTS=weatherapi OCI_PASSWORD=... ./tools/refresh_real_components.sh
# - Override default OCI artifact path: DEFAULT_WEATHERAPI_ARTIFACT=weatherapi/openweatherapi.component.wasm ONLY_OCI=1 ONLY_COMPONENTS=weatherapi ./tools/refresh_real_components.sh

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURES="$ROOT/tests/fixtures/real_components"
LOCK="$FIXTURES/lock.json"
COMPONENTS_ROOT_DEFAULT="$(cd "$ROOT/.." && pwd)"
COMPONENTS_ROOT="${COMPONENTS_ROOT:-$COMPONENTS_ROOT_DEFAULT}"
REQUIRED_COMPONENTS=("adaptive_card" "templates" "weatherapi")
GIT_BIN="${GIT_BIN:-git}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
ALLOW_BRANCH_FALLBACK="${ALLOW_BRANCH_FALLBACK:-1}"
BRANCH_FALLBACK="${BRANCH_FALLBACK:-master}"
TMP_ROOT="${TMP_ROOT:-${TMPDIR:-/tmp}/greentic-refresh-real}"
OCI_BIN="${OCI_BIN:-oras}"
OCI_USERNAME="${OCI_USERNAME:-${USER:-}}"
DEFAULT_WEATHERAPI_OCI_REF="${DEFAULT_WEATHERAPI_OCI_REF:-ghcr.io/greenticai/private/mcp-components/openweatherapi.component:latest}"
DEFAULT_WEATHERAPI_ARTIFACT="${DEFAULT_WEATHERAPI_ARTIFACT:-weatherapi/openweatherapi.component.wasm}"
ONLY_OCI="${ONLY_OCI:-0}"
ONLY_COMPONENTS="${ONLY_COMPONENTS:-}"

# In ONLY_OCI mode, do not fail the run for components that lack OCI refs.
if [[ "$ONLY_OCI" == "1" ]]; then
  is_required() { return 1; }
fi

log() { printf '[refresh] %s\n' "$*" >&2; }
warn() { printf '[refresh][warn] %s\n' "$*" >&2; }
fatal() { printf '[refresh][error] %s\n' "$*" >&2; exit 1; }

require_file() {
  local path="$1"
  [[ -f "$path" ]] || fatal "missing $path"
}

# Uppercase with underscores for env var overrides.
to_env_key() {
  echo "$1" | tr '[:lower:]-' '[:upper:]_'
}

is_required() {
  local name="$1"
  for req in "${REQUIRED_COMPONENTS[@]}"; do
    [[ "$req" == "$name" ]] && return 0
  done
  return 1
}

ensure_git() {
  if ! command -v "$GIT_BIN" >/dev/null 2>&1; then
    fatal "git is required to clone pinned repos (override path with GIT_BIN)"
  fi
}

ensure_python() {
  if command -v "$PYTHON_BIN" >/dev/null 2>&1; then
    return
  fi
  if [[ "$PYTHON_BIN" != "python" ]] && command -v python >/dev/null 2>&1; then
    PYTHON_BIN="python"
    return
  fi
  fatal "python3/python is required to parse lock.json (set PYTHON_BIN to override)"
}

ensure_oci() {
  if ! command -v "$OCI_BIN" >/dev/null 2>&1; then
    warn "OCI client '$OCI_BIN' not found; skipping OCI pulls"
    return 1
  fi
  if [[ -z "${OCI_PASSWORD:-}" ]]; then
    warn "OCI_PASSWORD not set; OCI pulls will be skipped"
    return 1
  fi
  return 0
}

resolve_repo_dir() {
  local name="$1"
  local repo="$2"
  local key
  key="$(to_env_key "$name")"
  local override_var="COMPONENT_${key}_DIR"
  local override="${!override_var:-}"
  if [[ -n "$override" ]]; then
    echo "$override"
    return
  fi
  local base
  base="$(basename "$repo")"
  base="${base%.git}"
  local candidate="$COMPONENTS_ROOT/$base"
  if [[ -d "$candidate/.git" ]]; then
    echo "$candidate"
    return
  fi
  echo "$repo"
}

prepare_checkout() {
  local name="$1"
  local repo_source="$2"
  local target="$3"

  mkdir -p "$TMP_ROOT"
  local checkout_dir
  checkout_dir="$(mktemp -d "$TMP_ROOT/${name}.XXXX")" || return 1

  log "$name: cloning $repo_source -> $checkout_dir"
  if ! GIT_TERMINAL_PROMPT=0 "$GIT_BIN" clone --filter=blob:none "$repo_source" "$checkout_dir" >&2; then
    warn "$name: clone failed from $repo_source"
    rm -rf "$checkout_dir"
    return 1
  fi

  if [[ -n "$target" && "$target" != TODO* ]]; then
    if ! "$GIT_BIN" -C "$checkout_dir" cat-file -e "$target^{commit}" >/dev/null 2>&1; then
      log "$name: fetching commit/branch $target"
      if ! "$GIT_BIN" -C "$checkout_dir" fetch --depth 1 origin "$target" >&2; then
        warn "$name: shallow fetch of $target failed, retrying full fetch"
        if ! "$GIT_BIN" -C "$checkout_dir" fetch origin "$target" >&2; then
          warn "$name: fetch of $target failed; try manual fetch"
          rm -rf "$checkout_dir"
          return 1
        fi
      fi
    fi

    log "$name: checking out $target"
    if ! "$GIT_BIN" -C "$checkout_dir" checkout --detach "$target" >&2; then
      warn "$name: checkout of $target failed"
      rm -rf "$checkout_dir"
      return 1
    fi
  fi

  echo "$checkout_dir"
}

fetch_from_oci() {
  local name="$1"
  local oci_ref="$2"
  local dest_dir="$3"
  local dest_filename="$4"

  ensure_oci || return 1
  mkdir -p "$dest_dir"
  local tmp_pull
  tmp_pull="$(mktemp -d "$TMP_ROOT/${name}-oci.XXXX")" || return 1
  log "$name: pulling OCI artifact $oci_ref via $OCI_BIN"
  (
    cd "$tmp_pull" || exit 1
    if ! OCI_USERNAME="$OCI_USERNAME" OCI_PASSWORD="$OCI_PASSWORD" "$OCI_BIN" pull "$oci_ref" --output . >&2; then
      warn "$name: oras pull --output failed; retrying without --output"
      if ! OCI_USERNAME="$OCI_USERNAME" OCI_PASSWORD="$OCI_PASSWORD" "$OCI_BIN" pull "$oci_ref" >&2; then
        exit 42
      fi
    fi
  )
  rc=$?
  if [[ $rc -eq 42 || $rc -ne 0 ]]; then
    warn "$name: OCI pull failed for $oci_ref"
    rm -rf "$tmp_pull"
    return 1
  fi

  local candidate
  candidate="$(find "$tmp_pull" -type f -name "$dest_filename" 2>/dev/null | head -n 1)"
  if [[ -z "$candidate" ]]; then
    candidate="$(find "$tmp_pull" -type f -name '*.wasm' 2>/dev/null | head -n 1)"
  fi
  if [[ -z "$candidate" ]]; then
    candidate="$(find "$tmp_pull/blobs" -type f 2>/dev/null | head -n 1)"
  fi
  if [[ -z "$candidate" ]]; then
    warn "$name: OCI pull succeeded but no usable artifact found"
    rm -rf "$tmp_pull"
    return 1
  fi

  cp "$candidate" "$dest_dir/$dest_filename"
  rm -rf "$tmp_pull"
  log "$name: downloaded from OCI to $dest_dir/$dest_filename"
  echo "$dest_dir/$dest_filename"
}

fetch_from_oci_layout() {
  local name="$1"
  local oci_ref="$2"
  local dest_dir="$3"
  local dest_filename="$4"

  ensure_oci || return 1
  mkdir -p "$dest_dir"
  local tmp_layout
  tmp_layout="$(mktemp -d "$TMP_ROOT/${name}-layout.XXXX")" || return 1

  log "$name: pulling OCI artifact via OCI layout from $oci_ref"
  if ! OCI_USERNAME="$OCI_USERNAME" OCI_PASSWORD="$OCI_PASSWORD" \
        "$OCI_BIN" copy "$oci_ref" --to-oci-layout "$tmp_layout:latest" >&2; then
    warn "$name: OCI layout copy failed for $oci_ref"
    rm -rf "$tmp_layout"
    return 1
  fi

  # Prefer the largest blob as a heuristic; avoids manifest/config when unnamed.
  local blob
  blob="$(find "$tmp_layout/blobs" -type f 2>/dev/null \
        | while read -r f; do printf '%s\t%s\n' "$(wc -c <"$f")" "$f"; done \
        | sort -nr | head -n 1 | cut -f2-)"
  if [[ -z "$blob" ]]; then
    warn "$name: OCI layout copy succeeded but no blobs found"
    rm -rf "$tmp_layout"
    return 1
  fi

  # Avoid tiny blobs (manifest/config); require >50KB.
  local size
  size="$(wc -c <"$blob")"
  if [[ "$size" -lt 51200 ]]; then
    warn "$name: largest blob too small ($size bytes); refusing to copy"
    rm -rf "$tmp_layout"
    return 1
  fi

  cp "$blob" "$dest_dir/$dest_filename"
  rm -rf "$tmp_layout"
  log "$name: downloaded from OCI layout to $dest_dir/$dest_filename"
  echo "$dest_dir/$dest_filename"
}

compute_hash() {
  local path="$1"
  if command -v b3sum >/dev/null 2>&1; then
    b3sum "$path" | awk '{print $1}'
  else
    echo ""
  fi
}

find_artifact() {
  local repo_dir="$1"
  local filename="$2"
  # Accept variants produced by cargo-component (e.g. component_<name>.wasm).
  local patterns=("$filename" "component_*.wasm" "*.component.wasm" "component.wasm" "*.wasm")
  local targets=(
    "target/wasm32-wasip2/release"
    "target/wasm32-wasip1/release"
    "target/wasm32-wasi/release"
  )
  for tgt in "${targets[@]}"; do
    for pat in "${patterns[@]}"; do
      local found
      found="$(find "$repo_dir/$tgt" -maxdepth 1 -type f -name "$pat" 2>/dev/null | head -n 1)"
      [[ -n "$found" ]] && { echo "$found"; return 0; }
    done
  done
  local found
  for pat in "${patterns[@]}"; do
    found="$(find "$repo_dir" -maxdepth 4 -type f -name "$pat" 2>/dev/null | head -n 1)"
    [[ -n "$found" ]] && { echo "$found"; return 0; }
  done
  return 1
}

attempt_build() {
  local repo_dir="$1"
  local target="${WASM_TARGET:-wasm32-wasip2}"
  if ! command -v cargo >/dev/null 2>&1; then
    warn "cargo not available; skipping build in $repo_dir"
    return 1
  fi
  if command -v cargo-component >/dev/null 2>&1; then
    (cd "$repo_dir" && cargo component build --release --target "$target")
  else
    warn "cargo-component not installed; falling back to cargo build --release --target $target"
    (cd "$repo_dir" && cargo build --release --target "$target")
  fi
}

require_file "$LOCK"
ensure_git
ensure_python
log "Refreshing real component artifacts into $FIXTURES"
log "Using COMPONENTS_ROOT=${COMPONENTS_ROOT}"

COMPONENTS=()
while IFS=$'\t' read -r name repo commit artifact; do
  [[ -z "$name" ]] && continue
  COMPONENTS+=("$name"$'\t'"$repo"$'\t'"$commit"$'\t'"$artifact")
done < <("$PYTHON_BIN" - "$LOCK" <<'PY'
import json, sys
from pathlib import Path

lock = Path(sys.argv[1])
data = json.loads(lock.read_text())
for name, entry in data.items():
    repo = entry.get("repo", "")
    commit = entry.get("commit", "")
    artifact = entry.get("artifact", "")
    print("\t".join([name, repo or "", commit or "", artifact or ""]))
PY
)

if [[ "${#COMPONENTS[@]}" -eq 0 ]]; then
  fatal "no components found in $LOCK"
fi

status=0
for line in "${COMPONENTS[@]}"; do
  IFS=$'\t' read -r name repo commit artifact <<<"$line"
  if [[ -n "$ONLY_COMPONENTS" ]]; then
    IFS=',' read -r -a only_list <<<"$ONLY_COMPONENTS"
    keep=0
    for x in "${only_list[@]}"; do
      [[ "$x" == "$name" ]] && keep=1
    done
    if [[ $keep -ne 1 ]]; then
      log "$name: skipped due to ONLY_COMPONENTS"
      continue
    fi
  fi
  if [[ "$name" == "weatherapi" && -z "${artifact:-}" ]]; then
    artifact="$DEFAULT_WEATHERAPI_ARTIFACT"
    log "$name: lock.json artifact empty; using default artifact path '$artifact'"
  fi
  dest="$FIXTURES/$artifact"
  dest_dir="$(dirname "$dest")"
  filename="$(basename "$artifact")"
  if [[ -z "$filename" || "$filename" == "/" ]]; then
    warn "$name: invalid artifact path in lock.json: '$artifact' (must include a filename)"
    is_required "$name" && status=1
    continue
  fi
  oci_ref_env="OCI_REF_$(to_env_key "$name")"
  oci_ref="${!oci_ref_env:-}"
  if [[ -z "$oci_ref" && "$name" == "weatherapi" ]]; then
    oci_ref="$DEFAULT_WEATHERAPI_OCI_REF"
  fi
  if [[ "$ONLY_OCI" == "1" && -z "$oci_ref" ]]; then
    log "$name: ONLY_OCI=1 and no OCI ref; skipping component"
    continue
  fi
  if [[ "$name" == "weatherapi" && -z "$oci_ref" ]]; then
    warn "$name: OCI ref required but missing for OCI-only component"
    status=1
    continue
  fi

  commit_target="$commit"
  skip_checkout=false
  if [[ "$ONLY_OCI" == "1" ]]; then
    skip_checkout=true
    log "$name: ONLY_OCI=1; skipping git checkout/build"
  elif [[ "$name" == "weatherapi" ]]; then
    skip_checkout=true
    log "$name: OCI-only component; skipping git checkout/build"
  elif [[ -z "$repo" ]]; then
    skip_checkout=true
    log "$name: no repo provided; skipping checkout (expecting OCI artifact)"
  elif [[ -z "$commit_target" || "$commit_target" == TODO* ]]; then
    if [[ "$ALLOW_BRANCH_FALLBACK" == "1" ]]; then
      commit_target="$BRANCH_FALLBACK"
      log "$name: commit TODO/empty; using branch fallback '$commit_target'"
    else
      warn "$name: commit is TODO; populate lock.json or set ALLOW_BRANCH_FALLBACK=1"
      is_required "$name" && status=1
      continue
    fi
  fi

  repo_dir=""
  repo_source="$(resolve_repo_dir "$name" "$repo")"
  artifact_path=""

  if ! $skip_checkout; then
    repo_dir="$(prepare_checkout "$name" "$repo_source" "$commit_target" || true)"
    if [[ -n "$repo_dir" ]]; then
      head_commit="$("$GIT_BIN" -C "$repo_dir" rev-parse HEAD || true)"
      if [[ -n "$head_commit" ]]; then
        if [[ -n "$commit" && "$commit" != TODO* && "$head_commit" != "$commit"* ]]; then
          warn "$name: HEAD $head_commit does not match lock commit $commit (build will still continue)"
        elif [[ "$commit_target" != "$head_commit" && "$commit_target" != "$commit" ]]; then
          log "$name: using fallback target $commit_target (HEAD $head_commit)"
        fi
      fi

      artifact_path="$(find_artifact "$repo_dir" "$filename" || true)"
      if [[ -z "$artifact_path" ]]; then
        log "$name: building artifact (target ${WASM_TARGET:-wasm32-wasip2})"
        if ! attempt_build "$repo_dir"; then
          warn "$name: build failed; will try OCI if available"
        else
          artifact_path="$(find_artifact "$repo_dir" "$filename" || true)"
        fi
      fi
    else
      if [[ -n "$oci_ref" ]]; then
        log "$name: checkout failed; trying OCI ref $oci_ref"
      else
        warn "$name: checkout failed and no OCI ref; skipping"
        is_required "$name" && status=1
        continue
      fi
    fi
  fi

  if [[ -z "$artifact_path" && -n "$oci_ref" ]]; then
    log "$name: attempting OCI pull from $oci_ref"
    if [[ "$name" == "weatherapi" ]]; then
      artifact_path="$(fetch_from_oci_layout "$name" "$oci_ref" "$dest_dir" "$filename" || true)"
    else
      artifact_path="$(fetch_from_oci "$name" "$oci_ref" "$dest_dir" "$filename" || true)"
    fi
  fi

  if [[ -z "$artifact_path" ]]; then
    warn "$name: artifact $filename not found"
    is_required "$name" && status=1
    continue
  fi

  mkdir -p "$dest_dir"
  cp "$artifact_path" "$dest"
  hash="$(compute_hash "$dest")"
  log "$name: copied $(realpath "$artifact_path") -> $dest"
  log "$name: HEAD commit $head_commit"
  if [[ -n "$hash" ]]; then
    log "$name: blake3 $hash (update lock.json hash_blake3)"
  else
    warn "b3sum not available; hash not computed"
  fi

  # Clean up temporary checkout when we created one under TMP_ROOT.
  if [[ -n "$repo_dir" && "$repo_dir" == "$TMP_ROOT/"* ]]; then
    rm -rf "$repo_dir"
  fi
done

if [[ $status -ne 0 ]]; then
  fatal "one or more required artifacts missing; see warnings above"
fi
log "done"

