# greentic-dev: Stage 1 Audit (PR-02)

Date: 2026-03-02
Scope: wizard schema/plan/execute/migrate + delegation
Repo note: current implementation is a deterministic plan-first wizard that shells out to delegated Greentic binaries.

## Exact CLI Help Output

From `target/debug/greentic-dev`:

```text
$ target/debug/greentic-dev wizard --help
Deterministic orchestration for dev workbench workflows

Usage: greentic-dev wizard <COMMAND>

Commands:
  run     Build a deterministic wizard plan and optionally execute it
  replay  Replay a previously persisted wizard plan + answers
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

```text
$ target/debug/greentic-dev wizard run --help
Build a deterministic wizard plan and optionally execute it

Usage: greentic-dev wizard run [OPTIONS] --target <TARGET> --mode <MODE>

Options:
      --target <TARGET>      Target domain
      --mode <MODE>          Operation mode for the target
      --frontend <FRONTEND>  Frontend mode (text/json/adaptive-card) [default: json]
      --locale <LOCALE>      Locale (BCP47), passed to providers and recorded in plan metadata
      --answers <ANSWERS>    Answers file (JSON object)
      --out <OUT>            Override output directory (default: `.greentic/wizard/<run-id>/`)
      --dry-run              Preview only (default when neither --dry-run nor --execute is set)
      --execute              Execute plan steps
      --yes                  Skip interactive confirmation prompt
      --non-interactive      Allow execution in non-interactive contexts
      --unsafe-commands      Allow commands outside the default run-command allowlist
      --allow-destructive    Allow destructive operations (delete/overwrite/move) when requested by a plan step
  -h, --help                 Print help
```

```text
$ target/debug/greentic-dev wizard replay --help
Replay a previously persisted wizard plan + answers

Usage: greentic-dev wizard replay [OPTIONS] --answers <ANSWERS>

Options:
      --answers <ANSWERS>  Path to a persisted answers file from a prior run
      --execute            Execute plan steps
      --dry-run            Preview only (default when neither --dry-run nor --execute is set)
      --yes                Skip interactive confirmation prompt
      --non-interactive    Allow execution in non-interactive contexts
      --unsafe-commands    Allow commands outside the default run-command allowlist
      --allow-destructive  Allow destructive operations (delete/overwrite/move) when requested by a plan step
      --out <OUT>          Override output directory (default: reuse answers parent)
  -h, --help               Print help
```

## A. CLI Surface

| Item | Current behavior | Files/lines |
|---|---|---|
| Wizard command path(s) | Top-level `wizard` subcommand with `run` and `replay`; dispatch in `main` directly to wizard module. | `src/cli.rs:117-123`, `src/main.rs:59-62` |
| Flags for locale | `wizard run` supports `--locale`; replay has no locale override. | `src/cli.rs:136-138`, `src/cli.rs:166-190` |
| Flags for answers import/export | `--answers` exists for `run` and `replay` import. No `--emit-answers` flag exists. | `src/cli.rs:139-141`, `src/cli.rs:168-169` |
| Validate/apply split | No explicit `validate`/`apply` subcommands; behavior is mode-based (`dry-run` default, `--execute` for apply). | `src/cli.rs:145-150`, `src/wizard/mod.rs:205-213`, `src/wizard/mod.rs:61-82`, `src/wizard/mod.rs:95-116` |
| Non-zero exit handling | Execution failures bubble as `anyhow` errors; delegated passthrough commands exit with child status code. | `src/wizard/executor.rs:62-69`, `src/main.rs:25-47`, `src/main.rs:63-67`, `src/passthrough.rs:37-44` |

Default mode and interactivity:

- Defaults to dry-run if neither `--dry-run` nor `--execute` is set (`src/wizard/mod.rs:205-213`).
- Execute in non-interactive context requires `--yes` or `--non-interactive` (`src/wizard/confirm.rs:10-17`).

## B. Schema + Questions

| Item | Current approach | Files/lines |
|---|---|---|
| Schema identity | No `wizard_id`/`schema_id` fields exist. Plan metadata carries `target`, `mode`, `locale`, `frontend`. | `src/wizard/plan.rs:35-41` |
| Schema versioning | No schema version concept for answers; only plan format version (`plan_version: 1`). | `src/wizard/plan.rs:25-33`, `src/wizard/provider.rs:41-43` |
| Question model | No interactive question schema in this repo. Inputs are flat JSON key/value answers, mapped into command args by static per-target mapping. | `src/wizard/mod.rs:43-45`, `src/wizard/provider.rs:73-193`, `src/wizard/provider.rs:195-240` |
| Validation rules | Validation is implicit/coarse: JSON parse for answers, supported target/mode registry check, frontend parse, allowlist/unsafe-arg checks before execute. | `src/wizard/mod.rs:28-41`, `src/wizard/mod.rs:169-177`, `src/wizard/registry.rs:12-58`, `src/wizard/executor.rs:34-45` |
| Defaults | Default locale `en-US`; default frontend `json`; default execution mode `dry-run`; default out dir `.greentic/wizard/run-<unix-seconds>`. | `src/wizard/mod.rs:27`, `src/cli.rs:134-135`, `src/wizard/mod.rs:205-213`, `src/wizard/persistence.rs:15-24` |
| i18n keys | No per-question i18n key system. Locale is stored/passed through metadata only. | `src/wizard/provider.rs:44-49`, `src/wizard/plan.rs:39` |

## C. Plan / Execute / Migrate

| Item | Current approach | Files/lines |
|---|---|---|
| Plan representation | `WizardPlan` with metadata, optional string inputs map, and ordered semantic + `RunCommand` steps. | `src/wizard/plan.rs:25-69` |
| Apply/execution | `wizard run/replay --execute` confirms then executes only `RunCommand` steps; logs `exec.log`; captures versions and executed count back into `plan.json`. | `src/wizard/mod.rs:61-82`, `src/wizard/mod.rs:95-116`, `src/wizard/executor.rs:20-77`, `src/wizard/mod.rs:216-227` |
| Validation-only path | Dry-run renders plan and writes `answers.json`/`plan.json` without command execution. | `src/wizard/mod.rs:55-60`, `src/wizard/mod.rs:87-94`, `src/wizard/persistence.rs:35-48` |
| Migration | No migration mechanism or `--migrate` flag exists for answers or plans. | `src/cli.rs:117-190`, `src/wizard/persistence.rs:51-78` |
| Locks/reproducibility | Replay can pin tool versions via `inputs.resolved_versions.<program>`; validated against `<program> --version` at execute time. No broader lock document schema. | `src/wizard/executor.rs:47-52`, `src/wizard/executor.rs:85-117`, `src/wizard/mod.rs:220-223` |

## D. Delegation Model

- Wizard itself delegates by emitting `RunCommand` steps for these binaries:
  - `greentic-pack`, `greentic-component`, `greentic-flow`, `greentic-operator`, `greentic-runner-cli`
  - references: `src/wizard/provider.rs:75-189`, allowlist in `src/wizard/executor.rs:119-127`
- Broader repo delegation pattern (non-wizard commands) is passthrough using resolved binaries and inherited stdio:
  - `src/main.rs:25-47`, `src/main.rs:63-67`, `src/passthrough.rs:8-44`

## E. Tests

| Test | What it covers | Command |
|---|---|---|
| `tests/wizard_cli.rs` | CLI surface behavior, snapshot plans, replay behavior, non-interactive execute gating, allowlist/destructive checks, version pin enforcement, execution metadata persistence | `cargo test --test wizard_cli` |
| `src/wizard/mod.rs` unit tests | execution mode precedence and answer merge precedence | `cargo test wizard::mod::tests` |
| `src/wizard/provider.rs` unit tests | command argument mapping and provider refs extraction | `cargo test wizard::provider::tests` |
| `src/wizard/registry.rs` unit tests | supported/unsupported target-mode resolution | `cargo test wizard::registry::tests` |
| `src/wizard/executor.rs` unit tests | allowlist, unsafe args, version pin checks | `cargo test wizard::executor::tests` |
| Snapshots | deterministic JSON outputs for `pack.build` and `flow.create` | `tests/snapshots/wizard_pack_build_plan.json`, `tests/snapshots/wizard_flow_create_plan.json` |

## Validation Run Status

Attempted:

```text
cargo test --test wizard_cli
```

Current result in this environment: blocked by unrelated dependency graph compile error in `greentic-mcp-exec` (mixed `wasmtime` 41/42 types), before wizard tests can execute.

## Constraints and Compatibility Notes for Stage 2

- Existing user-facing command shape is `wizard run` / `wizard replay`; changes should preserve compatibility aliases.
- `--answers` currently expects a plain JSON object (not an envelope); switching to AnswerDocument must consider back-compat parser behavior.
- Replay currently trusts persisted `plan.json` structure if it deserializes; tightening this is potentially behavior-breaking and should be explicit.
- Error text includes `PR-01` wording (`src/wizard/mod.rs:35-40`), which may need normalization if phase labels matter.
