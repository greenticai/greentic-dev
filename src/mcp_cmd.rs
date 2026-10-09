use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, btree_map::Entry};

use crate::path_safety::normalize_under_root;

pub fn doctor(target: Option<&str>, json: bool) -> Result<()> {
    let Some(target) = target else {
        // No tool map supplied: report only generator + wasm toolchain readiness.
        let generator = GeneratorStatus::detect();
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&generator).context("failed to encode JSON report")?
            );
        } else {
            print_generator_status(&generator);
        }
        return Ok(());
    };

    let workspace_root = std::env::current_dir()
        .context("failed to resolve workspace root")?
        .canonicalize()
        .context("failed to canonicalize workspace root")?;
    let config_path = locate_toolmap(&workspace_root, target)?;
    let config = load_tool_map_config(&config_path)
        .with_context(|| format!("failed to load MCP tool map from {}", config_path.display()))?;
    let map = ToolMap::from_config(&config).context("tool map contains duplicate tool names")?;
    let report = ToolMapReport::from_map(&config_path, &map);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).context("failed to encode JSON report")?
        );
    } else {
        print_report(&report);
    }

    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct ToolRef {
    name: String,
    component: String,
    entry: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_retries: Option<u32>,
    #[serde(default)]
    retry_backoff_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolMapConfig {
    tools: Vec<ToolRef>,
}

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

#[derive(Debug, Clone)]
struct ToolMap {
    tools: BTreeMap<String, ToolRef>,
}

impl ToolMap {
    fn from_config(config: &ToolMapConfig) -> Result<Self> {
        let mut tools = BTreeMap::new();
        for tool in &config.tools {
            match tools.entry(tool.name.clone()) {
                Entry::Vacant(slot) => {
                    slot.insert(tool.clone());
                }
                Entry::Occupied(_) => {
                    bail!("tool map contains duplicate tool names");
                }
            }
        }
        Ok(Self { tools })
    }

    fn iter(&self) -> impl Iterator<Item = (&String, &ToolRef)> {
        self.tools.iter()
    }
}

fn load_tool_map_config(path: &Path) -> Result<ToolMapConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read MCP tool map {}", path.display()))?;
    if is_json(path, &content) {
        Ok(serde_json::from_str(&content)
            .with_context(|| format!("invalid MCP tool map JSON {}", path.display()))?)
    } else {
        Ok(serde_yaml_bw::from_str(&content)
            .with_context(|| format!("invalid MCP tool map YAML {}", path.display()))?)
    }
}

fn is_json(path: &Path, content: &str) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if matches!(ext, "json") {
            return true;
        }
        if matches!(ext, "yaml" | "yml") {
            return false;
        }
    }
    content
        .chars()
        .find(|c| !c.is_whitespace())
        .is_some_and(|c| c == '{' || c == '[')
}

fn locate_toolmap(workspace_root: &Path, target: &str) -> Result<PathBuf> {
    let initial = PathBuf::from(target);
    if initial.is_absolute() {
        bail!("tool map path must be relative to the workspace root");
    }

    let candidates = [initial.clone(), PathBuf::from("providers").join(&initial)];

    for candidate in candidates {
        let joined = workspace_root.join(&candidate);
        if joined.is_file() {
            return normalize_under_root(workspace_root, &candidate);
        }
        if joined.is_dir() {
            let safe_dir = normalize_under_root(workspace_root, &candidate)?;
            for name in [
                "toolmap.yaml",
                "toolmap.yml",
                "toolmap.json",
                "mcp.yaml",
                "mcp.json",
            ] {
                let file = safe_dir.join(name);
                if file.is_file() {
                    return Ok(file);
                }
            }
        }
    }

    bail!("unable to find MCP tool map at `{target}`")
}

#[derive(Debug, Serialize)]
struct ToolMapReport {
    tool_map_path: String,
    tool_count: usize,
    tools: Vec<ToolHealth>,
    warnings: Vec<String>,
    generator: GeneratorStatus,
}

#[derive(Debug, Serialize)]
struct ToolHealth {
    name: String,
    entry: String,
    component: String,
    resolved_path: String,
    exists: bool,
    size_bytes: Option<u64>,
    timeout_ms: Option<u64>,
    max_retries: u32,
    retry_backoff_ms: u64,
}

impl ToolMapReport {
    fn from_map(config_path: &Path, map: &ToolMap) -> Self {
        let base_dir = config_path
            .parent()
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut warnings = Vec::new();
        let mut tools = Vec::new();

        for (_, tool) in map.iter() {
            let resolved_path = resolve_component_path(&base_dir, &tool.component);
            let (exists, size) = match fs::metadata(&resolved_path) {
                Ok(meta) if meta.is_file() => (true, Some(meta.len())),
                _ => {
                    warnings.push(format!(
                        "tool `{}` component missing at {}",
                        tool.name,
                        resolved_path.display()
                    ));
                    (false, None)
                }
            };

            tools.push(ToolHealth {
                name: tool.name.clone(),
                entry: tool.entry.clone(),
                component: tool.component.clone(),
                resolved_path: resolved_path.display().to_string(),
                exists,
                size_bytes: size,
                timeout_ms: tool.timeout_ms,
                max_retries: tool.max_retries.unwrap_or(0),
                retry_backoff_ms: tool.retry_backoff_ms.unwrap_or(200),
            });
        }

        Self {
            tool_map_path: config_path.display().to_string(),
            tool_count: tools.len(),
            tools,
            warnings,
            generator: GeneratorStatus::detect(),
        }
    }
}

fn resolve_component_path(base_dir: &Path, component: &str) -> PathBuf {
    let raw = PathBuf::from(component);
    if raw.is_absolute() {
        raw
    } else {
        base_dir.join(raw)
    }
}

fn print_report(report: &ToolMapReport) {
    println!("MCP tool map: {}", report.tool_map_path);
    println!("Tools: {}", report.tool_count);
    for tool in &report.tools {
        println!("- {}", tool.name);
        println!("  entry: {}", tool.entry);
        println!(
            "  component: {}{}",
            tool.resolved_path,
            if tool.exists { "" } else { " (missing)" }
        );
        println!(
            "  timeout: {}",
            tool.timeout_ms
                .map(|ms| format!("{ms} ms"))
                .unwrap_or_else(|| "not set".into())
        );
        println!(
            "  retries: {} (backoff {} ms)",
            tool.max_retries, tool.retry_backoff_ms
        );
        if let Some(size) = tool.size_bytes {
            println!("  size: {size} bytes");
        }
    }
    if !report.warnings.is_empty() {
        println!("\nWarnings:");
        for warning in &report.warnings {
            println!("  - {warning}");
        }
    }
    print_generator_status(&report.generator);
}

fn print_generator_status(generator: &GeneratorStatus) {
    let locale = crate::i18n::select_locale(None);
    println!(
        "{}",
        crate::i18n::t(&locale, "cli.command.mcp.doctor.generator.header")
    );
    match (&generator.resolved_path, &generator.version) {
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
        _ => println!(
            "  {}",
            crate::i18n::t(&locale, "cli.command.mcp.doctor.generator.missing")
        ),
    }
    let toolchain_ready = generator.cargo_available && generator.wasm_target_installed;
    if toolchain_ready {
        println!(
            "  {}",
            crate::i18n::t(&locale, "cli.command.mcp.doctor.toolchain.ready")
        );
    }
    if !toolchain_ready && !generator.cargo_available {
        println!(
            "  {}",
            crate::i18n::t(&locale, "cli.command.mcp.doctor.toolchain.cargo_missing")
        );
    }
    if !toolchain_ready && !generator.wasm_target_installed {
        println!(
            "  {}",
            crate::i18n::t(&locale, "cli.command.mcp.doctor.toolchain.wasm_missing")
        );
    }
}

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

    #[test]
    fn doctor_without_toolmap_returns_ok() {
        // No toolmap: doctor must succeed, printing only the generator/toolchain
        // section (best-effort detect never hard-errors).
        assert!(super::doctor(None, true).is_ok());
        assert!(super::doctor(None, false).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ToolMap, ToolMapConfig, ToolMapReport, ToolRef, is_json, load_tool_map_config,
        locate_toolmap,
    };
    use tempfile::tempdir;

    fn sample_tool(name: &str) -> ToolRef {
        ToolRef {
            name: name.to_string(),
            component: "component.wasm".to_string(),
            entry: "run".to_string(),
            timeout_ms: Some(500),
            max_retries: Some(2),
            retry_backoff_ms: Some(100),
        }
    }

    #[test]
    fn json_detection_prefers_extension_and_content() {
        assert!(is_json(std::path::Path::new("toolmap.json"), "tools: []"));
        assert!(!is_json(
            std::path::Path::new("toolmap.yaml"),
            "{\"tools\":[]}"
        ));
        assert!(is_json(
            std::path::Path::new("toolmap"),
            "  {\"tools\": []}"
        ));
    }

    #[test]
    fn duplicate_tool_names_are_rejected() {
        let config = ToolMapConfig {
            tools: vec![sample_tool("demo"), sample_tool("demo")],
        };

        let err = ToolMap::from_config(&config).unwrap_err();
        assert!(err.to_string().contains("duplicate tool names"));
    }

    #[test]
    fn load_tool_map_config_supports_json_and_yaml() {
        let dir = tempdir().unwrap();
        let json_path = dir.path().join("toolmap.json");
        let yaml_path = dir.path().join("toolmap.yaml");
        std::fs::write(
            &json_path,
            r#"{"tools":[{"name":"demo","component":"component.wasm","entry":"run"}]}"#,
        )
        .unwrap();
        std::fs::write(
            &yaml_path,
            "tools:\n  - name: demo\n    component: component.wasm\n    entry: run\n",
        )
        .unwrap();

        assert_eq!(load_tool_map_config(&json_path).unwrap().tools.len(), 1);
        assert_eq!(load_tool_map_config(&yaml_path).unwrap().tools.len(), 1);
    }

    #[test]
    fn locate_toolmap_finds_directory_default_files() {
        let dir = tempdir().unwrap();
        let provider_dir = dir.path().join("demo");
        std::fs::create_dir_all(&provider_dir).unwrap();
        let toolmap = provider_dir.join("toolmap.yaml");
        std::fs::write(&toolmap, "tools: []\n").unwrap();

        let located = locate_toolmap(dir.path(), "demo").unwrap();
        assert_eq!(located, toolmap.canonicalize().unwrap());
    }

    #[test]
    fn report_marks_missing_components_as_warnings() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("toolmap.yaml");
        std::fs::write(&config_path, "tools: []\n").unwrap();
        let map = ToolMap::from_config(&ToolMapConfig {
            tools: vec![sample_tool("demo")],
        })
        .unwrap();

        let report = ToolMapReport::from_map(&config_path, &map);
        assert_eq!(report.tool_count, 1);
        assert_eq!(report.tools[0].name, "demo");
        assert!(!report.tools[0].exists);
        assert_eq!(report.warnings.len(), 1);
    }
}
