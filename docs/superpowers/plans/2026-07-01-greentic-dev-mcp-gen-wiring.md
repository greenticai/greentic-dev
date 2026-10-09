# greentic-dev `mcp gen` Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose `greentic-mcp-generator` through `greentic-dev` as a passthrough subcommand `greentic-dev mcp gen`, so authors can generate a `wasix:mcp@25.06.18` router component from an OpenAPI/Swagger spec without leaving the cockpit CLI.

**Architecture:** `greentic-dev` stays a thin wrapper. A new channel-agnostic resolver locates the external `greentic-mcp-gen` binary; the existing pre-parse delegation hook (`maybe_delegate_mcp_passthrough` in `main.rs`) routes `mcp gen <ARGS…>` to it and forwards argv verbatim, propagating the exit code. Supporting work: install-catalogue registration, `mcp doctor` reporting, and i18n strings. No OpenAPI/wasm dependencies enter `greentic-dev`.

**Tech Stack:** Rust 2024 (edition 2024, toolchain 1.95.0), `clap` (manual + derive), `anyhow`, `which`, embedded JSON i18n catalogs.

## Global Constraints

- Rust toolchain **1.95.0** (`rust-toolchain.toml`, do not edit); edition **2024** (`std::env::set_var` requires `unsafe` — tests must avoid env mutation).
- No `unwrap()` / `panic!()` on production paths — use `anyhow` with context.
- English only in source/tests/comments; all user-facing strings via `crate::i18n::t` / `tf` (never hardcode).
- `#![forbid(unsafe_code)]`-style discipline: the only accepted `unsafe`/command-exec is the existing passthrough pattern (exec resolved tool by argv, never a shell) with its `// Accepted risk` + `// foxguard: ignore[rs/no-command-injection]` comments.
- Conventional Commits (`feat:`, `fix:`, `docs:`).
- Gate before "done": `bash ci/local_check.sh` (fmt + clippy `-D warnings` + tests).
- i18n fallback resolves to `en`, so new keys land in `i18n/en.json` (no key-parity test exists).
- Generator binary is unsuffixed (`greentic-mcp-gen`); never apply the `-dev`/`-rnd` channel suffix to it.

## File Structure

- `src/passthrough.rs` — add `external_tool_env_key()` + `resolve_external_tool()` (channel-agnostic resolve); extend `install_all_delegated_tools()` to also install external tools. (Task 1, Task 2)
- `src/toolchain_catalogue.rs` — add `GREENTIC_EXTERNAL_TOOL_PACKAGES` const with the generator. (Task 2)
- `src/main.rs` — route `mcp gen` in `maybe_delegate_mcp_passthrough` via a pure arg-slicer + delegation; defensive clap dispatch arm. (Task 3, Task 4)
- `src/cli.rs` — add `McpCommand::Gen(PassthroughArgs)` variant + `mcp gen` help node. (Task 4)
- `src/mcp_cmd.rs` — add `GeneratorStatus` and surface it in `doctor` (text + JSON). (Task 5)
- `i18n/en.json` — new keys for gen about, guided install error, doctor lines. (Tasks 3 & 5)
- `README.md`, `.codex/repo_overview.md` — docs + POST-PR sync. (Task 6)

---

### Task 1: Channel-agnostic external-tool resolver

**Files:**
- Modify: `src/passthrough.rs` (add functions near `resolve_binary`, ~line 70)
- Test: `src/passthrough.rs` (`#[cfg(test)]` module, ~line 384)

**Interfaces:**
- Produces:
  - `pub(crate) fn external_tool_env_key(name: &str) -> String`
  - `pub fn resolve_external_tool(name: &str) -> anyhow::Result<std::path::PathBuf>`

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` in `src/passthrough.rs`:

```rust
#[test]
fn external_tool_env_key_is_plain_uppercase_no_channel_suffix() {
    // The key derives from the plain binary name; it must never carry a
    // `-dev`/`-rnd` channel suffix.
    assert_eq!(
        external_tool_env_key("greentic-mcp-gen"),
        "GREENTIC_DEV_BIN_GREENTIC_MCP_GEN"
    );
}

#[test]
fn resolve_external_tool_errors_with_plain_name_when_absent() {
    // A name that is not on PATH and has no env override resolves to an error
    // that mentions the plain (unsuffixed) name.
    let err = resolve_external_tool("greentic-mcp-gen-absent-xyz")
        .expect_err("expected resolution to fail");
    assert!(err.to_string().contains("greentic-mcp-gen-absent-xyz"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p greentic-dev external_tool -- --nocapture`
Expected: FAIL — `external_tool_env_key` / `resolve_external_tool` not found.

- [ ] **Step 3: Implement the resolver**

Add to `src/passthrough.rs` (after `resolve_binary_for_channel`):

```rust
/// Environment-override key for an external tool, e.g. `greentic-mcp-gen`
/// → `GREENTIC_DEV_BIN_GREENTIC_MCP_GEN`.
pub(crate) fn external_tool_env_key(name: &str) -> String {
    format!("GREENTIC_DEV_BIN_{}", name.replace('-', "_").to_uppercase())
}

/// Resolve an external (non-Greentic-channel) tool binary by its plain name.
///
/// Unlike [`resolve_binary`], this never appends the toolchain channel suffix
/// (`-dev`/`-rnd`): external tools such as `greentic-mcp-gen` ship a single,
/// unsuffixed binary. Resolution order: `GREENTIC_DEV_BIN_<NAME>` env override,
/// then `PATH`.
pub fn resolve_external_tool(name: &str) -> Result<PathBuf> {
    let locale = crate::i18n::select_locale(None);
    let env_key = external_tool_env_key(name);
    if let Ok(path) = env::var(&env_key) {
        let pb = PathBuf::from(path);
        if pb.exists() {
            return Ok(pb);
        }
        bail!(
            "{}",
            crate::i18n::tf(
                &locale,
                "runtime.passthrough.error.env_binary_missing",
                &[
                    ("env_key", env_key.clone()),
                    ("path", pb.display().to_string()),
                ],
            )
        );
    }

    if let Ok(path) = which::which(name) {
        return Ok(path);
    }

    bail!(
        "{}",
        crate::i18n::tf(
            &locale,
            "runtime.passthrough.error.binary_not_found",
            &[("name", name.to_string()), ("env_key", env_key)],
        )
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p greentic-dev external_tool -- --nocapture`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add src/passthrough.rs
git commit -m "feat: add channel-agnostic external-tool resolver"
```

---

### Task 2: Register the generator in the install catalogue

**Files:**
- Modify: `src/toolchain_catalogue.rs` (add const after `GREENTIC_TOOLCHAIN_PACKAGES`)
- Modify: `src/passthrough.rs` (`install_all_delegated_tools`, ~line 154; import the new const)
- Test: `src/toolchain_catalogue.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: `ToolchainPackageSpec { crate_name, bins }` (existing).
- Produces: `pub const GREENTIC_EXTERNAL_TOOL_PACKAGES: &[ToolchainPackageSpec]`.

- [ ] **Step 1: Write the failing test**

Add to `src/toolchain_catalogue.rs` (create a `#[cfg(test)] mod tests` if none exists):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_packages_include_mcp_generator() {
        let found = GREENTIC_EXTERNAL_TOOL_PACKAGES.iter().find(|pkg| {
            pkg.crate_name == "greentic-mcp-generator"
        });
        let pkg = found.expect("generator must be registered as an external tool");
        assert_eq!(pkg.bins, &["greentic-mcp-gen"]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p greentic-dev external_packages_include_mcp_generator`
Expected: FAIL — `GREENTIC_EXTERNAL_TOOL_PACKAGES` not found.

- [ ] **Step 3: Add the catalogue entry**

In `src/toolchain_catalogue.rs`, after the `GREENTIC_TOOLCHAIN_PACKAGES` array:

```rust
/// External tools distributed as their own single, unsuffixed binary
/// (not part of the Greentic channel-suffixed toolchain). Resolved and
/// installed by plain name — never with a `-dev`/`-rnd` suffix.
pub const GREENTIC_EXTERNAL_TOOL_PACKAGES: &[ToolchainPackageSpec] = &[
    ToolchainPackageSpec {
        crate_name: "greentic-mcp-generator",
        bins: &["greentic-mcp-gen"],
    },
];
```

- [ ] **Step 4: Install external tools alongside the delegated ones**

In `src/passthrough.rs`, update the import at the top:

```rust
use crate::toolchain_catalogue::{GREENTIC_EXTERNAL_TOOL_PACKAGES, GREENTIC_TOOLCHAIN_PACKAGES};
```

Then extend `install_all_delegated_tools`, adding the external loop before `Ok(())`:

```rust
pub fn install_all_delegated_tools(latest: bool, locale: &str) -> Result<()> {
    ensure_cargo_binstall()?;
    let channel = current_toolchain_channel();
    for package in GREENTIC_TOOLCHAIN_PACKAGES {
        for bin_name in package.bins {
            install_with_binstall(
                package.crate_name,
                &delegated_binary_name_for_channel(bin_name, channel),
                latest,
                locale,
            )?;
        }
    }
    // External tools ship a single unsuffixed binary — install by plain name.
    for package in GREENTIC_EXTERNAL_TOOL_PACKAGES {
        for bin_name in package.bins {
            install_with_binstall(package.crate_name, bin_name, latest, locale)?;
        }
    }
    Ok(())
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p greentic-dev external_packages_include_mcp_generator`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/toolchain_catalogue.rs src/passthrough.rs
git commit -m "feat: register greentic-mcp-generator as an installable external tool"
```

---

### Task 3: Route `mcp gen` to the generator (functional core)

**Files:**
- Modify: `src/main.rs` (`maybe_delegate_mcp_passthrough`, ~line 77; add pure arg-slicer + `#[cfg(test)] mod tests`)
- Modify: `i18n/en.json` (add keys)

**Interfaces:**
- Consumes: `passthrough::resolve_external_tool` (Task 1), `passthrough::run_passthrough`.
- Produces: `fn mcp_gen_args(argv: &[OsString]) -> Option<Vec<OsString>>` (module-private in `main.rs`).

- [ ] **Step 1: Add i18n keys**

In `i18n/en.json`, add these entries (top-level string keys):

```json
  "cli.command.mcp.gen.about": "Generate an MCP router component from an OpenAPI/Swagger spec (passthrough to greentic-mcp-gen)",
  "runtime.mcp.gen.error.not_installed": "greentic-mcp-gen was not found. Install it with `cargo binstall greentic-mcp-generator` (set GITHUB_TOKEN for the private repo) or run `greentic-dev install`."
```

- [ ] **Step 2: Write the failing test**

Add a `#[cfg(test)] mod tests` to `src/main.rs` (or extend the existing one):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn argv(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn mcp_gen_args_captures_everything_after_gen() {
        let a = argv(&["greentic-dev", "mcp", "gen", "--spec", "./api.yaml", "--output-dir", "./out"]);
        let forwarded = mcp_gen_args(&a).expect("mcp gen should be recognized");
        assert_eq!(
            forwarded,
            argv(&["--spec", "./api.yaml", "--output-dir", "./out"])
        );
    }

    #[test]
    fn mcp_gen_args_forwards_subcommand_style_args_verbatim() {
        let a = argv(&["greentic-dev", "mcp", "gen", "discovery", "--url", "https://x/y", "--dry-run"]);
        let forwarded = mcp_gen_args(&a).expect("mcp gen should be recognized");
        assert_eq!(
            forwarded,
            argv(&["discovery", "--url", "https://x/y", "--dry-run"])
        );
    }

    #[test]
    fn mcp_gen_args_ignores_non_gen() {
        assert!(mcp_gen_args(&argv(&["greentic-dev", "mcp", "doctor", "providers"])).is_none());
        assert!(mcp_gen_args(&argv(&["greentic-dev", "flow", "gen"])).is_none());
        assert!(mcp_gen_args(&argv(&["greentic-dev", "mcp"])).is_none());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p greentic-dev mcp_gen_args`
Expected: FAIL — `mcp_gen_args` not found.

- [ ] **Step 4: Add the pure arg-slicer**

In `src/main.rs`, add near `rewritten_mcp_passthrough_args`:

```rust
/// If `argv` is `greentic-dev mcp gen …`, return the tokens after `gen`
/// (everything to forward to the generator). Otherwise `None`.
fn mcp_gen_args(argv: &[OsString]) -> Option<Vec<OsString>> {
    if argv.get(1)?.to_str()? != "mcp" {
        return None;
    }
    if argv.get(2)?.to_str()? != "gen" {
        return None;
    }
    Some(argv[3..].to_vec())
}
```

- [ ] **Step 5: Route `gen` in the pre-parse delegation hook**

In `src/main.rs`, modify `maybe_delegate_mcp_passthrough`. After the `if matches!(mcp_arg, "doctor" | "-h" | "--help")` early-return and **before** the `resolve_binary("greentic-mcp")` fallback, insert:

```rust
    if mcp_arg == "gen" {
        let locale = crate::i18n::select_locale(
            crate::i18n::cli_locale_from_argv(argv).as_deref(),
        );
        // Safe: `mcp_gen_args` returns Some here because argv[1]=="mcp" && argv[2]=="gen".
        let forwarded = mcp_gen_args(argv).unwrap_or_default();
        let bin = greentic_dev::passthrough::resolve_external_tool("greentic-mcp-gen")
            .map_err(|_| {
                anyhow::anyhow!(crate::i18n::t(&locale, "runtime.mcp.gen.error.not_installed"))
            })?;
        let status = run_passthrough(&bin, &forwarded, false)?;
        std::process::exit(status.code().unwrap_or(1));
    }
```

Add the import for `i18n` if not already in scope (the module is `greentic_dev::i18n`; if `crate::i18n` is not valid in `main.rs`, use `greentic_dev::i18n`). Confirm which path compiles — the file already calls `greentic_dev::i18n::select_locale` in `main`, so use `greentic_dev::i18n::…` here too:

```rust
    if mcp_arg == "gen" {
        let locale = greentic_dev::i18n::select_locale(
            greentic_dev::i18n::cli_locale_from_argv(argv).as_deref(),
        );
        let forwarded = mcp_gen_args(argv).unwrap_or_default();
        let bin = greentic_dev::passthrough::resolve_external_tool("greentic-mcp-gen")
            .map_err(|_| {
                anyhow::anyhow!(greentic_dev::i18n::t(&locale, "runtime.mcp.gen.error.not_installed"))
            })?;
        let status = run_passthrough(&bin, &forwarded, false)?;
        std::process::exit(status.code().unwrap_or(1));
    }
```

- [ ] **Step 6: Run tests + build to verify**

Run: `cargo test -p greentic-dev mcp_gen_args`
Expected: PASS.
Run: `cargo build -p greentic-dev`
Expected: builds clean.

- [ ] **Step 7: Manual smoke (optional, no binary needed)**

Run: `GREENTIC_DEV_BIN_GREENTIC_MCP_GEN=/nonexistent cargo run -p greentic-dev -- mcp gen --help`
Expected: exits non-zero printing the guided "greentic-mcp-gen was not found…" message (the env override points at a missing path, so resolution fails with the guided error).

- [ ] **Step 8: Commit**

```bash
git add src/main.rs i18n/en.json
git commit -m "feat: route \`greentic-dev mcp gen\` to greentic-mcp-gen passthrough"
```

---

### Task 4: clap discoverability — `mcp gen` in help + defensive dispatch

**Files:**
- Modify: `src/cli.rs` (`McpCommand` enum, ~line 528; `mut_subcommand("mcp", …)` block, ~line 131)
- Modify: `src/main.rs` (`Command::Mcp` match arm, ~line 77)

**Interfaces:**
- Consumes: `PassthroughArgs` (existing), `passthrough::resolve_external_tool` (Task 1).
- Produces: `McpCommand::Gen(PassthroughArgs)` variant.

- [ ] **Step 1: Add the `Gen` variant**

In `src/cli.rs`, extend the `McpCommand` enum:

```rust
#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// cli.command.mcp.doctor.about
    Doctor(McpDoctorArgs),
    /// cli.command.mcp.gen.about
    Gen(PassthroughArgs),
}
```

- [ ] **Step 2: Wire the localized help node**

In `src/cli.rs`, inside the `.mut_subcommand("mcp", |sub| { … })` block, chain a `gen` node after the `doctor` node:

```rust
        .mut_subcommand("mcp", |sub| {
            sub.about(crate::i18n::t(locale, "cli.command.mcp.about"))
                .mut_subcommand("doctor", |sub| {
                    sub.about(crate::i18n::t(locale, "cli.command.mcp.doctor.about"))
                        .mut_arg("provider", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.mcp.doctor.provider"))
                        })
                        .mut_arg("json", |arg| {
                            arg.help(crate::i18n::t(locale, "cli.command.mcp.doctor.json"))
                        })
                })
                .mut_subcommand("gen", |sub| {
                    sub.about(crate::i18n::t(locale, "cli.command.mcp.gen.about"))
                })
        })
```

- [ ] **Step 3: Add the defensive dispatch arm**

In `src/main.rs`, extend the `Command::Mcp` match (reachable only if pre-parse delegation is bypassed; keeps behavior correct and the match exhaustive):

```rust
        Command::Mcp(mcp) => match mcp {
            McpCommand::Doctor(args) => mcp_cmd::doctor(&args.provider, args.json),
            McpCommand::Gen(args) => {
                let bin = greentic_dev::passthrough::resolve_external_tool("greentic-mcp-gen")
                    .map_err(|_| {
                        anyhow::anyhow!(greentic_dev::i18n::t(
                            &selected_locale,
                            "runtime.mcp.gen.error.not_installed"
                        ))
                    })?;
                let status = run_passthrough(&bin, &args.args, false)?;
                std::process::exit(status.code().unwrap_or(1));
            }
        },
```

- [ ] **Step 4: Build + verify help lists `gen`**

Run: `cargo build -p greentic-dev`
Expected: builds clean (match is exhaustive).
Run: `cargo run -p greentic-dev -- mcp --help`
Expected: output lists both `doctor` and `gen` with the localized about text.

> Note: `greentic-dev mcp gen …` is still handled by the pre-parse hook (Task 3), so arbitrary generator args never reach clap. This arm is the exhaustive/defensive fallback.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "feat: surface \`mcp gen\` in clap help with defensive dispatch"
```

---

### Task 5: Extend `mcp doctor` with generator + wasm-toolchain status

**Files:**
- Modify: `src/mcp_cmd.rs` (add `GeneratorStatus`; add field to `ToolMapReport`; extend `print_report`)
- Test: `src/mcp_cmd.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: `passthrough::resolve_external_tool` (Task 1).
- Produces: `struct GeneratorStatus` with `fn absent() -> Self` and `fn detect() -> Self`; new `generator` field on `ToolMapReport`.

- [ ] **Step 1: Write the failing test**

Add to `src/mcp_cmd.rs` (in a `#[cfg(test)] mod tests`, create if absent):

```rust
#[cfg(test)]
mod generator_tests {
    use super::*;

    #[test]
    fn absent_generator_status_has_no_path_or_version() {
        let status = GeneratorStatus::absent();
        assert_eq!(status.binary_name, "greentic-mcp-gen");
        assert!(status.resolved_path.is_none());
        assert!(status.version.is_none());
    }

    #[test]
    fn generator_status_serializes_expected_fields() {
        let json = serde_json::to_value(GeneratorStatus::absent()).unwrap();
        assert!(json.get("binary_name").is_some());
        assert!(json.get("resolved_path").is_some()); // present as null
        assert!(json.get("version").is_some());
        assert!(json.get("cargo_available").is_some());
        assert!(json.get("wasm_target_installed").is_some());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p greentic-dev generator_tests`
Expected: FAIL — `GeneratorStatus` not found.

- [ ] **Step 3: Add `GeneratorStatus`**

In `src/mcp_cmd.rs`, add near `ToolMapReport`:

```rust
use std::process::Command as ProcessCommand;

#[derive(Debug, Serialize)]
struct GeneratorStatus {
    binary_name: String,
    resolved_path: Option<String>,
    version: Option<String>,
    cargo_available: bool,
    wasm_target_installed: bool,
}

impl GeneratorStatus {
    fn absent() -> Self {
        Self {
            binary_name: "greentic-mcp-gen".to_string(),
            resolved_path: None,
            version: None,
            cargo_available: cargo_available(),
            wasm_target_installed: wasm_target_installed(),
        }
    }

    fn detect() -> Self {
        match crate::passthrough::resolve_external_tool("greentic-mcp-gen") {
            Ok(path) => {
                let version = ProcessCommand::new(&path)
                    .arg("--version")
                    .output()
                    .ok()
                    .filter(|out| out.status.success())
                    .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                    .filter(|s| !s.is_empty());
                Self {
                    binary_name: "greentic-mcp-gen".to_string(),
                    resolved_path: Some(path.display().to_string()),
                    version,
                    cargo_available: cargo_available(),
                    wasm_target_installed: wasm_target_installed(),
                }
            }
            Err(_) => Self::absent(),
        }
    }
}

/// Best-effort: is `cargo` invokable?
fn cargo_available() -> bool {
    ProcessCommand::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Best-effort: is the `wasm32-wasip2` target installed?
fn wasm_target_installed() -> bool {
    ProcessCommand::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).contains("wasm32-wasip2"))
        .unwrap_or(false)
}
```

- [ ] **Step 4: Add the `generator` field to `ToolMapReport`**

In `src/mcp_cmd.rs`, add the field to the struct and populate it in `from_map` (additive — existing JSON gains a `generator` object):

```rust
#[derive(Debug, Serialize)]
struct ToolMapReport {
    tool_map_path: String,
    tool_count: usize,
    tools: Vec<ToolHealth>,
    warnings: Vec<String>,
    generator: GeneratorStatus,
}
```

At the end of `ToolMapReport::from_map`, set the new field in the returned struct literal:

```rust
        Self {
            tool_map_path: config_path.display().to_string(),
            tool_count: tools.len(),
            tools,
            warnings,
            generator: GeneratorStatus::detect(),
        }
```

- [ ] **Step 5: Render the generator section in `print_report`**

In `src/mcp_cmd.rs`, inside `print_report`, after the existing tool-map output, append (use the report's `generator` field; add i18n keys in Step 7):

```rust
    let locale = crate::i18n::select_locale(None);
    println!("{}", crate::i18n::t(&locale, "cli.command.mcp.doctor.generator.header"));
    match (&report.generator.resolved_path, &report.generator.version) {
        (Some(path), Some(version)) => println!(
            "  {}",
            crate::i18n::tf(
                &locale,
                "cli.command.mcp.doctor.generator.found",
                &[("path", path.clone()), ("version", version.clone())],
            )
        ),
        (Some(path), None) => println!(
            "  {}",
            crate::i18n::tf(
                &locale,
                "cli.command.mcp.doctor.generator.found",
                &[("path", path.clone()), ("version", "unknown".to_string())],
            )
        ),
        _ => println!("  {}", crate::i18n::t(&locale, "cli.command.mcp.doctor.generator.missing")),
    }
    if report.generator.cargo_available && report.generator.wasm_target_installed {
        println!("  {}", crate::i18n::t(&locale, "cli.command.mcp.doctor.toolchain.ready"));
    } else {
        if !report.generator.cargo_available {
            println!("  {}", crate::i18n::t(&locale, "cli.command.mcp.doctor.toolchain.cargo_missing"));
        }
        if !report.generator.wasm_target_installed {
            println!("  {}", crate::i18n::t(&locale, "cli.command.mcp.doctor.toolchain.wasm_missing"));
        }
    }
```

> If `print_report` does not currently take the report by reference in a way that exposes `generator`, confirm its signature is `fn print_report(report: &ToolMapReport)` and adjust the call site accordingly. The JSON path (`serde_json::to_string_pretty(&report)`) needs no change — the new field serializes automatically.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p greentic-dev generator_tests`
Expected: PASS.

- [ ] **Step 7: Add doctor i18n keys**

In `i18n/en.json`, add:

```json
  "cli.command.mcp.doctor.generator.header": "MCP generator (greentic-mcp-gen):",
  "cli.command.mcp.doctor.generator.found": "found: {path} ({version})",
  "cli.command.mcp.doctor.generator.missing": "not installed — run `greentic-dev install` or `cargo binstall greentic-mcp-generator`",
  "cli.command.mcp.doctor.toolchain.ready": "wasm toolchain ready (cargo + wasm32-wasip2)",
  "cli.command.mcp.doctor.toolchain.cargo_missing": "cargo not found on PATH",
  "cli.command.mcp.doctor.toolchain.wasm_missing": "wasm32-wasip2 target missing — run: rustup target add wasm32-wasip2"
```

- [ ] **Step 8: Build + verify**

Run: `cargo build -p greentic-dev`
Expected: builds clean.

- [ ] **Step 9: Commit**

```bash
git add src/mcp_cmd.rs i18n/en.json
git commit -m "feat: report greentic-mcp-gen + wasm toolchain status in mcp doctor"
```

---

### Task 6: Docs + final gate

**Files:**
- Modify: `README.md`
- Modify: `.codex/repo_overview.md`

- [ ] **Step 1: Document `mcp gen` in the README**

Add a section to `README.md` (near the MCP / passthrough documentation):

````markdown
## Generate MCP components from OpenAPI (`mcp gen`)

`greentic-dev mcp gen` is a passthrough to the `greentic-mcp-gen` binary
(from `greentic-mcp-generator`). Every argument after `gen` is forwarded
verbatim, so the full generator surface is available:

```bash
# Generate from an OpenAPI/Swagger spec
greentic-dev mcp gen --spec ./api.yaml --output-dir ./out

# Google Discovery pipeline
greentic-dev mcp gen discovery --url "https://sheets.googleapis.com/\$discovery/rest?version=v4" --profile sheets-crm --out ./out
```

The generator is a separate, private tool. Install it once with
`cargo binstall greentic-mcp-generator` (set `GITHUB_TOKEN` for the private
repo) or via `greentic-dev install`. Override the resolved binary with
`GREENTIC_DEV_BIN_GREENTIC_MCP_GEN`. Building the resulting `.wasm` requires
`cargo` + the `wasm32-wasip2` target; run `greentic-dev mcp doctor <toolmap>`
to check readiness.
````

- [ ] **Step 2: Sync `.codex/repo_overview.md`**

Update the relevant section of `.codex/repo_overview.md` to note the new `mcp gen`
passthrough, the `GREENTIC_EXTERNAL_TOOL_PACKAGES` catalogue, and the extended
`mcp doctor` reporting. Keep it factual and one paragraph.

- [ ] **Step 3: Run the full local CI gate**

Run: `bash ci/local_check.sh`
Expected: fmt clean, clippy `-D warnings` clean, all tests pass. If a failure is
outside this change's scope, document it in the PR summary rather than hiding it.

- [ ] **Step 4: Commit**

```bash
git add README.md .codex/repo_overview.md
git commit -m "docs: document \`mcp gen\` passthrough and toolchain readiness"
```

---

## Self-Review

**Spec coverage:**
- Command surface `mcp gen` + full argv passthrough → Task 3 (routing) + Task 4 (clap surface). ✅
- Passthrough mechanism, no library dep → Tasks 1/3, no `Cargo.toml` dependency added. ✅
- Channel-suffix gotcha (resolve by plain name) → Task 1 `resolve_external_tool`. ✅
- Guided error, no auto-install → Task 3 `runtime.mcp.gen.error.not_installed`. ✅
- Install-catalogue registration → Task 2. ✅
- `mcp doctor` extension (generator + wasm toolchain) → Task 5. ✅
- i18n for all new strings → Tasks 3 & 5 (`i18n/en.json`). ✅
- Exit-code propagation verbatim → Tasks 3 & 4 (`std::process::exit(status.code().unwrap_or(1))`). ✅
- Security: reuse passthrough exec pattern, no new shell → `run_passthrough` reused. ✅
- Docs + `.codex` sync + CI gate → Task 6. ✅

**Placeholder scan:** No TBD/TODO; every code step has complete code. The two "confirm the signature" notes (Task 3 Step 5 i18n path, Task 5 Step 5 `print_report` signature) are verification guards with a concrete fallback given, not placeholders.

**Type consistency:** `resolve_external_tool(&str) -> Result<PathBuf>` used identically in Tasks 3, 4, 5. `external_tool_env_key` naming consistent. `GeneratorStatus` fields (`binary_name`, `resolved_path`, `version`, `cargo_available`, `wasm_target_installed`) match between definition (Task 5 Step 3), test (Step 1), and render (Step 5). `mcp_gen_args` signature identical in definition and tests. `GREENTIC_EXTERNAL_TOOL_PACKAGES` type matches `GREENTIC_TOOLCHAIN_PACKAGES` (`&[ToolchainPackageSpec]`).
