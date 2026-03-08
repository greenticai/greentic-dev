# PR: Launcher-Only `greentic-dev wizard` (Stage 2)

Date: 2026-03-04

## Summary

This PR reduces `greentic-dev` wizard behavior to a single launcher flow and removes legacy wizard command paths.

- Primary entrypoint: `greentic-dev wizard`
- Non-interactive subcommands:
  - `greentic-dev wizard validate --answers <FILE>`
  - `greentic-dev wizard apply --answers <FILE>`
- Removed: `wizard run` and `wizard replay`
- Answer input is now strict: only launcher `AnswerDocument` IDs are accepted.

## Goals

- Make wizard behavior launcher-only (`launcher.main`)
- Remove legacy target/mode command surface from `greentic-dev` wizard
- Remove replay path
- Keep deterministic validation/apply flows through AnswerDocument
- Delegate actual work to downstream wizards (`greentic-pack`, `greentic-operator`)

## CLI Behavior

### 1) Interactive Launcher

Command:

```bash
greentic-dev wizard
```

Behavior:

- Prompts:
  - `1) Build / Update a Pack (flows + components)`
  - `2) Build / Update a Production Bundle`
- Builds a launcher plan (`target=launcher`, `mode=main`)
- Executes delegation by default (apply mode)

Notes:

- Requires an interactive terminal.
- In non-interactive contexts, users should use `validate/apply --answers`.

### 2) Interactive Dry-Run

Command:

```bash
greentic-dev wizard --dry-run
```

Behavior:

- Same menu prompt as apply mode
- No delegated command execution
- Renders/persists deterministic plan and answers

Optional emit:

```bash
greentic-dev wizard --dry-run --emit-answers answers.json
```

### 3) Validate From AnswerDocument

Command:

```bash
greentic-dev wizard validate --answers answers.json
```

Behavior:

- Loads launcher AnswerDocument
- Validates schema identity and builds plan
- Does not execute delegated command

### 4) Apply From AnswerDocument

Command:

```bash
greentic-dev wizard apply --answers answers.json
```

Behavior:

- Loads launcher AnswerDocument
- Builds plan and executes delegated command

## Delegation

Launcher selections map to:

- `selected_action = "pack"` -> `greentic-pack wizard`
- `selected_action = "bundle"` -> `greentic-operator wizard`

Dry-run plan generation appends `--dry-run` to delegated args in the generated run command.

## AnswerDocument Contract

Accepted envelope (strict identity):

```json
{
  "wizard_id": "greentic-dev.wizard.launcher.main",
  "schema_id": "greentic-dev.launcher.main",
  "schema_version": "1.0.0",
  "locale": "en-US",
  "answers": {
    "selected_action": "pack"
  },
  "locks": {}
}
```

Identity rules:

- `wizard_id` must equal `greentic-dev.wizard.launcher.main`
- `schema_id` must equal `greentic-dev.launcher.main`
- Non-launcher IDs are rejected

## Removed Legacy Wizard Paths

- `wizard run`
- `wizard replay`
- Legacy `target/mode` mappings for pack/flow/component/bundle/dev wizard orchestration inside `greentic-dev`

## Files Updated

- `src/cli.rs`
- `src/main.rs`
- `src/wizard/mod.rs`
- `src/wizard/provider.rs`
- `src/wizard/registry.rs`
- `src/wizard/plan.rs`
- `src/wizard/persistence.rs`
- `tests/wizard_cli.rs`

## Tests Updated

`tests/wizard_cli.rs` now verifies:

- launcher command requires interactive terminal
- replay command is removed
- validate builds launcher dry-run plan
- apply executes delegated wizard command
- non-launcher AnswerDocument IDs are rejected
- emitted answers use launcher IDs

## Validation

```bash
cargo test --test wizard_cli
```

Result: 6 passed, 0 failed.
