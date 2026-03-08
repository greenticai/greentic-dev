# greentic-dev: Stage 1 — Audit (Wizard schema/plan/execute/migrate + delegation)

    **Purpose:** Produce a factual snapshot of the current wizard implementation so Stage 2 can be surgical.
    **Output artifacts:** `AUDIT.md` + filled tables below + exact file/line references.

    ## Scope
    - Identify existing wizard entrypoints (CLI, subcommands, modules)
    - Identify current schema representation (if any) and how questions are defined
    - Identify execution model: plan/apply/side effects; validate vs apply
    - Identify i18n handling and locale resolution
    - Identify any existing non-interactive answer support (flags/files/env)
    - Identify how this repo delegates to other binaries (if applicable)
    - Identify tests/snapshots covering wizard paths

    ## Repo description
    Passthrough wizard delegating into greentic-pack wizard

    ## Audit checklist

    ### A. CLI surface
    - [ ] List `wizard` subcommands and flags (copy exact help output)
    - [ ] Note any existing `--locale`, `--answers`, `--emit-answers` or equivalents
    - [ ] Note default mode (interactive vs non-interactive) and how it decides

    **Findings table (fill in):**
    | Item | Current behavior | Files/lines |
    |---|---|---|
    | Wizard command path(s) |  |  |
    | Flags for locale |  |  |
    | Flags for answers import/export |  |  |
    | Validate/apply split |  |  |
    | Non-zero exit handling |  |  |

    ### B. Schema + questions
    - [ ] Where are questions defined (data-driven JSON, Rust structs, macros, etc.)?
    - [ ] Is there a schema_id/schema_version today? If yes, where?
    - [ ] Is there an i18n key system already (en.json tags)?

    **Findings table (fill in):**
    | Item | Current approach | Files/lines |
    |---|---|---|
    | Schema identity |  |  |
    | Schema versioning |  |  |
    | Question model |  |  |
    | Validation rules |  |  |
    | Defaults |  |  |
    | i18n keys |  |  |

    ### C. Plan/execute/migrate
    - [ ] How does wizard output become actions? (plan object? direct side-effects?)
    - [ ] Where are execution side effects performed?
    - [ ] Is there any migration capability today?

    **Findings table (fill in):**
    | Item | Current approach | Files/lines |
    |---|---|---|
    | Plan representation |  |  |
    | Apply/execution |  |  |
    | Validation-only path |  |  |
    | Migration |  |  |
    | Locks/reproducibility |  |  |

    ### D. Tests
    - [ ] Identify unit/snapshot/e2e tests that touch wizard paths
    - [ ] Capture how to run them locally

    **Findings table (fill in):**
    | Test | What it covers | Command |
    |---|---|---|
    |  |  |  |

    ### Ripgrep audit patterns
Run these from repo root (adjust paths if needed):

```bash
rg -n "wizard|qa|question|prompt|interactive|inquirer|dialoguer|clap.*wizard|subcommand.*wizard" .
rg -n "schema(_id|_version)?|AnswerDocument|answers\.json|emit-answers|--answers|--locale|i18n" .
rg -n "apply_answers|setup|default|install|provision|plan|execute|validate|migrate" .
```

    ## Deliverables
    1) `AUDIT.md` with:
       - Exact CLI help output (copy/paste)
       - File/line references for all key paths
       - Completed tables above
    2) Notes on any constraints or compatibility requirements

    ## Stop condition
    **Do not implement changes in Stage 1.** Only gather facts needed for Stage 2.

---

## Stage 1 completion (2026-03-02)

Status: completed.

Primary artifact:

- `AUDIT.md` (repo root) contains:
  - exact CLI help output
  - completed A/B/C/D findings tables
  - file/line references for key behavior
  - test inventory and run command
  - constraints/compat notes for Stage 2

Notes:

- No implementation changes were made as part of Stage 1 audit.
