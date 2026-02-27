use std::collections::HashMap;

use serde_yaml_bw::Value as YamlValue;

#[derive(Clone, Debug)]
pub struct ComponentStub {
    pub schema: String,
    pub defaults: YamlValue,
}

#[derive(Clone, Debug)]
pub struct DescribeRegistry {
    stubs: HashMap<String, ComponentStub>,
}

impl DescribeRegistry {
    pub fn new() -> Self {
        let mut stubs = HashMap::new();
        let defaults: YamlValue = serde_yaml_bw::from_str(
            r#"
component: oauth
inputs:
  client_id: null
  client_secret: null
  scopes: []
"#,
        )
        .expect("static oauth defaults");

        stubs.insert(
            "oauth".to_string(),
            ComponentStub {
                schema: r#"{"$id":"https://greenticai.github.io/component-oauth/schemas/v1/oauth.node.schema.json","type":"object"}"#.to_string(),
                defaults,
            },
        );
        Self { stubs }
    }

    pub fn get_schema(&self, name: &str) -> Option<&str> {
        self.stubs.get(name).map(|stub| stub.schema.as_str())
    }

    pub fn get_defaults(&self, name: &str) -> Option<&YamlValue> {
        self.stubs.get(name).map(|stub| &stub.defaults)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ComponentStub)> {
        self.stubs.iter().map(|(name, stub)| (name.as_str(), stub))
    }
}

impl Default for DescribeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

