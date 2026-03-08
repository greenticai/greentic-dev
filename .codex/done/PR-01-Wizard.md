PR-DEV-01
Add greentic-dev wizard Orchestrator + Dev Workbench Integration

Repo: greentic-dev
Theme: Deterministic orchestration, delegation, multi-frontend UX, replay, and developer workbench automation.

🎯 Purpose

Create a single unified developer entrypoint:

greentic-dev wizard

capable of orchestrating:

Operator bundle creation

Gtpack creation & composition

Component scaffolding & building

Flow creation & wiring (local + remote components)

Doctor validation

Dev run execution

Multi-step chained workflows

Deterministic plan-first execution with replay

greentic-dev becomes the developer workbench orchestrator, not the owner of pack/component/flow logic.

🧠 Design Philosophy

greentic-dev wizard orchestrates.

Other repos own:

Specs

Apply logic

Bundle/materialization logic

Execution must be:

Deterministic

Plan-first

Replayable

Safe by default (dry-run)

No duplication of QA types or bundle schemas.

🚫 Non-Goals

No new pack or bundle layout definitions.

No provider-specific setup logic in greentic-dev.

No rewriting lower-level repos.

No breaking CLI UX unless explicitly documented.

🏗 High-Level Architecture
greentic-dev wizard
        |
        v
  Wizard Registry
        |
        v
  Target Providers
(operator | pack | component | flow | bundle | dev)
        |
        v
  QaSpec → Answers → Apply → WizardPlan
        |
        v
   Plan Executor (dry-run or execute)
📋 Capabilities
Phase 1 — Orchestrator Core

Target discovery

Delegation

Frontend selection

Deterministic plan composition

Replay support

Answer prefill & merging

i18n propagation

Phase 2 — Dev Workbench Targets

Supports orchestrating workflows for:

1️⃣ Gtpack creation & management

create gtpack

add app packs

add provider packs

build pack

Delegates to greentic-pack wizard provider.

2️⃣ Component scaffolding & building

scaffold component

add features

build component

Delegates to greentic-component wizard provider.

3️⃣ Flow creation & wiring

create flow

add nodes

attach local components

attach remote components

wire nodes

Delegates to greentic-flow wizard provider.

4️⃣ Bundle composition (via operator)

create bundle

add packs

run pack setup

manage tenancy

manage allow rules (via gmap pipeline)

Delegates to greentic-operator wizard provider.

5️⃣ Doctor & Dev Run orchestration

Supports plan steps such as:

RunDoctor

DevRun

ValidateBundle

These are high-level plan steps that wrap existing commands.

🧩 CLI Interface
greentic-dev wizard run \
  --target <operator|pack|component|flow|bundle|dev> \
  --mode <create|update|build|wire|...> \
  --frontend <text|json|adaptive-card> \
  --locale <bcp47> \
  --answers <file> \
  --dry-run \
  --execute \
  --out <dir>

Also:

greentic-dev wizard replay --answers <file> --execute
🔄 Deterministic Plan Model
Rules

apply() must never write files when dry_run=true.

All execution happens after confirmation or --execute.

Plan JSON must be stable and snapshot-testable.

Plan metadata must include:

locale

target

mode

version info if available

🧱 WizardPlan Step Types

High-level steps only:

ResolvePacks

CreateBundle

AddPacksToBundle

ApplyPackSetup

CreateGtpack

ScaffoldComponent

BuildComponent

CreateFlow

WireFlow

ApplyGmapRules

RunResolver

ValidateBundle

RunDoctor

DevRun

RunCommand (fallback)

Do not define low-level file ops unless underlying APIs require them.

🧭 Wizard Registry

src/wizard/registry.rs

Maps:

operator.create
pack.create
pack.build
component.scaffold
component.build
flow.create
flow.wire
bundle.create
dev.doctor
dev.run

Providers can be:

Direct crate integration (workspace)

Shell-out bridge (initial phase)

Future plugin integration

🖥 Frontend Support

Text interactive

JSON non-interactive

Adaptive Card (initially JSON output; future messaging integration)

Reuse greentic-qa runner APIs if available.
Otherwise wrap CLI calls behind trait abstraction.

🌍 i18n

--locale passed to providers.

Stored in plan metadata.

QA runner resolves localized strings.

Fallback order:

Requested locale

Default locale

Raw text

💾 Persistence

Store under:

.greentic/wizard/<timestamp>/
  answers.json
  plan.json

Replay supported via:

greentic-dev wizard replay --answers <file> --execute
🧪 Tests
Unit

Answer precedence merge

Deterministic plan ordering

Registry resolution

Integration

Mock wizard provider

--dry-run stable JSON output

Replay roundtrip

Dev-workbench chained workflow smoke (mock providers)

📚 Docs

docs/wizard/README.md

docs/wizard/architecture.md

docs/wizard/dev-workbench.md

🔐 Safety

Dry-run default

Execution requires confirmation or --execute

No destructive filesystem ops outside declared plan steps

No breaking existing CLI workflows

🧾 Definition of Done

greentic-dev wizard compiles and runs

Supports multiple targets

Plan-first model enforced

Deterministic JSON output

Replay works

Dev workbench orchestration supported via delegation

Tests pass

## Resolved Questions

1) --dry-run and --execute both present

Answer: Mutually exclusive.

Rule:

Default is --dry-run when neither is specified.

If both are provided, return a clear CLI error: “Choose one of --dry-run or --execute.”

Rationale: Keeps intent unambiguous and avoids “oops executed” surprises.

2) Confirmation UX before execution

Answer:

If --execute is used in an interactive terminal, show plan summary and prompt: Execute plan? [y/N].

In non-interactive contexts (no TTY / CI), require --execute --yes (or --execute --non-interactive) to proceed. Otherwise fail with an actionable message.

Default: Never execute without explicit user consent.

Codex instruction: reuse any existing confirmation helper in greentic-dev; if none, implement a tiny one that checks TTY.

3) Answer merge precedence order

Answer (exact precedence, highest → lowest):

CLI explicit answers overrides (e.g., inline --set key=value if you add it later; for now this is mostly --answers plus mode/target/locale flags)

Parent prefill (answers passed down by delegator when composing specs)

Answers file (--answers <file>)

Provider defaults (defaults in QaSpec schema)

Empty / unset

Note: --answers file is an input; parent prefill should override it because delegation is intentional.

4) Plan schema/versioning

Answer: Yes, include an explicit version field.

Minimum fields:

plan_version: 1

created_at (optional, but do not use in snapshot tests unless fixed)

target, mode, locale

steps: [...]

inputs: { answers_ref?, provider_refs? } (optional)

Canonical schema reuse: If any existing plan schema exists in greentic-types, reuse it. If not, define a small local schema now with plan_version so it can converge later.

5) Registry keys: exhaustive or add more now?

Answer: For PR-01, include only the keys needed to prove the orchestration model plus the dev-workbench basics you listed:

operator.create

pack.create, pack.build

component.scaffold, component.build

flow.create, flow.wire

bundle.create

dev.doctor, dev.run

Do not add update variants now unless the underlying provider already supports them; otherwise it will bloat scope. Add as follow-up PR.

6) Provider integration mode for Phase 1

Answer: Prefer shell-out bridge first unless a provider is already available as a library crate in the workspace with a stable API.

Rule:

If direct integration is trivial and already idiomatic → do it.

Otherwise use shell-out behind a trait so you can swap to in-process later.

Reason: Keeps greentic-dev decoupled and avoids cross-repo dependency churn.

7) RunCommand safety policy

Answer: Implement a simple allowlist policy for PR-01.

Allowed by default (examples):

greentic-pack ...

greentic-component ...

greentic-flow ...

greentic-operator ...

cargo ... only if scoped to build/test actions you already use in dev flows (optional; safer to exclude in PR-01)

Blocked by default:

arbitrary commands

anything with rm, mv, dd, shell pipelines, or sh -c

Enforcement:

RunCommand step includes program and args (no shell string).

Executor rejects commands not in allowlist unless --unsafe-commands is explicitly provided.

Audit trail:

Always log each executed command step to .greentic/wizard/<id>/exec.log.

8) Replay semantics: re-resolve latest state or pin?

Answer: PR-01 replay should be deterministic with respect to recorded inputs, but may re-resolve external refs unless a digest was recorded.

Rule:

If plan step includes resolved digests/versions, replay must use them.

If plan only contains floating refs (e.g., latest), replay will re-resolve current. Document this clearly.

Practical approach:

During --dry-run, do not fetch network state.

During --execute, if resolver returns digest/version, store it into metadata for future replay.

9) Persistence location and --out

Answer: --out overrides the entire .greentic/wizard/<timestamp>/ directory.

If --out is not provided, default to .greentic/wizard/<run-id>/.

Always create:

answers.json

plan.json

exec.log (if executed)

No nested “double dirs” unless the repo already does it.

10) Adaptive Card support in PR-01

Answer: Yes, JSON output only is sufficient for PR-01.

The adaptive-card “frontend” can:

print the card JSON to stdout

accept a submitted JSON payload via --answers for replay/roundtrip

Transport integration (Teams/WebChat/etc.) is follow-up.

11) i18n source of truth if provider lacks locale assets

Answer: Fallback chain:

provider locale strings (if present)

provider default locale (if defined)

raw string fields in spec (existing title/description)

key-as-text (only if provider uses keys without raw fallback; add a warning)

Orchestrator responsibility: pass --locale through; do not invent translations.

12) “No destructive fs ops” definition and enforcement

Answer: For PR-01, define “destructive” as:

deleting files/dirs

overwriting existing files outside a designated working directory

moving/renaming user files

Enforcement level:

Wizard executor only writes within:

the chosen --out directory, and/or

a user-specified working directory explicitly confirmed in the plan

If a plan step requests deletion or overwrite, require an explicit --allow-destructive flag.

13) Existing CLI compatibility: reserved names/flags

Answer: Preserve existing greentic-dev subcommands and flags.

Ensure wizard is a new top-level subcommand that doesn’t collide.

Follow the existing clap style already used in greentic-dev.

Codex should audit src/ CLI structure and confirm no conflicts.

14) Test snapshot baseline / framework

Answer: Use whatever the repo already uses:

if it uses insta, use insta for JSON snapshots

otherwise implement a minimal golden-file snapshot under tests/snapshots/ with pretty_assertions diff

Rule: Snapshot must exclude nondeterministic fields (timestamps, temp paths).

15) Audit-first artifact: doc path/name/format

Answer: Required path:

docs/wizard/audit.md

Format:

short bullets:

existing CLI patterns

existing QA integration points (if any)

whether shell-out is already used elsewhere

recommended integration mode for phase 1

Must be written before final implementation is completed (but can be committed in the same PR).

## Implementation Adjustments

- Make `--dry-run` / `--execute` mutually exclusive with default dry-run.
- Add confirmation prompt and `--yes` gating for non-interactive execute.
- Define answer precedence rules explicitly in code + docs.
- Add `plan_version: 1` and deterministic snapshot rules.
- Use shell-out providers behind a trait by default.
- Add `RunCommand` allowlist policy and `exec.log`.
- Define replay pinning behavior based on recorded digests.
- Define persistence semantics for `--out`.
- Keep adaptive-card support JSON-only in PR-01.
- Define i18n fallback chain and destructive-op enforcement.
- Require `docs/wizard/audit.md` and follow repo snapshot strategy.

Codex Prompt (Final Version)

You are implementing PR-DEV-01 with Dev Workbench extensions.

Pre-authorized:

Create/update files

Add tests/docs

Add CLI subcommands

Refactor safely (additive)

Add CI updates if required

Rules:

Do not duplicate QA types.

Do not implement pack/component/flow logic here — delegate.

Keep plan-first deterministic model.

Dry-run must be side-effect free.

Preserve existing CLI UX patterns.

Use high-level plan steps.

Add registry entries for workbench targets.

Tests must include deterministic snapshot test.

Replay must work.

Implement audit first and record findings in docs.

Proceed without repeatedly asking for confirmation unless blocked by missing APIs.
