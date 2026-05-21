use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    #[serde(rename = "type")]
    pub node_type: String,
    pub name: String,
    pub desc: String,

    #[serde(default)]
    pub children: Vec<Node>,

    #[serde(default)]
    pub parameters: Vec<Param>,

    #[serde(default)]
    pub request_template: Option<String>,

    #[serde(default, alias = "requestTemplates")]
    pub request_templates: Vec<String>,

    #[serde(default)]
    pub requests: Vec<HttpRequestBlock>,

    #[serde(default)]
    pub execution_type: Option<String>,

    #[serde(default)]
    pub command_template: Option<String>,

    #[serde(default, alias = "commandTemplates")]
    pub command_templates: Vec<String>,

    #[serde(default)]
    pub commands: Vec<CommandBlock>,

    #[serde(default)]
    pub command_program: Option<String>,

    #[serde(default)]
    pub command_args: Vec<String>,

    #[serde(default, alias = "workingDirectory", alias = "sourceDirectory")]
    pub directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RootJson {
    #[serde(default)]
    pub agent_name: Option<String>,
    pub architecture: Node,

    #[serde(default)]
    pub components: Option<Components>,

    #[serde(default)]
    pub start: Option<StartBlock>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct StartBlock {
    #[serde(default, alias = "requestTemplates")]
    pub request_templates: Vec<String>,

    #[serde(default)]
    pub requests: Vec<HttpRequestBlock>,

    #[serde(default, alias = "commandTemplates")]
    pub command_templates: Vec<String>,

    #[serde(default)]
    pub commands: Vec<CommandBlock>,

    #[serde(default, alias = "workingDirectory", alias = "sourceDirectory")]
    pub directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum HttpRequestBlock {
    Template(String),
    Config {
        template: String,

        #[serde(default, alias = "workingDirectory", alias = "sourceDirectory")]
        directory: Option<String>,
    },
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum CommandBlock {
    Template(String),
    Config {
        #[serde(default)]
        template: Option<String>,

        #[serde(default)]
        program: Option<String>,

        #[serde(default)]
        args: Vec<String>,

        #[serde(default)]
        elevated: bool,

        #[serde(default, alias = "workingDirectory", alias = "sourceDirectory")]
        directory: Option<String>,
    },
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Components {
    pub function_mapper: Option<ModelConfig>,
    pub entity_recognizer: Option<ModelConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub model_url: Option<String>,
    pub tokenizer_url: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Param {
    pub name: String,
    pub required: bool,

    #[serde(rename = "type", default)]
    pub kind: ParamKind,

    #[serde(default)]
    pub tag: Option<String>,

    #[serde(default)]
    pub extractor: Option<String>,

    #[serde(default)]
    pub pattern: Option<String>,

    #[serde(default, rename = "acceptLabels")]
    pub accept_labels: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ParamKind {
    #[default]
    Text,
    File,
    Files,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FileInput {
    pub path: String,

    #[serde(default)]
    pub filename: Option<String>,

    #[serde(default)]
    pub content_type: Option<String>,

    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", content = "data", rename_all = "lowercase")]
pub enum OutputPayload {
    String(String),
    Binary(String),
    File(FileInput),
    Json(serde_json::Value),
    Files(Vec<FileInput>),
    Empty,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum RequestParamValue {
    Text(String),
    TextList(Vec<String>),
    File(FileInput),
    FileList(Vec<FileInput>),
}

#[derive(Debug, Clone)]
pub enum ParamValue {
    Text(String),
    File(FileInput),
    Files(Vec<FileInput>),
}

pub type ParamValues = HashMap<String, ParamValue>;

impl Param {
    pub fn is_text(&self) -> bool {
        self.kind == ParamKind::Text
    }
}

impl ParamValue {
    pub fn is_missing(&self) -> bool {
        match self {
            Self::Text(value) => value.trim().is_empty(),
            Self::File(file) => file.path.trim().is_empty(),
            Self::Files(files) => {
                files.is_empty() || files.iter().any(|file| file.path.trim().is_empty())
            }
        }
    }
}
