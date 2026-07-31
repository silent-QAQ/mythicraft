use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Page,
    Tooltip,
    Resource,
    Actions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArcartxDocument {
    pub kind: DocumentKind,
    pub page_id: String,
    pub version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    pub page: PageConfig,
    /// Exact JSON-shaped input after YAML/JSON decoding. UiOpen uses this value so ArcartX
    /// spelling and extension fields remain available to the client-side renderer.
    #[serde(skip)]
    pub raw_model: Value,
    #[serde(default)]
    pub resources: Vec<ResourceRef>,
    #[serde(default)]
    pub actions: Vec<ActionDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PageConfig {
    #[serde(rename = "ui")]
    #[serde(default)]
    pub ui: UiSettings,
    #[serde(rename = "controls")]
    #[serde(default)]
    pub controls: BTreeMap<String, Control>,
    #[serde(rename = "template")]
    #[serde(default)]
    pub template: BTreeMap<String, Control>,
    #[serde(rename = "tasks")]
    #[serde(default)]
    pub tasks: BTreeMap<String, UiTask>,
    #[serde(rename = "root_control")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_control: Option<Control>,
    #[serde(rename = "match")]
    #[serde(default)]
    pub match_rules: Vec<String>,
    #[serde(rename = "hide")]
    #[serde(default)]
    pub hide_rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UiSettings {
    /// The original ArcartX keys are retained as JSON values because ArcartX accepts strings for
    /// several boolean/numeric settings (for example `isHud: "false"`).
    #[serde(flatten)]
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Control {
    pub id: String,
    #[serde(rename = "type")]
    pub control_type: String,
    #[serde(rename = "val")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(rename = "attribute")]
    #[serde(default)]
    pub attributes: BTreeMap<String, Value>,
    #[serde(rename = "effect")]
    #[serde(default)]
    pub effects: BTreeMap<String, Value>,
    #[serde(rename = "action")]
    #[serde(default)]
    pub actions: BTreeMap<String, String>,
    #[serde(default)]
    pub children: BTreeMap<String, Control>,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UiTask {
    #[serde(rename = "type")]
    #[serde(default)]
    pub task_type: Option<String>,
    #[serde(default)]
    pub time: Option<Value>,
    #[serde(default)]
    pub cycle: Option<Value>,
    #[serde(default)]
    pub run: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ResourceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Click,
    Submit,
    Change,
    KeyPress,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDefinition {
    pub id: String,
    pub control_id: String,
    pub action_type: ActionType,
    pub command: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}
