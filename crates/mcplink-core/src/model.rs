use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Codex,
    Hermes,
    Claude,
    Opencode,
}

impl Agent {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Hermes => "Hermes",
            Self::Claude => "Claude Code",
            Self::Opencode => "OpenCode",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub agent: Agent,
    pub label: String,
    pub path: PathBuf,
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub plugin_root: Option<PathBuf>,
    #[serde(default)]
    pub discovery_error: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub layout: Option<OpenCodeLayout>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OpenCodeLayout {
    V1,
    V2,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Definition {
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
    pub url: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub enabled: bool,
    pub oauth: bool,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub oauth_scopes: Vec<String>,
    pub oauth_callback_port: Option<u16>,
    pub include_tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub timeout: Option<u64>,
    pub startup_timeout: Option<u64>,
    pub include_specified: bool,
    pub resources_disabled: bool,
    pub prompts_disabled: bool,
    pub original: Value,
    pub original_agent: Option<Agent>,
    pub origin_project: Option<PathBuf>,
    pub origin_layout: Option<OpenCodeLayout>,
}

impl Definition {
    pub fn transport(&self) -> &'static str {
        if self.url.is_some() { "http" } else { "stdio" }
    }

    pub fn summary(&self) -> String {
        if let Some(raw) = &self.url {
            if let Ok(url) = url::Url::parse(raw) {
                return url.origin().ascii_serialization();
            }
            return "HTTP MCP".into();
        }
        self.command.as_deref().unwrap_or("未设置启动命令").into()
    }

    pub fn has_credentials(&self) -> bool {
        self.oauth || !self.headers.is_empty() || !self.env.is_empty()
    }

    pub fn public_value(&self) -> Value {
        let headers: BTreeMap<_, _> = self
            .headers
            .keys()
            .map(|key| (key.clone(), "••••••••"))
            .collect();
        let env: BTreeMap<_, _> = self
            .env
            .iter()
            .map(|(key, value)| {
                let display = if value.starts_with("${") {
                    value.clone()
                } else {
                    "••••••••".into()
                };
                (key.clone(), display)
            })
            .collect();
        let args: Vec<_> = self
            .args
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let previous = index
                    .checked_sub(1)
                    .and_then(|i| self.args.get(i))
                    .map(|v| v.to_lowercase())
                    .unwrap_or_default();
                if ["token", "secret", "password", "api-key"]
                    .iter()
                    .any(|key| {
                        previous.contains(key) || value.to_lowercase().contains(&format!("{key}="))
                    })
                    || value.len() > 24
                        && value
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    "••••••••".to_owned()
                } else {
                    value.clone()
                }
            })
            .collect();
        let url = self.url.as_ref().map(|raw| {
            let mut url = url::Url::parse(raw).ok();
            if let Some(url) = url.as_mut() {
                if url.query().is_some() {
                    url.set_query(Some("••••••••"));
                }
                url.set_fragment(None);
            }
            url.map(|u| u.to_string())
                .unwrap_or_else(|| "••••••••".into())
        });
        json!({"command":self.command,"args":args,"env":env,"cwd":self.cwd,"url":url,"headers":headers,"enabled":self.enabled,"oauth":self.oauth,"clientId":self.client_id,
            "clientSecret":self.client_secret.as_ref().map(|_|"••••••••"),"oauthScopes":self.oauth_scopes,"oauthCallbackPort":self.oauth_callback_port,
            "includeTools":self.include_tools,"excludeTools":self.exclude_tools,"timeout":self.timeout,"startupTimeout":self.startup_timeout,"includeSpecified":self.include_specified,"resourcesDisabled":self.resources_disabled,"promptsDisabled":self.prompts_disabled,"transport":self.transport()})
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub title: String,
    pub transport: String,
    pub endpoint: String,
    pub account_label: String,
    pub auth_kind: String,
    pub definition_ref: String,
    pub revision: u64,
    pub active: bool,
    pub status: String,
    pub checked_at: Option<u64>,
    pub tool_count: Option<usize>,
    pub last_error: Option<String>,
}

impl Connection {
    pub fn public_value(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        if let Some(map) = value.as_object_mut() {
            map.remove("definitionRef");
        }
        value
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub connection_id: String,
    pub source_id: String,
    pub name: String,
    pub mode: String,
    pub fingerprint: String,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Clone)]
pub struct Detected {
    pub source: Source,
    pub name: String,
    pub raw: Value,
    pub error: Option<String>,
    pub definition: Definition,
    pub fingerprint: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: String,
    pub connection_id: String,
    pub source_id: String,
    pub name: String,
    pub mode: String,
    pub action: String,
    pub expected_hash: String,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub previous_binding: Option<Binding>,
    #[serde(default)]
    pub restore_unmanaged: bool,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePlan {
    pub id: String,
    pub changes: Vec<Change>,
    pub created_at: u64,
}
