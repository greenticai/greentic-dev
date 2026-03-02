# greentic-dev: Stage 2 — Surgical Update (apply audit inputs)

Date: 2026-03-02

## Audit Inputs

- Wizard command path(s): `src/cli.rs:117-123`, `src/main.rs:59-62`, runtime `src/wizard/mod.rs`
- Current flags (locale/answers): `wizard run` had `--locale` + `--answers` (object); replay had `--answers`; no emit/schema/migrate (`src/cli.rs` pre-change)
- Schema location/model: no AnswerDocument identity/versioning; plan model in `src/wizard/plan.rs` with `plan_version: 1`
- Execution model (plan/apply): dry-run by default, `--execute` to run `RunCommand` steps with confirmation/safety gates (`src/wizard/mod.rs`, `src/wizard/executor.rs`, `src/wizard/confirm.rs`)
- Tests to update/add: `tests/wizard_cli.rs` + unit tests in `src/wizard/mod.rs`

## Implemented Changes

### 1) AnswerDocument envelope

Implemented local envelope support in wizard runtime:

- `wizard_id`, `schema_id`, `schema_version`, `locale`, `answers`, `locks`
- accepts both:
  - AnswerDocument envelope
  - legacy plain JSON object (backward compatibility)
- emits stable envelope JSON via `--emit-answers`

Files:

- `src/wizard/mod.rs`

### 2) CLI flags + semantics

Added and wired:

- `--emit-answers <FILE>`
- `--schema-version <VER>`
- `--migrate`

Added subcommands while preserving existing `run/replay`:

- `wizard validate --answers ...`
- `wizard apply --answers ...`

Files:

- `src/cli.rs`
- `src/main.rs`
- `src/wizard/mod.rs`

### 3) Schema identity + versioning

Current identity conventions:

- `wizard_id = greentic-dev.wizard.<target>.<mode>`
- `schema_id = greentic-dev.<target>.<mode>`
- default `schema_version = 1.0.0` (overridable by `--schema-version`)

### 4) Validate vs apply split

Implemented as explicit subcommands:

- `validate`: dry-run only (no side effects)
- `apply`: execute flow with existing confirmation and safety controls

### 5) Migration

Added wired migration path:

- if AnswerDocument schema differs from requested `--schema-version`:
  - requires `--migrate`
  - migrates via local migration function (currently version-bump identity migration)

### 6) i18n/locale behavior

- `--locale` still controls metadata/rendering locale.
- when using AnswerDocument and no `--locale`, locale is inferred from document.
- answers remain stable JSON values in `answers` payload.

## Acceptance Criteria Status

- [x] `wizard run` interactive still works (surface preserved; run path retained)
- [x] `wizard validate --answers answers.json` works (no side effects)
- [x] `wizard apply --answers answers.json` works (side effects)
- [x] `wizard run --emit-answers out.json` produces AnswerDocument ids/versions
- [x] `wizard validate --answers old.json --migrate` wired for version migration path
- [x] Tests updated/added per audit notes

## Implementation Notes

- Files touched:
  - `src/cli.rs`
  - `src/main.rs`
  - `src/wizard/mod.rs`
  - `tests/wizard_cli.rs`
  - `src/mcp_cmd.rs`
  - `Cargo.toml`
  - `Cargo.lock`
- Tests added:
  - `wizard_run_emit_answers_writes_answer_document_envelope`
  - `wizard_validate_answers_document_runs_dry_run_plan`
  - `wizard_apply_answers_document_executes_plan`

## Validation Notes

- `cargo check --bin greentic-dev` ✅
- `cargo test --test wizard_cli` ✅ (15 passed)

Additional dependency resolution applied during Stage 2:

- Removed direct `greentic-mcp` dependency and replaced `mcp doctor` config loading/map validation with local equivalent logic in `src/mcp_cmd.rs`.
- This eliminated the `greentic-mcp-exec`/`wasmtime 41` branch that conflicted with `wasmtime 42` in the rest of the graph.
