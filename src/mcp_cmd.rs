use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, btree_map::Entry};

use crate::path_safety::normalize_under_root;

pub fn doctor(target: &str, json: bool) -> Result<()> {
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
}
