use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use greentic_component::describe::{DescribePayload, DescribeVersion};
use greentic_component::lifecycle::Lifecycle;
use greentic_component::manifest::ComponentManifest;
use greentic_component::prepare::PreparedComponent;
use greentic_component::prepare_component;
use greentic_flow::flow_bundle::NodeRef;
use jsonschema::{Draft, Validator};
use semver::{Version, VersionReq};
use serde::Serialize;
use serde_json::Value as JsonValue;

#[derive(Debug, Clone)]
pub struct ResolvedComponent {
    pub name: String,
    pub version: Version,
    pub wasm_path: PathBuf,
    #[allow(dead_code)]
    pub manifest_path: PathBuf,
    pub schema_json: Option<String>,
    pub manifest_json: Option<String>,
    pub capabilities_json: Option<JsonValue>,
    #[allow(dead_code)]
    pub limits_json: Option<JsonValue>,
    pub world: String,
    pub wasm_hash: String,
    #[allow(dead_code)]
    describe: DescribePayload,
}

#[derive(Debug, Clone)]
pub struct ResolvedNode {
    pub node_id: String,
    pub component: Arc<ResolvedComponent>,
    pub pointer: String,
    pub config: JsonValue,
}

#[derive(Debug, Clone)]
pub struct NodeSchemaError {
    pub node_id: String,
    pub component: String,
    pub pointer: String,
    pub message: String,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct ComponentCacheKey {
    name: String,
    version: Version,
}

impl ComponentCacheKey {
    fn new(name: impl Into<String>, version: &Version) -> Self {
        Self {
            name: name.into(),
            version: version.clone(),
        }
    }
}

pub struct ComponentResolver {
    component_dir: Option<PathBuf>,
    cache: HashMap<ComponentCacheKey, Arc<ResolvedComponent>>,
    schema_cache: HashMap<String, Arc<CachedSchema>>,
}

struct CachedSchema(Validator);

impl ComponentResolver {
    pub fn new(component_dir: Option<PathBuf>) -> Self {
        Self {
            component_dir,
            cache: HashMap::new(),
            schema_cache: HashMap::new(),
        }
    }

    pub fn resolve_component(
        &mut self,
        name: &str,
        version_req: &VersionReq,
    ) -> Result<Arc<ResolvedComponent>> {
        self.load_component(name, version_req)
    }

    pub fn resolve_node(&mut self, node: &NodeRef, flow_doc: &JsonValue) -> Result<ResolvedNode> {
        let component_key = &node.component;
        let pointer = format!("/nodes/{}/{}", node.node_id, component_key.name);
        let config = extract_node_payload(flow_doc, &node.node_id, &component_key.name)
            .with_context(|| {
                format!(
                    "failed to extract payload for node `{}` ({})",
                    node.node_id, component_key.name
                )
            })?;

        let version_req = parse_version_req(&component_key.version_req).with_context(|| {
            format!(
                "node `{}` has invalid semver requirement `{}`",
                node.node_id, component_key.version_req
            )
        })?;

        let component = self
            .load_component(&component_key.name, &version_req)
            .with_context(|| {
                format!(
                    "node `{}`: failed to prepare component `{}`",
                    node.node_id, component_key.name
                )
            })?;

        Ok(ResolvedNode {
            node_id: node.node_id.clone(),
            component,
            pointer,
            config,
        })
    }

    pub fn validate_node(&mut self, node: &ResolvedNode) -> Result<Vec<NodeSchemaError>> {
        let Some(schema_json) = &node.component.schema_json else {
            return Ok(Vec::new());
        };

        let validator = self.compile_schema(schema_json)?;
        let mut issues = Vec::new();
        if let Err(error) = validator.0.validate(&node.config) {
            for error in std::iter::once(error).chain(validator.0.iter_errors(&node.config)) {
                let suffix = error.instance_path().to_string();
                let pointer = if suffix.is_empty() || suffix == "/" {
                    node.pointer.clone()
                } else {
                    format!("{}{}", node.pointer, suffix)
                };
                issues.push(NodeSchemaError {
                    node_id: node.node_id.clone(),
                    component: node.component.name.clone(),
                    pointer,
                    message: error.to_string(),
                });
            }
        }
        Ok(issues)
    }

    fn compile_schema(&mut self, schema_json: &str) -> Result<Arc<CachedSchema>> {
        if let Some(existing) = self.schema_cache.get(schema_json) {
            return Ok(existing.clone());
        }

        let schema_value: JsonValue =
            serde_json::from_str(schema_json).context("invalid schema JSON")?;
        let compiled = jsonschema::options()
            .with_draft(Draft::Draft7)
            .build(&schema_value)
            .map_err(|error| anyhow!("failed to compile schema JSON: {error}"))?;
        let entry = Arc::new(CachedSchema(compiled));
        self.schema_cache
            .insert(schema_json.to_string(), entry.clone());
        Ok(entry)
    }

    fn load_component(
        &mut self,
        name: &str,
        version_req: &VersionReq,
    ) -> Result<Arc<ResolvedComponent>> {
        let target = component_target(name, self.component_dir.as_deref());
        let target_display = match &target {
            ComponentTarget::Direct(id) => id.clone(),
            ComponentTarget::Path(path) => path.display().to_string(),
        };

        let prepared = prepare_component(target.as_ref()).with_context(|| {
            format!(
                "resolver looked for `{name}` via `{target_display}` but prepare_component failed"
            )
        })?;

        if !version_req.matches(&prepared.manifest.version) {
            bail!(
                "component `{name}` version `{}` does not satisfy requirement `{version_req}`",
                prepared.manifest.version
            );
        }

        let key = ComponentCacheKey::new(name, &prepared.manifest.version);
        if let Some(existing) = self.cache.get(&key) {
            return Ok(existing.clone());
        }

        let resolved = Arc::new(to_resolved_component(prepared)?);
        self.cache.insert(key, resolved.clone());
        Ok(resolved)
    }
}

enum ComponentTarget {
    Direct(String),
    Path(PathBuf),
}

impl ComponentTarget {
    fn as_ref(&self) -> &str {
        match self {
            ComponentTarget::Direct(id) => id,
            ComponentTarget::Path(path) => path.to_str().expect("component path utf-8"),
        }
    }
}

fn component_target(name: &str, root: Option<&Path>) -> ComponentTarget {
    if let Some(dir) = root {
        let candidate = dir.join(name);
        if candidate.exists() {
            return ComponentTarget::Path(candidate);
        }

        // Fallback: many manifests use fully-qualified ids (e.g., ai.greentic.hello-world) but are
        // checked into components/ under the short name (hello-world).
        if let Some(short) = name
            .rsplit(['.', ':', '/'])
            .next()
            .filter(|s| !s.is_empty())
        {
            let alt = dir.join(short);
            if alt.exists() {
                return ComponentTarget::Path(alt);
            }
        }

        return ComponentTarget::Path(candidate);
    }
    ComponentTarget::Direct(name.to_string())
}

fn parse_version_req(input: &str) -> Result<VersionReq> {
    if input.trim().is_empty() {
        VersionReq::parse("*").map_err(Into::into)
    } else {
        VersionReq::parse(input).map_err(Into::into)
    }
}

fn to_resolved_component(prepared: PreparedComponent) -> Result<ResolvedComponent> {
    let manifest_json = fs::read_to_string(&prepared.manifest_path)
        .with_context(|| format!("failed to read {}", prepared.manifest_path.display()))?;
    let capabilities_json = serde_json::to_value(&prepared.manifest.capabilities)
        .context("failed to serialize capabilities")?;
    let limits_json = prepared
        .manifest
        .limits
        .as_ref()
        .map(|limits| serde_json::to_value(limits).expect("limits serialize"));
    let schema_json = select_schema(&prepared.describe);

    Ok(ResolvedComponent {
        name: prepared.manifest.id.as_str().to_string(),
        version: prepared.manifest.version.clone(),
        wasm_path: prepared.wasm_path.clone(),
        manifest_path: prepared.manifest_path.clone(),
        schema_json,
        manifest_json: Some(manifest_json),
        capabilities_json: Some(capabilities_json),
        limits_json,
        world: prepared.manifest.world.as_str().to_string(),
        wasm_hash: prepared.wasm_hash.clone(),
        describe: prepared.describe,
    })
}

fn extract_node_payload(
    document: &JsonValue,
    node_id: &str,
    component_name: &str,
) -> Result<JsonValue> {
    let nodes = document
        .get("nodes")
        .and_then(|value| value.as_object())
        .context("flow document missing `nodes` object")?;

    let node_entry = nodes
        .get(node_id)
        .and_then(|value| value.as_object())
        .context(format!("flow document missing node `{node_id}`"))?;

    let payload = node_entry.get(component_name).cloned().context(format!(
        "node `{node_id}` missing component payload `{component_name}`"
    ))?;

    Ok(payload)
}

fn select_schema(describe: &DescribePayload) -> Option<String> {
    choose_latest_version(&describe.versions)
        .map(|entry| serde_json::to_string(&entry.schema).expect("describe schema serializes"))
}

fn choose_latest_version(versions: &[DescribeVersion]) -> Option<DescribeVersion> {
    let mut sorted = versions.to_vec();
    sorted.sort_by(|a, b| b.version.cmp(&a.version));
    sorted.into_iter().next()
}

#[allow(dead_code)]
#[derive(Serialize)]
struct PreparedComponentView<'a> {
    manifest: &'a ComponentManifest,
    manifest_path: String,
    wasm_path: String,
    wasm_hash: &'a str,
    world_ok: bool,
    hash_verified: bool,
    describe: &'a DescribePayload,
    lifecycle: &'a Lifecycle,
}

#[allow(dead_code)]
pub fn inspect(target: &str, compact_json: bool) -> Result<()> {
    let prepared = prepare_component(target)
        .with_context(|| format!("failed to prepare component `{target}`"))?;
    let view = PreparedComponentView {
        manifest: &prepared.manifest,
        manifest_path: prepared.manifest_path.display().to_string(),
        wasm_path: prepared.wasm_path.display().to_string(),
        wasm_hash: &prepared.wasm_hash,
        world_ok: prepared.world_ok,
        hash_verified: prepared.hash_verified,
        describe: &prepared.describe,
        lifecycle: &prepared.lifecycle,
    };

    if compact_json {
        println!("{}", serde_json::to_string(&view)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&view)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{component_target, extract_node_payload, parse_version_req};
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn empty_version_requirement_defaults_to_any() {
        let req = parse_version_req("").unwrap();
        assert!(req.matches(&semver::Version::parse("1.2.3").unwrap()));
    }

    #[test]
    fn invalid_version_requirement_is_rejected() {
        assert!(parse_version_req("not-a-semver").is_err());
    }

    #[test]
    fn component_target_falls_back_to_short_name() {
        let dir = tempdir().unwrap();
        let short = dir.path().join("hello-world");
        std::fs::write(&short, "stub").unwrap();

        let target = component_target("ai.greentic.hello-world", Some(dir.path()));
        match target {
            super::ComponentTarget::Path(path) => assert_eq!(path, short),
            _ => panic!("expected path target"),
        }
    }

    #[test]
    fn extract_node_payload_reads_component_payload() {
        let document = json!({
            "nodes": {
                "n1": {
                    "demo.component": { "enabled": true }
                }
            }
        });

        let payload = extract_node_payload(&document, "n1", "demo.component").unwrap();
        assert_eq!(payload["enabled"], true);
    }
}
