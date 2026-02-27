# Real component artifacts for realism ladder tests

These files are committed so CI can run deterministically without downloading from git.

Refresh guidance (developer-only, not run in CI):
1) Populate `lock.json` with pinned repo + commit + hash_blake3 values (leave as TODO only if intentionally missing). By default the script will fall back to the `master` branch when commit is TODO/empty (override with `ALLOW_BRANCH_FALLBACK=0` or `BRANCH_FALLBACK=<branch>`).
2) Run `tools/refresh_real_components.sh`. It clones into a temp directory (under `./.tmp/refresh-real`), fetches the pinned commit (or fallback branch), builds if missing, copies artifacts here, and prints blake3 hashes. Local repo overrides (`COMPONENT_<NAME>_DIR` or `COMPONENTS_ROOT`) are used as clone sources; the temp checkout is always removed afterward. If `OCI_PASSWORD` is set and `oras` is available, the script will also try to pull missing artifacts from OCI when an `OCI_REF_<NAME>` env var is present (weatherapi defaults to `ghcr.io/greenticai/private/mcp-components/openweatherAPI.component.wasm:latest`).
3) If a repo has local changes, point the script at a clean clone via `COMPONENT_<NAME>_DIR` (or stash/commit) so it can check out the pinned commit.
4) Update `lock.json` with the printed hashes (and confirm commit SHAs), then commit the artifacts + lock.json.

Notes:
- adaptive_card/templates are required for tests; weatherapi is optional and tests will skip if missing.
- The refresh script is non-destructive to dirty repos: if it detects local changes it will skip checkout rather than clobber them.
- `COMPONENTS_ROOT` can override the parent directory search if your clones live elsewhere; per-repo overrides use `COMPONENT_<NAME>_DIR`.

Current expectations:
- `adaptive_card/component.wasm` — adaptive card component
- `templates/component.wasm` — templates component
- `weatherapi/weatherapi.wasm` — WeatherAPI tool (to be provided)
- `lock.json` — pinned source metadata (repo, commit, filename, hash)

If an artifact is missing locally, weather tests will auto-skip until provided.

