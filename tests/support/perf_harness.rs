use std::time::{Duration, Instant};

use greentic_dev::dev_runner::schema::{
    compile_schema, validate_yaml_against_compiled_schema, validate_yaml_against_schema,
};
use greentic_dev::dev_runner::{ComponentSchema, FlowValidator, StaticComponentDescriber};
use greentic_dev::registry::DescribeRegistry;
use serde_yaml_bw::Value as YamlValue;

const HOT_SCHEMA: &str = r#"{
  "$id": "greentic://bench/hot/v1",
  "type": "object",
  "required": ["component", "inputs"],
  "properties": {
    "component": { "const": "hot" },
    "inputs": {
      "type": "object",
      "required": ["name", "count", "enabled"],
      "properties": {
        "name": { "type": "string" },
        "count": { "type": "integer", "minimum": 0 },
        "enabled": { "type": "boolean" }
      },
      "additionalProperties": false
    }
  },
  "additionalProperties": false
}"#;

pub fn build_validator() -> FlowValidator<StaticComponentDescriber> {
    let mut describer = StaticComponentDescriber::new();
    describer.register_schema(
        "hot",
        ComponentSchema {
            node_schema: Some(HOT_SCHEMA.to_string()),
        },
    );

    FlowValidator::new(describer, DescribeRegistry::default())
}

pub fn build_document(node_count: usize) -> YamlValue {
    let mut yaml = String::from("nodes:\n");

    for index in 0..node_count {
        yaml.push_str("  - component: hot\n");
        yaml.push_str("    inputs:\n");
        yaml.push_str(&format!("      name: node-{index}\n"));
        yaml.push_str(&format!("      count: {}\n", index % 100));
        yaml.push_str("      enabled: true\n");
    }

    serde_yaml_bw::from_str(&yaml).expect("generated benchmark YAML is valid")
}

#[allow(dead_code)]
pub fn run_validation_loops(node_count: usize, iterations: usize) -> Duration {
    let validator = build_validator();
    let document = build_document(node_count);

    let start = Instant::now();

    for _ in 0..iterations {
        validator
            .validate_document(&document)
            .expect("benchmark workload should remain valid");
    }

    start.elapsed()
}

#[allow(dead_code)]
pub fn run_schema_validation_loops_uncached(node_count: usize, iterations: usize) -> Duration {
    let document = build_document(node_count);
    let nodes = document
        .as_mapping()
        .and_then(|mapping| mapping.get("nodes"))
        .and_then(|value| value.as_sequence())
        .expect("generated document has nodes sequence");
    let start = Instant::now();

    for _ in 0..iterations {
        for node in nodes {
            validate_yaml_against_schema(node, HOT_SCHEMA)
                .expect("schema benchmark input should remain valid");
        }
    }

    start.elapsed()
}

#[allow(dead_code)]
pub fn run_schema_validation_loops_cached(node_count: usize, iterations: usize) -> Duration {
    let document = build_document(node_count);
    let nodes = document
        .as_mapping()
        .and_then(|mapping| mapping.get("nodes"))
        .and_then(|value| value.as_sequence())
        .expect("generated document has nodes sequence");
    let compiled = compile_schema(HOT_SCHEMA).expect("hot schema compiles");
    let start = Instant::now();

    for _ in 0..iterations {
        for node in nodes {
            validate_yaml_against_compiled_schema(node, &compiled.validator)
                .expect("schema benchmark input should remain valid");
        }
    }

    start.elapsed()
}
