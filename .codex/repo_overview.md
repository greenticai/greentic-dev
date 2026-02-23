# Repository Overview

## 1. High-Level Purpose
- Workspace for the `greentic-dev` CLI and related tooling used to design, validate, and run Greentic flows and packs. The CLI validates flow YAML against component schemas, builds deterministic `.gtpack` bundles, runs them with mocks/telemetry, verifies signatures, and delegates to companion tools (`greentic-component`, `greentic-flow`, `greentic-pack`, `greentic-runner-cli`, `greentic-gui`, `greentic-secrets`) via PATH.
- Contains a small `dev-viewer` utility to render flow transcripts with schema context. Written in Rust with multiple internal crates and relies on Greentic-specific libraries (`greentic_flow`, `greentic_pack`, `greentic_runner`, `greentic_component`).

## 2. Main Components and Functionality
- **Path:** `src/main.rs`, `src/cli.rs`  
  **Role:** Entrypoint and argument parsing for the `greentic-dev` CLI.  
  **Key functionality:** Defines subcommands for flow validation, pack build/run/verify, pack scaffolding passthrough, component passthrough, config editing, and optional MCP doctor. Bridges CLI enums to module implementations and parses policy/mocks/signing enums.

- **Path:** `src/pack_build.rs`  
  **Role:** Build deterministic `.gtpack` archives from flow YAML.  
  **Key functionality:** Loads and validates flow bundles; resolves components and schema validation via `ComponentResolver`; writes resolved configs under `.greentic/resolved_config`; loads optional pack metadata; builds packs with signing/provenance; optional deterministic re-build when `LOCAL_CHECK_STRICT` is set.  
  **Key dependencies / integration points:** Uses `greentic_pack` builder, `greentic_flow` bundle validation, `ComponentResolver` for component prep and schema validation, git metadata for provenance.

- **Path:** `src/component_resolver.rs`  
  **Role:** Resolve and validate component artifacts used in flows/packs.  
  **Key functionality:** Prepares components (optionally from local `--component-dir`); enforces semver requirements; caches prepared components and compiled JSON Schemas; validates node configs against component schemas; extracts configs from flow documents.  
  **Key dependencies / integration points:** Relies on `greentic_component` prepare/describe APIs, `jsonschema` for validation, used by pack build.

- **Path:** `src/pack_run.rs`  
  **Role:** Execute built packs locally with mocks/telemetry controls.  
  **Key functionality:** Parses input JSON, constructs OTLP hook, configures HTTP/MCP mocks and allowlists, runs packs via `greentic_runner::desktop::Runner`, writes artifacts if requested, and prints run results as pretty JSON.

- **Path:** `src/pack_verify.rs`  
  **Role:** Verify pack signatures and emit manifest/report.  
  **Key functionality:** Opens packs with selected signing policy, prints success and warnings or emits detailed JSON (manifest, report, SBOM) on request.

- **Path:** `src/dev_runner/*` (`registry.rs`, `runner.rs`, `schema.rs`, `transcript.rs`)  
  **Role:** Flow validation utilities reused across tools.  
  **Key functionality:** Maintains describe registry with defaults/schemas; validates flow documents against schemas via `FlowValidator`; extracts schema IDs and validates YAML against JSON Schema; reads/writes transcripts (`FlowTranscript`, `NodeTranscript`) for later viewing.

- **Path:** `src/config.rs`  
  **Role:** Load user config (`~/.greentic/config.toml`).  
  **Key functionality:** Deserializes tool paths and default component settings; helper to locate config path.

- **Path:** `src/cmd/*`, `src/component_cli.rs`, `src/delegate/*`, `src/util/*`, `src/passthrough.rs`  
  **Role:** Glue/passthrough and helper functions for CLI subcommands (pack scaffolding via `packc`, component passthrough, config set command, etc.).  
  **Key functionality:** Thin wrappers around external tools and internal helpers. Delegated tools are resolved from PATH (or explicit env override) and are installed explicitly via `greentic-dev tools install` / `greentic-dev install tools` (no auto-install fallback).

- **Path:** `crates/dev-viewer/`  
  **Role:** Standalone CLI to render flow transcripts.  
  **Key functionality:** Loads YAML transcripts into `FlowTranscript`, annotates resolved configs with default/override markers from run logs, and prints them with structural formatting. Uses shared types from `greentic_dev::dev_runner`.

- **Path:** `xtask/`  
  **Role:** Helper crate for repo tasks (e.g., docs/site generation, not deeply inspected here).  
  **Key functionality:** Supports ancillary automation; not part of primary runtime path.

## 3. Work In Progress, TODOs, and Stubs
- No explicit TODO/FIXME/HACK markers or `unimplemented!/todo!` stubs found in the codebase during scan.

## 4. Broken, Failing, or Conflicting Areas
- None observed after bumping to `greentic-runner-host`/`desktop` 0.4.10; pack smoke now passes end-to-end with `greentic-flow` 0.4.4 and the bundled `dev.greentic.echo` component.

## 5. Notes for Future Work
- After publishing a new release from current sources, verify `cargo binstall greentic-dev` install logs are clean and only include `greentic-dev` binary (no bundled delegated binaries).
- GitHub Actions now installs delegated tool binaries through `greentic-dev install tools --latest`; keep workflow and CLI install behavior aligned if delegated tool names change.
- GitHub Actions cache paths were tightened to cache cargo index/cache/git only (not full `CARGO_HOME` source trees) to reduce flaky registry-source mutation issues during `greentic-interfaces` WIT staging.
- GitHub Pages workflow has been removed; docs are no longer auto-published from this repository via Actions.
- Verify the GitHub Actions release workflow after recent changes to ensure matrix builds and asset uploads run for both tag and master pushes without YAML errors.
- Keep an eye on upstream runner/flow releases; currently using `greentic-flow` 0.4.4 with `greentic-runner-host`/`desktop` 0.4.10. Upgrade together as new versions land.
- Flow semantics audit added: see `docs/audits/flow_semantics_in_dev.md` and `docs/audits/flow_move_plan.md` for non-pass-through behaviors in `flow_cmd.rs` (config-flow rendering, manifest normalization, add-step orchestration) and a migration plan to move semantics into greentic-flow.
- Component resolution audit updated: see `docs/audits/components_semantics_in_dev.md` for current behavior; transport move plan in `docs/audits/distributor_client_move_plan.md`; component semantics move plan in `docs/audits/component_move_plan.md`.
