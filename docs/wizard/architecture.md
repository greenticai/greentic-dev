# Wizard Architecture (Launcher-Only)

## Overview

`greentic-dev wizard` is a launcher-first flow:

1. Collect launcher selection (`pack` or `bundle`) via interactive prompt, or load AnswerDocument.
2. Build deterministic launcher plan (`launcher.main`).
3. Persist `answers.json` + `plan.json`.
4. Dry-run: render plan only.
5. Apply: confirm and execute delegated command; append `exec.log`.

## Modules

- `src/wizard/registry.rs`: launcher registration (`launcher.main`)
- `src/wizard/provider.rs`: provider trait + shell launcher provider
- `src/wizard/plan.rs`: `plan_version: 1` model and step types
- `src/wizard/persistence.rs`: output dir and plan/answers persistence
- `src/wizard/confirm.rs`: interactive/non-interactive execute confirmation
- `src/wizard/executor.rs`: allowlist enforcement + command execution logging

## Delegation

- `selected_action = pack` -> `greentic-pack wizard`
- `selected_action = bundle` -> `greentic-bundle wizard`

## AnswerDocument Rules

Only launcher identity is accepted:

- `wizard_id = greentic-dev.wizard.launcher.main`
- `schema_id = greentic-dev.launcher.main`

Documents with non-launcher IDs are rejected.

## Determinism

- Plan has explicit `plan_version: 1`.
- Step ordering is deterministic by construction.
- Default output dir includes time-based run id; use `--out` for fixed paths.

## Destructive Control

- `RunCommand` steps may mark `destructive: true`.
- Executor rejects destructive plans unless `--allow-destructive` is set.
