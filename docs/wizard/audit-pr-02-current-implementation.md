# Wizard Audit (PR-02): Current Implementation Baseline

Date: 2026-03-02
Scope: `greentic-dev` wizard implementation in this repository (`src/wizard/*`, CLI wiring, wizard tests/docs)

## Audit Goal

Establish an exact behavioral baseline of the current wizard implementation before surgical changes in PR-03.

## What Exists Today

### CLI Surface and Dispatch

- Top-level command wiring exists in `src/cli.rs` and `src/main.rs`:
  - `wizard run`
  - `wizard replay`
- `main.rs` delegates wizard commands directly to:
  - `wizard::run(args)`
  - `wizard::replay(args)`

### Execution Mode Rules

Implemented in `src/wizard/mod.rs` (`resolve_execution_mode`):

- Default mode is dry-run when neither `--dry-run` nor `--execute` is provided.
- `--dry-run` + `--execute` together are rejected with:
  - `Choose one of --dry-run or --execute.`

### Target/Mode Registry

Implemented in `src/wizard/registry.rs` as static string mapping.

Supported keys:

- `operator.create`
- `pack.create`
- `pack.build`
- `component.scaffold`
- `component.build`
- `flow.create`
- `flow.wire`
- `bundle.create`
- `dev.doctor`
- `dev.run`

Unsupported keys fail early in `wizard::run` with PR-01-specific error text:

- `unsupported wizard target/mode \'<target>\'.\'<mode>\' for PR-01`

### Provider and Plan Construction

Implemented in `src/wizard/provider.rs` (`ShellWizardProvider`).

Current model:

- Always uses shell provider (`ShellWizardProvider`) for supported keys.
- Plan is deterministic and always has:
  - `plan_version: 1`
  - `created_at: None`
  - metadata (`target`, `mode`, `locale`, `frontend`)
- For each key, plan includes:
  - one high-level semantic step (or two for `bundle.create`), then
  - one terminal `RunCommand` step.

Answers usage:

- `wizard run` currently loads answers only from `--answers <json>` file.
- Internal merge function supports precedence layers, but run path currently passes only the answers file layer.
- If answers object is non-empty, `inputs.answers_ref = "answers.json"` is set.
- If `answers.provider_refs` exists, string entries are copied to sorted `inputs.provider_refs.<k>` keys.

Command synthesis details:

- `pack.build` does not include help fallback; with empty answers it runs `greentic-pack build`.
- Most other mappings use help fallback if required args are absent:
  - e.g. `new --help`, `build --help`, `add-step --help`, or `--help` for `dev.run`.
- `dev.doctor` uses positional flow then optional `--json`; no flow means `doctor --help`.

### Plan Rendering / Frontends

Implemented in `src/wizard/mod.rs` (`render_plan`).

Supported frontends:

- `json` -> pretty JSON of `WizardPlan`
- `text` -> concise textual summary
- `adaptive-card` -> Adaptive Card JSON envelope with `{ "data": { "plan": ... } }`

### Persistence and Replay

Implemented in `src/wizard/persistence.rs` and `src/wizard/mod.rs`.

Run persistence:

- Default output dir: `.greentic/wizard/run-<unix-seconds>/`
- Persisted files:
  - `answers.json`
  - `plan.json`
  - `exec.log` (only if execute actually runs command steps)

Replay behavior:

- `wizard replay --answers <path>` requires sibling `plan.json`.
- Replay loads persisted `answers.json` + `plan.json`, re-persists to output dir, then renders the plan.
- Replay output dir defaults to answers parent unless overridden by `--out`.

### Execute-Time Confirmation and Safety

Confirmation (`src/wizard/confirm.rs`):

- `--yes` bypasses prompt.
- Interactive TTY: asks `Execute plan? [y/N]:`.
- Non-interactive without `--yes` or `--non-interactive`: hard fail.

Executor controls (`src/wizard/executor.rs`):

- Only executes `WizardStep::RunCommand` steps.
- Program allowlist unless `--unsafe-commands`:
  - `greentic-pack`
  - `greentic-component`
  - `greentic-flow`
  - `greentic-operator`
  - `greentic-runner-cli`
- Blocks arguments containing exact tokens in deny list:
  - `|`, `;`, `&&`, `||`, `sh`, `-c`, `rm`, `mv`, `dd`
- Destructive step gate:
  - if any run step has `destructive: true`, requires `--allow-destructive`.

Execution logging and metadata:

- Appends `RUN <program> <args...>` lines to `exec.log`.
- Probes `<program> --version` before execution; if successful:
  - validates pinned version in `plan.inputs["resolved_versions.<program>"]` when present.
  - records resolved versions in execution report.
- After execution, wizard annotates and re-persists plan with:
  - `inputs.resolved_versions.<program>`
  - `inputs.executed_commands` (string count)

### Data Model

`src/wizard/plan.rs`:

- `WizardPlan` has `plan_version`, optional `created_at`, `metadata`, `inputs`, `steps`.
- `WizardStep` enum includes many semantic step types plus `RunCommand`.
- `RunCommandStep` has `program`, `args`, optional `destructive`.

## Test Coverage Snapshot

Wizard tests exist in:

- `tests/wizard_cli.rs`
- unit tests in `src/wizard/*`
- snapshots:
  - `tests/snapshots/wizard_pack_build_plan.json`
  - `tests/snapshots/wizard_flow_create_plan.json`

Covered behaviors include:

- dry-run/execute flag exclusivity
- dry-run JSON shape and snapshots
- replay roundtrip dry-run
- non-interactive execute gating
- frontend outputs (text, adaptive-card)
- replay safety checks (allowlist, destructive gate)
- version-pin mismatch failure
- execution persistence (`exec.log`, resolved versions, executed count)

## Audit Findings Relevant to PR-03

1. PR labeling drift in user-facing and doc text
- Errors and docs still refer to PR-01 / PR-DEV-01.
- Any PR-03 change that relies on user-visible phase labels should normalize this.

2. `pack.build` fallback behavior differs from others
- Empty answers produce executable `greentic-pack build` (not `--help`).
- This is intentional in code but inconsistent with other mappings that switch to help.

3. Unsafe-argument filter is token-based and coarse
- Current deny list is exact-token matching only.
- It blocks `rm` even when used as a legitimate argument value.
- It does not inspect substrings or structured argument semantics.

4. Replay trust model is permissive
- Replay accepts persisted plan steps as-is and executes them if safety gates pass.
- No schema/plan_version strict enforcement beyond serde decode.

5. Determinism is mostly structural, but default run-id is time-based
- Plan content is deterministic for fixed inputs.
- Default output path uses current UNIX seconds, so filesystem location is nondeterministic unless `--out` is provided.

## Validation Status During This Audit

Attempted: `cargo test --test wizard_cli`

Result: could not complete due to unrelated dependency build failure in external crate graph (`greentic-mcp-exec` type mismatch caused by mixed `wasmtime` versions 41/42).

This does not change the static audit conclusions above, but runtime verification in this environment is currently blocked until dependency resolution is fixed.

## Recommended PR-03 Input

Use this baseline as the contract for surgical changes:

- preserve current safety gates unless intentionally changed
- explicitly decide whether to normalize help-fallback behavior across all mappings
- decide whether PR-03 should keep or remove PR-01 wording from errors/docs
- if replay hardening is in scope, define plan validation policy before implementation
