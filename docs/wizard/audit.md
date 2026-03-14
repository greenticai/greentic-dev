# Wizard Audit Notes

Status: updated for launcher-only implementation.

## Current Architecture

- `greentic-dev wizard` is a launcher flow, not a per-target orchestrator.
- `greentic-dev` delegates implementation work to downstream wizards.
- Deterministic plan-first behavior and persistence remain in `greentic-dev`.

## Current Delegation

- Pack path -> `greentic-pack wizard`
- Bundle path -> `greentic-bundle wizard`

## Current Answer Contract

- Only launcher AnswerDocument IDs are accepted:
  - `wizard_id = greentic-dev.wizard.launcher.main`
  - `schema_id = greentic-dev.launcher.main`

## Safety Model

- Execute path uses command allowlist enforcement.
- Unsafe shell-like args are blocked.
- Destructive command steps require explicit `--allow-destructive`.
