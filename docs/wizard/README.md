# greentic-dev wizard

`greentic-dev wizard` is a launcher-only deterministic orchestration entrypoint.

## Commands

- `greentic-dev wizard`
- `greentic-dev wizard --dry-run`
- `greentic-dev wizard validate --answers <path>`
- `greentic-dev wizard apply --answers <path>`

Removed:

- `wizard run`
- `wizard replay`

## Launcher Contract

Plan identity is fixed to:

- `target`: `launcher`
- `mode`: `main`

Selection mapping:

- `selected_action = "pack"` -> delegated command `greentic-pack wizard`
- `selected_action = "bundle"` -> delegated command `greentic-bundle wizard`
- when `answers.delegate_answer_document` is present, delegated execution uses `wizard apply --answers <persisted-file>` instead of the interactive delegated menu

For dry-run plans, delegated args include `--dry-run`.

## AnswerDocument

`validate` and `apply` natively accept launcher documents.

Required identity fields:

- `wizard_id = "greentic-dev.wizard.launcher.main"`
- `schema_id = "greentic-dev.launcher.main"`

Example:

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

Bundle and pack AnswerDocuments are also accepted at the top level of `greentic-dev wizard --answers <FILE>`.
They are wrapped into launcher answers automatically as:

- bundle document -> `selected_action = "bundle"`
- pack document -> `selected_action = "pack"`

and delegated through `wizard apply --answers <persisted-file>`.

## Frontends

- `text`
- `json`
- `adaptive-card`

## Execution Rules

- `wizard` defaults to apply mode (unless `--dry-run`).
- `validate` always dry-run.
- `apply` executes delegation.
- `--emit-answers <path>` is delegated during execute flows so the downstream pack/bundle wizard writes the emitted AnswerDocument.
- `--emit-answers <path>` on dry-run / validate emits the launcher AnswerDocument directly from `greentic-dev`.
- Execute confirmation rules:
  - Interactive TTY prompts unless `--yes`.
  - Non-interactive requires `--yes` or `--non-interactive`.

## Persistence

- Default output: `.greentic/wizard/<run-id>/`
- `--out` overrides output directory.
- Persisted files:
  - `answers.json`
  - `plan.json`
  - `exec.log` (only when executed)

## Safety

- Only `RunCommand` steps are executed.
- Default allowlist:
  - `greentic-pack`
  - `greentic-component`
  - `greentic-flow`
- `greentic-bundle`
  - `greentic-runner-cli`
- Non-allowlist programs require `--unsafe-commands`.
- Destructive steps require `--allow-destructive`.
