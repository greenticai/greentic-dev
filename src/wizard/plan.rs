use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WizardFrontend {
    Text,
    Json,
    AdaptiveCard,
}

impl WizardFrontend {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            "adaptive-card" => Some(Self::AdaptiveCard),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WizardPlan {
    pub plan_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    pub metadata: WizardPlanMetadata,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
    pub steps: Vec<WizardStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WizardPlanMetadata {
    pub target: String,
    pub mode: String,
    pub locale: String,
    pub frontend: WizardFrontend,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum WizardStep {
    LaunchPackWizard,
    LaunchBundleWizard,
    RunCommand(RunCommandStep),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunCommandStep {
    pub program: String,
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub destructive: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WizardAnswers {
    pub data: serde_json::Value,
}

impl Default for WizardAnswers {
    fn default() -> Self {
        Self {
            data: serde_json::Value::Object(Default::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RunCommandStep, WizardAnswers, WizardFrontend, WizardStep};

    #[test]
    fn frontend_parser_accepts_supported_values() {
        assert_eq!(WizardFrontend::parse("text"), Some(WizardFrontend::Text));
        assert_eq!(WizardFrontend::parse("json"), Some(WizardFrontend::Json));
        assert_eq!(
            WizardFrontend::parse("adaptive-card"),
            Some(WizardFrontend::AdaptiveCard)
        );
        assert_eq!(WizardFrontend::parse("html"), None);
    }

    #[test]
    fn default_answers_start_with_empty_object() {
        let answers = WizardAnswers::default();
        assert!(answers.data.is_object());
        assert_eq!(answers.data.as_object().unwrap().len(), 0);
    }

    #[test]
    fn run_command_step_omits_false_destructive_flag() {
        let step = WizardStep::RunCommand(RunCommandStep {
            program: "echo".to_string(),
            args: vec!["hello".to_string()],
            destructive: false,
        });

        let value = serde_json::to_value(&step).unwrap();
        assert!(value.get("destructive").is_none());
    }
}
