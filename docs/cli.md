# greentic-dev CLI Guide

`greentic-dev` is a passthrough wrapper over upstream CLIs, plus a launcher wizard.

## Flow (passthrough to greentic-flow)

- `flow ...` delegates directly to `greentic-flow` (including `--help`).

## Component (passthrough to greentic-component)

- `component ...` delegates directly to `greentic-component` (including `--help`).

## Pack (passthrough to greentic-pack; `pack run` uses greentic-runner-cli)

- `pack ...` delegates to `greentic-pack`.
- `pack run ...` delegates to `greentic-runner-cli`.

## GUI / Secrets / MCP

- `gui ...` delegates to `greentic-gui`.
- `secrets ...` wraps `greentic-secrets` convenience flows.
- `mcp doctor` is available when the optional feature is enabled.

## CBOR

- `cbor <file>.cbor` decodes a CBOR payload and prints pretty JSON.

## Coverage

- `greentic-dev coverage`
- `greentic-dev coverage --skip-run`

Behavior:

- ensures `cargo-llvm-cov` and `cargo-nextest` are installed when the run is not skipped
- ensures `llvm-tools-preview` is available through `rustup`
- creates `target/coverage` when missing
- fails with Codex-oriented instructions if `coverage-policy.json` is missing
- writes the report to `target/coverage/coverage.json`
- validates the report against the global floor, per-file defaults, exclusions, and overrides in `coverage-policy.json`
- exits non-zero when setup fails, the coverage run fails, or the policy is violated
- `--skip-run` reuses an existing `target/coverage/coverage.json` file and only evaluates policy compliance

## Install

- `greentic-dev install`
- `greentic-dev install --tenant <TENANT> --token <TOKEN-or-env:VAR>`
- `greentic-dev install --tenant <TENANT> --token <TOKEN-or-env:VAR> --bin-dir <DIR>`
- `greentic-dev install --tenant <TENANT> --token <TOKEN-or-env:VAR> --docs-dir <DIR>`
- `greentic-dev install --tenant <TENANT> --locale <BCP47>`
- `greentic-dev install tools`

Behavior:

- bare `install` always runs the OSS delegated tool installer first
- when `--tenant` is omitted, the command stops after the OSS install step
- when `--tenant` is present, the command prompts for a hidden token if `--token` is omitted in an interactive terminal
- when `--tenant` is present in a non-interactive context, `--token` is required
- when a tenant token is available, the command also installs tenant-authorized binaries and docs
- `--locale` selects translated manifest/doc values when available; exact locale is preferred, then language-only fallback (`nl-NL` -> `nl`)
- `install tools` remains the legacy OSS-only `cargo binstall` path

Commercial install contract:

- tenant manifests are pulled from `oci://ghcr.io/greentic-biz/customers-tools/<tenant>:latest`
- tenant manifests may include expanded tool/doc entries or GitHub-hosted manifest references
- tenant manifests may also use the simple OCI payload shape:
  - tools: `{ id, targets }`
  - docs: `{ url, file_name }`
- commercial binaries and docs must come from GitHub-hosted URLs
- supported target `os` values are `linux`, `macos`, and `windows`
- supported target `arch` values are `x86_64` and `aarch64`
- Linux/macOS archives are expected as `.tar.gz`; Windows archives are expected as `.zip`
- `.tgz` is also accepted for gzip-compressed tarballs

Schema contract:

- tenant manifests should include `$schema` pointing to `tenant-tools.schema.json`
- tool manifests should include `$schema` pointing to `tool.schema.json`
- doc manifests should include `$schema` pointing to `doc.schema.json`
- `schema_version` is currently `"1"`
- `greentic-dev` currently consumes these schema-decorated manifests but does not perform JSON Schema validation before install
- tool/doc manifests may include `i18n` maps keyed by locale such as `nl` or `nl-NL`

Doc manifest notes:

- docs use `source.type = "download"`
- docs include `download_file_name` as part of the manifest contract
- docs use `default_relative_path` for the installed path under the docs root
- `default_relative_path` must remain within the docs directory; path traversal is rejected
- localized doc entries may override `title`, `source.url`, `download_file_name`, and `default_relative_path`
- simple doc entries use `file_name`, which installs directly under the docs root

Default install locations:

- binaries: `$CARGO_HOME/bin` or `~/.cargo/bin`
- docs: `~/.greentic/install/docs`
- state: `~/.greentic/install/state.json`

## Wizard (Launcher-Only)

- `greentic-dev wizard`
- `greentic-dev wizard --dry-run`
- `greentic-dev wizard --answers <FILE>`
- `greentic-dev wizard --answers <FILE> --dry-run`
- `greentic-dev wizard validate --answers <FILE>`
- `greentic-dev wizard apply --answers <FILE>`

Behavior:

- `wizard` is interactive and prompts for launcher action:
  - pack path -> delegates to `greentic-pack wizard`
  - bundle path -> delegates to `greentic-bundle wizard`
- `wizard --answers <FILE>` loads a launcher `AnswerDocument` and executes it directly.
- `wizard --answers <FILE>` also accepts direct `greentic-bundle` / `greentic-pack` AnswerDocuments and wraps them into launcher delegation automatically.
- If the launcher answers include `answers.delegate_answer_document`, the delegated wizard is replayed via its own `wizard apply --answers <FILE>` path instead of opening an inner interactive menu.
- `--dry-run` builds/renders plan without delegated execution.
- `wizard --answers <FILE> --dry-run` builds plan from `AnswerDocument` without delegated execution.
- `validate` builds plan from `AnswerDocument` without delegated execution.
- `apply` builds and executes delegation from `AnswerDocument`.
- `--emit-answers <FILE>` during interactive execution is captured through a delegated answers file and then written back as a launcher AnswerDocument envelope.
- `--emit-answers <FILE>` during dry-run / validate writes the launcher AnswerDocument locally because no delegated wizard executes.
- `wizard run` and `wizard replay` are removed.

Launcher AnswerDocument identity is strict:

- `wizard_id`: `greentic-dev.wizard.launcher.main`
- `schema_id`: `greentic-dev.launcher.main`

Other non-launcher IDs are rejected by `validate` / `apply`.

## Tips

- Missing delegated tools are not auto-installed. Use `greentic-dev install tools` (or `--latest`).
- Environment overrides:
  - `GREENTIC_DEV_BIN_GREENTIC_FLOW`
  - `GREENTIC_DEV_BIN_GREENTIC_COMPONENT`
  - `GREENTIC_DEV_BIN_GREENTIC_PACK`
  - `GREENTIC_DEV_BIN_GREENTIC_RUNNER_CLI`
  - `GREENTIC_DEV_BIN_GREENTIC_GUI`
  - `GREENTIC_DEV_BIN_GREENTIC_SECRETS`
