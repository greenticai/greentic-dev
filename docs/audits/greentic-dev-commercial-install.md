# Audit: greentic-dev Commercial Install Flow

Date: 2026-03-08

## Summary

`greentic-dev` does not currently have a general install/catalog workflow.

Today, `greentic-dev install` only supports one subcommand:

- `greentic-dev install tools`

That path is a thin wrapper around `cargo binstall` for a fixed open source tool list. There is no current concept of:

- installable commercial tools
- tenant-gated manifests
- documentation guide installation
- token validation in the install path

The closest existing tenant-aware code is the distributor profile/client used by `component add` and `pack init`. That code already carries `tenant_id` and `token` in config, but it is not wired into the install workflow.

This audit recommends keeping the current delegated OSS tool install path intact while adding a separate manifest-driven install layer that can merge:

- built-in open source install specs
- tenant-resolved commercial specs from `greentic-biz`

Licensing and token validation should stay outside `greentic-dev`, with the CLI acting as a manifest consumer rather than an entitlement engine.

## Current State

## Current CLI surface

Relevant code:

- `src/main.rs`
- `src/cli.rs`
- `src/cmd/tools.rs`
- `src/passthrough.rs`

Observed behavior:

1. `src/cli.rs` defines:
   - `tools install`
   - `install tools`
2. `src/main.rs` maps both to `tools::install(args.latest)`.
3. `src/cmd/tools.rs` immediately calls `install_all_delegated_tools`.
4. `src/passthrough.rs` installs a fixed list of seven binaries via `cargo binstall`.

Current delegated install set:

- `greentic-component`
- `greentic-flow`
- `greentic-pack`
- `greentic-runner`
- `greentic-runner-cli`
- `greentic-gui`
- `greentic-secrets`

This means the current install pipeline is:

```text
greentic-dev install tools [--latest]
        |
        v
ensure cargo-binstall exists
        |
        v
for each hard-coded InstallSpec
        |
        v
run cargo binstall --locked <crate> --bin <bin>
```

There is no resolver, manifest fetch, remote catalog merge, or doc install step in this flow.

## Existing tenant/token support

Relevant code:

- `src/config.rs`
- `src/distributor.rs`
- `docs/distributor.md`

Existing support already present outside install:

- config supports `token`
- config supports `tenant_id`
- token supports `env:VAR` indirection
- distributor client sends `Authorization: Bearer <token>`

This is currently used for distributor-backed artifact resolution, not tool installation.

Important constraint:

- install does not currently accept `--tenant` or `--token`
- there is no global shared profile selection in the install command
- tenant-aware behavior is isolated to distributor flows

## Gaps Against The Proposed Commercial Flow

The PR goal assumes a workflow like:

```bash
greentic-dev install
greentic-dev install --tenant acme --token <token>
```

That does not match the current CLI shape.

Current reality:

- `install` requires a subcommand
- install logic only knows how to install Rust binaries from crates
- no install abstraction exists for `gtpack`, OCI artifacts, repos, or docs

So the commercial design needs two things:

1. A new install catalog abstraction.
2. A CLI surface that can express tenant-aware catalog resolution without breaking existing `install tools` behavior.

## Recommended Architecture

## Layer split

Recommended responsibility split:

| Layer | Responsibility |
| --- | --- |
| `greentic-dev` | CLI UX, manifest fetch, merge, cache, install execution |
| `greentic-biz` | Tenant entitlements, commercial tool metadata, guide mappings |
| distribution service | Token validation, manifest authorization, revocation |

This keeps commercial enforcement outside the OSS CLI.

## Install model

Introduce an install catalog model with two sources:

1. Built-in/public open source catalog
2. Tenant manifest resolved from `greentic-biz` or a distribution API

Merged model:

```text
install_set = open_source_catalog
if tenant manifest present:
    install_set += tenant catalog entries
```

The current hard-coded delegated tool list becomes the initial OSS catalog source.

## CLI recommendation

Recommended future shape:

```bash
greentic-dev install
greentic-dev install --tenant acme --token env:GREENTIC_TOKEN
greentic-dev install tools --latest
```

Recommended behavior:

- `install`
  - installs from the install catalog
  - no tenant: OSS-only entries
  - tenant + token: OSS + tenant-authorized entries
- `install tools`
  - preserved as the legacy explicit delegated-binaries path
  - remains a thin wrapper over `cargo binstall`

This avoids breaking current users while allowing the new UX described in the PR.

Implementation note:

- `src/cli.rs` currently models `Install` only as a subcommand enum.
- Supporting bare `greentic-dev install` will require reshaping the Clap command model.
- This is a CLI-breaking internal change, but the external compatibility can be preserved by keeping `install tools`.

## Where `--tenant` and `--token` should live

Recommended precedence:

1. explicit CLI flags on `install`
2. named install/distributor profile from config
3. env vars
4. unauthenticated OSS-only mode

Recommended new flags:

- `--tenant <tenant>`
- `--token <token-or-env:VAR>`
- `--profile <name>` optional, if reusing distributor-style profiles

Recommendation:

- do not add `--tenant` and `--token` to `install tools`
- add them only to the new manifest-driven `install` path

Reason:

- `install tools` is currently a deterministic OSS-only `cargo binstall` wrapper
- overloading that path with remote entitlement logic will make behavior harder to reason about

## Token validation location

Token validation should not happen in `greentic-dev`.

Recommended flow:

1. CLI sends tenant + token to manifest source.
2. Manifest source validates token externally.
3. CLI receives a resolved manifest.
4. CLI installs exactly what the manifest allows.

This aligns with the existing distributor pattern, where the CLI passes a bearer token but does not interpret commercial policy.

## Tenant manifest design

## Tenant manifest role

The tenant manifest should be the entitlement contract returned to the CLI. It should describe:

- installable tools
- installable docs/guides
- optional components/bundles
- source and install method per artifact

Recommended rule:

- tenant manifests may add entries
- tenant manifests should not silently override built-in OSS entries by name unless explicitly version-pinned and allowed by policy

Default merge policy:

- OSS entries are the base
- tenant entries can append new tool IDs
- collisions fail unless an explicit override flag is set in the manifest schema

## Proposed `greentic-biz` layout

```text
greentic-biz/
├─ tenants/
│  ├─ acme.json
│  ├─ zain.json
│  └─ meeza.json
├─ tools/
│  ├─ greentic-x/
│  │  ├─ manifest.json
│  │  └─ README.md
│  ├─ telecom-playbooks/
│  │  ├─ manifest.json
│  │  └─ README.md
│  └─ ai-ops/
│     ├─ manifest.json
│     └─ README.md
└─ docs/
   ├─ install-guides/
   └─ onboarding/
```

Recommended repo responsibilities:

- `tenants/`: per-customer entitlements
- `tools/`: metadata for each installable commercial artifact
- `docs/`: guide payloads referenced by tenant manifests

## Proposed manifest shapes

Tenant manifest:

```json
{
  "schema_version": "1",
  "tenant": "acme",
  "tools": [
    "greentic-x",
    "telecom-playbooks"
  ],
  "docs": [
    "acme-onboarding",
    "acme-architecture"
  ],
  "components": [
    "component-ticketing",
    "component-ai-router"
  ]
}
```

Tool manifest:

```json
{
  "schema_version": "1",
  "name": "telecom-playbooks",
  "description": "Telecom digital worker playbooks",
  "install": {
    "type": "gtpack"
  },
  "artifacts": [
    {
      "uri": "oci://ghcr.io/greenticai/playbooks/telecom:v1",
      "platform": "any"
    }
  ],
  "docs": [
    "telecom-playbooks-readme"
  ]
}
```

Doc manifest:

```json
{
  "schema_version": "1",
  "id": "acme-onboarding",
  "title": "Acme onboarding",
  "source": {
    "type": "markdown",
    "path": "docs/onboarding/acme.md"
  }
}
```

## Install artifact types

The install system should support typed entries rather than assuming everything is a Rust crate.

Recommended initial install types:

- `cargo-binstall-bin`
- `gtpack`
- `oci`
- `file`
- `git`
- `doc`

This is the core missing abstraction in today’s `src/passthrough.rs`.

## CLI flow

Recommended flow:

```text
greentic-dev install
      |
      v
load built-in OSS install catalog
      |
tenant flag present?
      |
 no   | yes
      |-----------------------------+
      |                             v
      |                    resolve tenant manifest
      |                    via greentic-biz or distribution API
      |                             |
      +-------------> merge catalogs
                                |
                                v
                        resolve artifact installers
                                |
                                v
                         install tools and docs
                                |
                                v
                         write local install cache
```

## Documentation guide handling

There is no existing guide installer in `greentic-dev`.

Recommendation for MVP:

- install docs into a user cache directory, not into the current workspace
- keep docs discoverable through a local manifest/index file

Suggested local layout:

```text
~/.greentic/install/
├─ manifests/
│  ├─ oss.json
│  └─ tenant-acme.json
├─ docs/
│  ├─ acme-onboarding/
│  │  └─ README.md
│  └─ acme-architecture/
│     └─ README.md
└─ state.json
```

Reason:

- installation is user-scoped, not workspace-scoped
- docs should remain available across workspaces

## Caching

Recommended caching policy:

- cache resolved tenant manifest locally
- cache downloaded artifacts by digest when available
- record the source manifest version used for each installed artifact

Recommended invalidation policy:

- `install` refreshes manifests by default
- `install --offline` uses the last cached manifest
- `install --latest` forces artifact refresh where supported

## Integration Points In Current Code

## Existing code that can be reused

1. `src/config.rs`
   - already models `token`, `tenant_id`, `environment_id`
   - can be extended or reused for install/distribution profiles
2. `src/distributor.rs`
   - already handles bearer token wiring
   - already supports `env:VAR` token indirection
   - can inspire a manifest client, but should not be overloaded blindly
3. `src/passthrough.rs`
   - current OSS delegated tool list can seed the base open source catalog

## Code that likely needs to change

1. `src/cli.rs`
   - add bare `install` command args
   - optionally add `--profile`, `--tenant`, `--token`, `--offline`
2. `src/main.rs`
   - route bare `install` into a new install catalog path
   - preserve `install tools`
3. New install modules, for example:
   - `src/install/catalog.rs`
   - `src/install/manifest.rs`
   - `src/install/merge.rs`
   - `src/install/executor.rs`
   - `src/install/cache.rs`
4. `src/passthrough.rs`
   - keep legacy `cargo binstall` path
   - optionally expose current fixed specs as catalog entries

## Recommended merge semantics

Recommended install merge rules:

1. Start with OSS built-in catalog.
2. If tenant manifest exists, load tenant tool/doc IDs.
3. Resolve each ID to a concrete manifest entry.
4. Deduplicate by stable install ID.
5. Reject conflicting definitions unless explicitly marked overrideable.

Recommended precedence:

- identical ID + identical artifact: dedupe
- identical ID + different artifact: fail
- explicit override policy in tenant manifest: allow with warning

This avoids accidental shadowing of OSS tools by a tenant catalog.

## Distribution Source Recommendation

Two candidate sources were described:

- raw GitHub content
- distribution API

Recommendation:

- long-term: distribution API
- MVP: distribution API preferred if token validation already exists
- fallback MVP: GitHub-backed manifest storage behind a validation service, not direct raw GitHub token access from the CLI

Reason:

- direct GitHub raw fetch is weak as an entitlement boundary
- revocation, expiry, and auditability are better handled by a dedicated service

If GitHub is used at all, it should remain a storage backend behind a service, not the client-facing auth boundary.

## Follow-up PR Plan

### PR-DEV-TENANT-INSTALL-02

Add a new manifest-driven `greentic-dev install` path.

Scope:

- new CLI args: `--tenant`, `--token`, optional `--profile`
- new install catalog abstraction
- preserve `install tools`

### PR-BIZ-REPO-01

Create `greentic-biz` repository structure and JSON schemas.

Scope:

- `tenants/`
- `tools/`
- `docs/`
- manifest examples and validation rules

### PR-DEV-TENANT-TOOLS-03

Implement manifest merge, artifact execution, and local install cache.

Scope:

- merge OSS and tenant entries
- support at least `cargo-binstall-bin` and `doc`
- optional `gtpack`/`oci` as follow-up if not ready

### PR-DEV-TENANT-DOCS-04

Add documentation installation and local guide index.

Scope:

- docs cache layout
- local discoverability
- manifest-to-doc rendering/copy flow

### PR-DISTRIBUTION-AUTH-01

Implement manifest resolution endpoint and token validation outside the CLI.

Scope:

- tenant/token validation
- manifest delivery
- revocation and expiry semantics

## Decisions

Recommended decisions from this audit:

1. Keep the current `install tools` path unchanged.
2. Add a new manifest-driven bare `install` flow instead of mutating the legacy tool installer.
3. Reuse existing config/profile/token conventions where possible.
4. Keep token validation outside `greentic-dev`.
5. Use `greentic-biz` as the entitlement data source, but not as the enforcement layer by itself.
6. Treat tenant manifests as additive by default; do not allow silent overrides.

## Open Questions

These need resolution in implementation PRs:

- Should commercial docs be cached only, or also copied into the workspace on request?
- Should tenant installs include components directly, or only tools that later resolve components?
- Is `oci` support required in the first installer PR, or can MVP ship with `cargo-binstall-bin` and `doc` only?
- Should install profile config live under `[distributor]`, or under a new `[install]` section that can still reuse the same token parsing rules?
- What exact command should list installed/commercially available tools after installation?

## Acceptance Mapping

This audit now documents:

- current `greentic-dev` install pipeline
- where `--tenant` and `--token` should be integrated
- a proposed tenant manifest system
- a proposed `greentic-biz` repository layout
- CLI changes needed for tenant-aware install
- follow-up PRs needed to implement the design
