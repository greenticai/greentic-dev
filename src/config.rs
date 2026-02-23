use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize, Clone)]
pub struct GreenticConfig {
    #[serde(default)]
    pub tools: ToolsSection,
    #[serde(default)]
    pub defaults: DefaultsSection,
    #[serde(default)]
    pub distributor: DistributorSection,
    /// Backward-compatible root-level [profiles.*] table used for distributor.
    #[serde(default, rename = "profiles")]
    pub legacy_distributor_profiles: HashMap<String, DistributorProfileConfig>,
}

impl GreenticConfig {
    pub fn distributor_profiles(&self) -> HashMap<String, DistributorProfileConfig> {
        let mut merged = self.distributor.merged_profiles();
        if merged.is_empty() && !self.legacy_distributor_profiles.is_empty() {
            merged.extend(self.legacy_distributor_profiles.clone());
        }
        merged
    }
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct ToolsSection {
    #[serde(rename = "greentic-component", default)]
    pub greentic_component: ToolEntry,
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct ToolEntry {
    pub path: Option<PathBuf>,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize, Clone)]
pub struct DefaultsSection {
    #[serde(default)]
    pub component: ComponentDefaults,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize, Clone)]
pub struct ComponentDefaults {
    pub org: Option<String>,
    pub template: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct DistributorSection {
    /// Configures the default distributor profile by name or inline struct.
    #[serde(default)]
    pub default_profile: Option<DefaultProfileSelection>,
    /// Profiles nested under [distributor.profiles.*].
    #[serde(default)]
    pub profiles: HashMap<String, DistributorProfileConfig>,
    /// Backward-compatible: `[distributor.<name>]` tables.
    #[serde(default, flatten)]
    legacy_profiles: HashMap<String, DistributorProfileConfig>,
}

impl DistributorSection {
    pub fn merged_profiles(&self) -> HashMap<String, DistributorProfileConfig> {
        let mut merged = self.profiles.clone();
        for (name, cfg) in self.legacy_profiles.iter() {
            merged.entry(name.clone()).or_insert_with(|| cfg.clone());
        }
        merged
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DefaultProfileSelection {
    Name(String),
    Inline(DistributorProfileConfig),
}

#[derive(Debug, Clone, Deserialize)]
pub struct DistributorProfileConfig {
    /// Optional profile name when provided inline.
    #[serde(default)]
    pub name: Option<String>,
    /// Base URL for the distributor (preferred field; falls back to `url` if set).
    #[serde(default)]
    pub base_url: Option<String>,
    /// Deprecated alias for base_url.
    #[serde(default)]
    pub url: Option<String>,
    /// API token; allow env:VAR indirection.
    #[serde(default)]
    pub token: Option<String>,
    /// Tenant identifier for distributor requests.
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Environment identifier for distributor requests.
    #[serde(default)]
    pub environment_id: Option<String>,
    /// Additional headers (optional).
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct LoadedGreenticConfig {
    pub config: GreenticConfig,
    pub loaded_from: Option<PathBuf>,
    pub attempted_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ConfigResolution {
    pub selected: Option<PathBuf>,
    pub attempted: Vec<PathBuf>,
    pub forced: Option<ConfigSource>,
}

#[derive(Debug, Clone)]
pub enum ConfigSource {
    Arg,
    Env(&'static str),
}

pub fn load() -> Result<GreenticConfig> {
    load_with_meta(None).map(|loaded| loaded.config)
}

pub fn load_from(path_override: Option<&str>) -> Result<GreenticConfig> {
    load_with_meta(path_override).map(|loaded| loaded.config)
}

pub fn load_with_meta(path_override: Option<&str>) -> Result<LoadedGreenticConfig> {
    let resolution = resolve_config_path(path_override);
    let forced_source = resolution.forced.clone();
    let attempted_paths = resolution.attempted.clone();

    let Some(selected) = resolution.selected else {
        return Ok(LoadedGreenticConfig {
            config: GreenticConfig::default(),
            loaded_from: None,
            attempted_paths,
        });
    };

    if !selected.exists() {
        let reason = match forced_source {
            Some(ConfigSource::Arg) => "explicit config override",
            Some(ConfigSource::Env(var)) => var,
            None => "config discovery",
        };
        bail!(
            "config file {} set via {} does not exist (searched: {})",
            selected.display(),
            reason,
            format_attempted(&resolution.attempted)
        );
    }

    let raw = fs::read_to_string(&selected)
        .with_context(|| format!("failed to read config at {}", selected.display()))?;
    let config: GreenticConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config at {}", selected.display()))?;

    Ok(LoadedGreenticConfig {
        config,
        loaded_from: Some(selected),
        attempted_paths,
    })
}

fn format_attempted(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "(none)".to_string();
    }
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn resolve_config_path(path_override: Option<&str>) -> ConfigResolution {
    let mut attempted = Vec::new();

    if let Some(raw) = path_override {
        let path = PathBuf::from(raw);
        attempted.push(path.clone());
        return ConfigResolution {
            selected: Some(path),
            attempted,
            forced: Some(ConfigSource::Arg),
        };
    }

    for (var, source) in [
        (
            "GREENTIC_DEV_CONFIG_FILE",
            ConfigSource::Env("GREENTIC_DEV_CONFIG_FILE"),
        ),
        (
            "GREENTIC_CONFIG_FILE",
            ConfigSource::Env("GREENTIC_CONFIG_FILE"),
        ),
        ("GREENTIC_CONFIG", ConfigSource::Env("GREENTIC_CONFIG")),
    ] {
        if let Ok(raw) = std::env::var(var)
            && !raw.is_empty()
        {
            let path = PathBuf::from(raw);
            attempted.push(path.clone());
            return ConfigResolution {
                selected: Some(path),
                attempted,
                forced: Some(source),
            };
        }
    }

    let mut candidates = Vec::new();
    let xdg_config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(dirs::config_dir);
    if let Some(mut dir) = xdg_config {
        dir.push("greentic-dev");
        dir.push("config.toml");
        push_unique(&mut candidates, dir);
    }
    if let Some(mut home) = dirs::home_dir() {
        let mut legacy = home.clone();
        legacy.push(".config");
        legacy.push("greentic-dev");
        legacy.push("config.toml");
        push_unique(&mut candidates, legacy);

        home.push(".greentic");
        home.push("config.toml");
        push_unique(&mut candidates, home);
    }

    let selected = candidates.iter().find(|path| path.exists()).cloned();
    attempted.extend(candidates);

    ConfigResolution {
        selected,
        attempted,
        forced: None,
    }
}

pub fn config_path() -> Option<PathBuf> {
    resolve_config_path(None).attempted.into_iter().next()
}

fn push_unique(vec: &mut Vec<PathBuf>, path: PathBuf) {
    if !vec.iter().any(|existing| existing == &path) {
        vec.push(path);
    }
}
