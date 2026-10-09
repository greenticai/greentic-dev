# Design: Wire `greentic-mcp-generator` into `greentic-dev`

- **Date:** 2026-07-01
- **Status:** Approved (design), pending implementation plan
- **Repo:** `greentic-dev`
- **Related repo:** `greentic-biz/greentic-mcp-generator` (binary `greentic-mcp-gen`, v1.1.0)

## Problem

`greentic-mcp-generator` turns an OpenAPI/Swagger spec (or a Google Discovery
doc) into a `wasix:mcp@25.06.18` router component compiled to `wasm32-wasip2`.
Today it is a standalone tool: no Greentic CLI reaches it. `gtc` only mentions
it in architecture docs; `greentic-dev` only references it in `#[cfg(test)]`
fixtures; `greentic-designer-sdk` (`gtdx`) can scaffold an empty
`wasix:mcp/router` skeleton but cannot derive one from a spec.

We want the developer cockpit `greentic-dev` to expose the generator so authors
can go from an OpenAPI spec to an MCP router component through one entrypoint,
without leaving the cockpit workflow.

## Goals

- Reach the generator from `greentic-dev` as a first-class, discoverable command.
- Preserve `greentic-dev`'s role as a **thin wrapper** over canonical CLIs — no
  code-gen logic, no OpenAPI/wasm dependencies pulled into `greentic-dev`.
- Zero-maintenance flag surface: forward everything to the generator verbatim so
  new generator flags need no `greentic-dev` change.
- Registered, guided install path; helpful failure when the binary is absent.

## Non-goals (YAGNI)

- No library / in-process generation (do not depend on
  `greentic-mcp-generator-core`).
- No silent auto-install of the generator binary.
- No re-declaration or curation of the generator's flags.
- No blocking wasm-toolchain preflight — `mcp doctor` reports readiness, it does
  not gate `mcp gen`.

## Decisions (locked during brainstorming)

| Decision | Choice |
| --- | --- |
| Integration mechanism | Passthrough to installed binary `greentic-mcp-gen` |
| Feature scope | Full passthrough of argv (spec, batch, discovery, discovery-to-openapi, profiles, mcp-test, `--upload`/OCI) |
| Command placement | Extend existing `mcp` subcommand → `greentic-dev mcp gen` |
| Missing-binary behavior | Resolve (env override → PATH), else fail with guided install message; **no** auto-install |
| Supporting integration | Register in install catalogue; extend `mcp doctor`; all new strings via i18n |

## Architecture

`greentic-dev` treats the generator exactly like the other delegated CLIs
(`component`, `flow`, `pack`, `runner`): **resolve → run passthrough**. The only
new logic is a command node, a binary-resolution path that tolerates the
generator's naming, a catalogue entry, and doctor reporting.

```
greentic-dev mcp gen [ARGS...]
        │
        ▼
 resolve_binary("greentic-mcp-gen")   # env override → PATH (channel-agnostic, see gotcha)
        │  found?
        ├── no  → guided install error (i18n), non-zero exit
        └── yes → run_passthrough(bin, [ARGS...])   # exec argv, no shell
                        │
                        ▼
                greentic-mcp-gen <ARGS...>   # exit code propagated verbatim
```

## Components / changes

### 1. Command surface — `src/cli.rs`
- Add a `Gen` variant to `McpCommand` (alongside `Doctor`).
- In the manually-built clap tree (`mut_subcommand("mcp", ...)`), add a `gen`
  subcommand configured with `trailing_var_arg(true)` + `allow_hyphen_values(true)`
  so **all** tokens after `gen` are captured raw into a `Vec<OsString>`.
- About/help text via `i18n::t` (`cli.command.mcp.gen.about`).
- Accepted consequence: `greentic-dev mcp gen --help` shows the *generator's*
  help, not `greentic-dev`'s. Document this in the about string.

### 2. Binary resolution — `src/passthrough.rs`
- Reuse `resolve_binary` / `run_passthrough`.
- **Gotcha — channel suffixing.** `delegated_binary_name_for_channel` appends
  `-dev` / `-rnd` to delegated binary names. Canonical Greentic CLIs ship those
  suffixed variants; the generator ships only `greentic-mcp-gen` (no suffix). On
  a `dev`/`rnd` toolchain channel the default resolver would look for
  `greentic-mcp-gen-dev` and fail.
  **Resolution:** resolve the generator by its **plain, channel-agnostic name**.
  Introduce a minimal resolver path (e.g. `resolve_external_tool(name)` that
  checks `GREENTIC_DEV_BIN_<UPPER>` env override, then plain `which(name)`, with
  no channel suffix), or mark the catalogue entry channel-agnostic and branch on
  it. Rejected alternative (cross-repo, deferred): make the generator publish
  channel-suffixed binaries.
- Env override key follows the existing pattern: `GREENTIC_DEV_BIN_GREENTIC_MCP_GEN`.

### 3. Install registration — `src/toolchain_catalogue.rs` (+ `src/install.rs` as needed)
- Add the generator so `greentic-dev install` can pull it. Because it is private
  and unsuffixed, the entry is marked channel-agnostic (does not go through the
  suffixing binstall path used by the canonical tools, or is flagged to skip
  suffixing).
- Crate `greentic-mcp-generator`, bin `greentic-mcp-gen`.
- Private distribution: install requires `GITHUB_TOKEN` (reuse `install.rs`
  token resolution). MVP does not auto-install on `mcp gen`; it only makes
  `greentic-dev install` aware of the tool and powers the guided error message.

### 4. `mcp doctor` extension — `src/mcp_cmd.rs`
- Keep existing tool-map health check.
- Add a report section:
  - `greentic-mcp-gen` present? path + `--version` output.
  - Best-effort wasm toolchain readiness: `cargo` present + `wasm32-wasip2`
    target installed (`rustup target list --installed` or equivalent). Best-effort
    only — a missing target is a warning, never a hard error.
- Honor the existing `--json` flag; extend the report struct with the new fields.

### 5. i18n — locale files + `i18n::t`
- New keys: `cli.command.mcp.gen.about`, guided-install error message,
  doctor report lines. Added to every locale file with key parity.

## Data flow

1. User runs `greentic-dev mcp gen --spec ./api.yaml --output-dir ./out`.
2. `cli.rs` routes to `McpCommand::Gen(argv)` with `argv = ["--spec", "./api.yaml", "--output-dir", "./out"]`.
3. Handler resolves `greentic-mcp-gen` (env → PATH, channel-agnostic).
4. If missing → print guided i18n error (`cargo binstall greentic-mcp-generator`
   + `GITHUB_TOKEN`, or `greentic-dev install`), exit non-zero.
5. If found → `run_passthrough(bin, argv)`; the generator does the real work
   (scaffold temp crate, `cargo build --target wasm32-wasip2`, emit `.wasm`,
   optional OCI upload). Its stdout/stderr and **exit code** pass through
   unchanged.

## Error handling

- **Binary absent:** non-zero exit + guided install message (i18n). No panic.
- **Binary present, non-zero exit:** propagate the generator's exit code verbatim;
  do not swallow or remap.
- **Security:** reuse the passthrough "accepted risk" pattern — exec a resolved
  tool by argv, never through a shell. No new shell invocation.
- No `unwrap()`/`panic!()` on the new paths; `anyhow` context on resolution/exec.

## Testing

- **Unit:**
  - argv forwarded verbatim, including leading `--`, hyphen values, and
    subcommand-style args (`discovery`, `--upload`).
  - channel-agnostic resolution: generator resolves by plain name under `dev`/`rnd`
    channels (regression guard for the suffix gotcha).
  - catalogue contains the generator entry (bin `greentic-mcp-gen`).
  - guided error emitted (correct i18n key) when the binary is unresolved.
- **i18n:** key-parity across locales for the new keys.
- **doctor:** JSON report includes the new generator/toolchain fields; text output
  renders them.
- **Gate:** `bash ci/local_check.sh` (fmt + clippy `-D warnings` + tests).

## Docs / process

- Update `README.md` (greentic-dev) with the `mcp gen` usage and the install/token
  note.
- PRE-PR and POST-PR sync of `.codex/repo_overview.md`.
- Conventional Commits; single PR against the repo's integration branch.

## Open questions

- None blocking. The channel-suffix resolution approach (dedicated resolver vs.
  catalogue flag) is an implementation detail to be settled in the plan.
