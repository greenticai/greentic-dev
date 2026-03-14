# Dev Workbench Launcher (Current Scope)

`greentic-dev` no longer owns per-target wizard mappings. It now provides a launcher flow that delegates to downstream wizards.

## Launcher Paths

- `pack` path delegates to `greentic-pack wizard`
- `bundle` path delegates to `greentic-bundle wizard`

## Interactive Usage

Apply mode (default):

```bash
greentic-dev wizard
```

Dry-run mode:

```bash
greentic-dev wizard --dry-run
```

## Deterministic Usage With Answers

Validate only:

```bash
greentic-dev wizard --answers answers.json --dry-run
```

Apply delegation:

```bash
greentic-dev wizard --answers answers.json
```

Equivalent explicit subcommands:

```bash
greentic-dev wizard validate --answers answers.json
greentic-dev wizard apply --answers answers.json
```

## AnswerDocument Example

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

## Notes

- `wizard run` and `wizard replay` are removed.
- Legacy `target.mode` orchestration is removed from `greentic-dev` wizard.
- Lower-level repos keep ownership of component/flow/pack/bundle apply logic.
