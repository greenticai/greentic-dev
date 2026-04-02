use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_yaml_bw::Value as YamlValue;

use super::registry::DescribeRegistry;
use super::schema::{CompiledSchema, compile_schema, validate_yaml_against_compiled_schema};
use crate::path_safety::normalize_under_root;

#[derive(Clone, Debug, Default)]
pub struct ComponentSchema {
    pub node_schema: Option<String>,
}

pub trait ComponentDescriber {
    fn describe(&self, component: &str) -> Result<ComponentSchema, String>;
}

#[derive(Debug, Clone)]
pub struct StaticComponentDescriber {
    schemas: HashMap<String, ComponentSchema>,
    fallback: ComponentSchema,
}

impl StaticComponentDescriber {
    pub fn new() -> Self {
        Self {
            schemas: HashMap::new(),
            fallback: ComponentSchema::default(),
        }
    }

    pub fn with_fallback(mut self, fallback_schema: ComponentSchema) -> Self {
        self.fallback = fallback_schema;
        self
    }

    pub fn register_schema<S: Into<String>>(
        &mut self,
        component: S,
        schema: ComponentSchema,
    ) -> &mut Self {
        self.schemas.insert(component.into(), schema);
        self
    }
}

impl ComponentDescriber for StaticComponentDescriber {
    fn describe(&self, component: &str) -> Result<ComponentSchema, String> {
        if let Some(schema) = self.schemas.get(component) {
            Ok(schema.clone())
        } else {
            Ok(self.fallback.clone())
        }
    }
}

impl Default for StaticComponentDescriber {
    fn default() -> Self {
        Self::new()
    }
}

pub struct FlowValidator<D> {
    describer: D,
    registry: DescribeRegistry,
}

#[derive(Clone)]
struct ComponentValidationPlan {
    schema_json: Option<String>,
    schema_id: Option<String>,
    defaults: Option<YamlValue>,
    compiled_schema: Option<Arc<CompiledSchema>>,
}

#[derive(Clone, Debug)]
pub struct ValidatedNode {
    pub component: String,
    pub node_config: YamlValue,
    pub schema_json: Option<String>,
    pub schema_id: Option<String>,
    pub defaults: Option<YamlValue>,
}

impl<D> FlowValidator<D>
where
    D: ComponentDescriber,
{
    pub fn new(describer: D, registry: DescribeRegistry) -> Self {
        Self {
            describer,
            registry,
        }
    }

    pub fn validate_file<P>(&self, path: P) -> Result<Vec<ValidatedNode>, FlowValidationError>
    where
        P: AsRef<Path>,
    {
        let path_ref = path.as_ref();
        let root = std::env::current_dir()
            .map_err(|error| FlowValidationError::Io {
                path: path_ref.to_path_buf(),
                error,
            })?
            .canonicalize()
            .map_err(|error| FlowValidationError::Io {
                path: path_ref.to_path_buf(),
                error,
            })?;
        let safe =
            normalize_under_root(&root, path_ref).map_err(|error| FlowValidationError::Io {
                path: path_ref.to_path_buf(),
                error: std::io::Error::other(error.to_string()),
            })?;
        let source = fs::read_to_string(&safe)
            .map_err(|error| FlowValidationError::Io { path: safe, error })?;
        self.validate_str(&source)
    }

    pub fn validate_str(
        &self,
        yaml_source: &str,
    ) -> Result<Vec<ValidatedNode>, FlowValidationError> {
        let document: YamlValue = serde_yaml_bw::from_str(yaml_source).map_err(|error| {
            FlowValidationError::YamlParse {
                error: error.to_string(),
            }
        })?;
        self.validate_document(&document)
    }

    pub fn validate_document(
        &self,
        document: &YamlValue,
    ) -> Result<Vec<ValidatedNode>, FlowValidationError> {
        let nodes = match nodes_from_document(document) {
            Some(nodes) => nodes,
            None => {
                return Err(FlowValidationError::MissingNodes);
            }
        };

        let mut validated_nodes = Vec::with_capacity(nodes.len());
        let mut schema_cache: HashMap<String, Arc<CompiledSchema>> = HashMap::new();
        let mut component_plan_cache: HashMap<String, ComponentValidationPlan> = HashMap::new();

        for (index, node) in nodes.iter().enumerate() {
            let node_mapping = match node.as_mapping() {
                Some(mapping) => mapping,
                None => {
                    return Err(FlowValidationError::NodeNotMapping { index });
                }
            };

            let component = component_name(node_mapping)
                .ok_or(FlowValidationError::MissingComponent { index })?;

            if !component_plan_cache.contains_key(component) {
                let schema = self.describer.describe(component).map_err(|error| {
                    FlowValidationError::DescribeFailed {
                        component: component.to_owned(),
                        error,
                    }
                })?;
                let schema_json = self
                    .registry
                    .get_schema(component)
                    .map(|schema| schema.to_owned())
                    .or_else(|| schema.node_schema.clone());
                let compiled_schema = if let Some(schema_json) = schema_json.as_deref() {
                    if let Some(compiled) = schema_cache.get(schema_json) {
                        Some(Arc::clone(compiled))
                    } else {
                        let compiled = compile_schema(schema_json).map_err(|message| {
                            FlowValidationError::SchemaValidation {
                                component: component.to_owned(),
                                index,
                                message,
                            }
                        })?;
                        let compiled = Arc::new(compiled);
                        schema_cache.insert(schema_json.to_owned(), Arc::clone(&compiled));
                        Some(compiled)
                    }
                } else {
                    None
                };
                let schema_id = compiled_schema
                    .as_ref()
                    .and_then(|compiled| compiled.schema_id.clone());
                let defaults = self.registry.get_defaults(component).cloned();
                component_plan_cache.insert(
                    component.to_owned(),
                    ComponentValidationPlan {
                        schema_json,
                        schema_id,
                        defaults,
                        compiled_schema,
                    },
                );
            }

            let plan = component_plan_cache
                .get(component)
                .expect("component plan cache must contain computed entry");
            if let Some(compiled) = plan.compiled_schema.as_deref() {
                validate_yaml_against_compiled_schema(node, &compiled.validator).map_err(
                    |message| FlowValidationError::SchemaValidation {
                        component: component.to_owned(),
                        index,
                        message,
                    },
                )?;
            }

            validated_nodes.push(ValidatedNode {
                component: component.to_owned(),
                node_config: node.clone(),
                schema_json: plan.schema_json.clone(),
                schema_id: plan.schema_id.clone(),
                defaults: plan.defaults.clone(),
            });
        }

        Ok(validated_nodes)
    }
}

fn nodes_from_document(document: &YamlValue) -> Option<&Vec<YamlValue>> {
    if let Some(sequence) = document.as_sequence() {
        return Some(&**sequence);
    }

    let mapping = document.as_mapping()?;
    mapping
        .get("nodes")
        .and_then(|value| value.as_sequence().map(|sequence| &**sequence))
}

fn component_name(mapping: &serde_yaml_bw::Mapping) -> Option<&str> {
    mapping
        .get("component")
        .and_then(|value| value.as_str())
        .or_else(|| mapping.get("type").and_then(|value| value.as_str()))
}

#[derive(Debug)]
pub enum FlowValidationError {
    Io {
        path: PathBuf,
        error: std::io::Error,
    },
    YamlParse {
        error: String,
    },
    MissingNodes,
    NodeNotMapping {
        index: usize,
    },
    MissingComponent {
        index: usize,
    },
    DescribeFailed {
        component: String,
        error: String,
    },
    SchemaValidation {
        component: String,
        index: usize,
        message: String,
    },
}
