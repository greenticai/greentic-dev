use serde_json::Value as JsonValue;
use serde_yaml_bw::Value as YamlValue;

pub fn validate_yaml_against_schema(node: &YamlValue, schema_json: &str) -> Result<(), String> {
    let schema: JsonValue = serde_json::from_str(schema_json)
        .map_err(|error| format!("invalid schema JSON: {error}"))?;
    let node_json = serde_json::to_value(node)
        .map_err(|error| format!("failed to convert YAML to JSON: {error}"))?;

    let validator = jsonschema::validator_for(&schema)
        .map_err(|error| format!("schema did not compile: {error}"))?;

    validator
        .validate(&node_json)
        .map_err(|error| error.to_string())
}

pub fn schema_id_from_json(schema_json: &str) -> Option<String> {
    let schema: JsonValue = serde_json::from_str(schema_json).ok()?;
    schema
        .get("$id")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::{schema_id_from_json, validate_yaml_against_schema};
    use serde_yaml_bw::Value as YamlValue;

    #[test]
    fn validates_matching_document() {
        let node: YamlValue = serde_yaml_bw::from_str("name: demo\n").unwrap();
        let schema = r#"{
          "type": "object",
          "required": ["name"],
          "properties": {
            "name": { "type": "string" }
          }
        }"#;

        assert!(validate_yaml_against_schema(&node, schema).is_ok());
    }

    #[test]
    fn rejects_invalid_schema_json() {
        let node: YamlValue = serde_yaml_bw::from_str("name: demo\n").unwrap();
        let err = validate_yaml_against_schema(&node, "{").unwrap_err();
        assert!(err.contains("invalid schema JSON"));
    }

    #[test]
    fn extracts_schema_id() {
        let schema = r#"{"$id":"greentic://schema/demo","type":"object"}"#;
        assert_eq!(
            schema_id_from_json(schema).as_deref(),
            Some("greentic://schema/demo")
        );
        assert!(schema_id_from_json("not json").is_none());
    }
}
