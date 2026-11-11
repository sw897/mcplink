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
    Zcode,
    Deepseek,
    Openclaw,
    Kimicode,
    Workbuddy,
    Qwenwork,
    Pi,
    Doubaowork,
    #[serde(other)]
    Removed,
}

impl Agent {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Hermes => "Hermes",
            Self::Claude => "Claude Code",
            Self::Opencode => "OpenCode",
            Self::Zcode => "ZCode",
            Self::Deepseek => "DeepSeek Harness",
            Self::Openclaw => "OpenClaw",
            Self::Kimicode => "Kimi Code",
            Self::Workbuddy => "WorkBuddy",
            Self::Qwenwork => "千问办公",
            Self::Pi => "Pi Agent",
            Self::Doubaowork => "豆包工作",
            Self::Removed => "已移除客户端",
        }
    }

    pub fn native_adapter(&self) -> bool {
        matches!(
            self,
            Self::Zcode
                | Self::Deepseek
                | Self::Openclaw
                | Self::Kimicode
                | Self::Workbuddy
                | Self::Qwenwork
                | Self::Pi
        )
    }

    pub fn manual_import(&self) -> bool {
        matches!(self, Self::Doubaowork)
    }

    pub fn supports_import(&self) -> bool {
        matches!(self, Self::Qwenwork | Self::Doubaowork)
    }

    pub fn supported() -> [Self; 12] {
        [
            Self::Codex,
            Self::Hermes,
            Self::Claude,
            Self::Opencode,
            Self::Zcode,
            Self::Deepseek,
            Self::Openclaw,
            Self::Kimicode,
            Self::Workbuddy,
            Self::Qwenwork,
            Self::Pi,
            Self::Doubaowork,
        ]
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
}

#[derive(Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Definition {
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<String>,
    pub url: Option<String>,
    pub http_api: Option<crate::http_api::HttpApi>,
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
}

impl Definition {
    pub fn transport(&self) -> &'static str {
        if self.http_api.is_some() {
            "http-api"
        } else if self.url.is_some() {
            "http"
        } else {
            "stdio"
        }
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

    pub fn changed_origin(&self, previous: &Self) -> bool {
        let (Some(old), Some(new)) = (&previous.url, &self.url) else {
            return false;
        };
        if old == new {
            return false;
        }
        match (url::Url::parse(old), url::Url::parse(new)) {
            (Ok(a), Ok(b)) if a.host().is_some() && b.host().is_some() => a.origin() != b.origin(),
            _ => true,
        }
    }

    pub fn discard_inherited_auth(&mut self, previous: &Self) {
        self.headers.retain(|key, value| {
            value != "••••••••"
                && !previous
                    .headers
                    .iter()
                    .any(|(old_key, old)| old_key.eq_ignore_ascii_case(key) && old == value)
        });
        if self.client_secret.as_deref() == Some("••••••••")
            || self.client_secret == previous.client_secret
        {
            self.client_secret = None;
        }
    }

    pub fn clear_native_auth(&mut self) {
        if let Some(map) = self.original.as_object_mut() {
            for key in [
                "headers",
                "http_headers",
                "env_http_headers",
                "bearer_token_env_var",
                "oauth",
                "auth",
            ] {
                map.remove(key);
            }
        }
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
                let reference = value
                    .strip_prefix("${")
                    .and_then(|value| value.strip_suffix('}'));
                let plain_reference = reference.is_some_and(|name| {
                    let name = name.strip_prefix("env:").unwrap_or(name);
                    !name.is_empty()
                        && name.bytes().enumerate().all(|(index, c)| {
                            c == b'_'
                                || c.is_ascii_alphabetic()
                                || (index > 0 && c.is_ascii_digit())
                        })
                });
                let display = if plain_reference {
                    value.clone()
                } else {
                    "••••••••".into()
                };
                (key.clone(), display)
            })
            .collect();
        // A positional value can be a credential regardless of its spelling or length.
        // References use the original slot and are scoped to the version returned by detail.
        let args: Vec<_> = (0..self.args.len())
            .map(|i| format!("••••••••[{}]", i + 1))
            .collect();
        let url = self.url.as_ref().map(|raw| match url::Url::parse(raw) {
            Ok(url)
                if url.path() == "/"
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && !raw.contains("${") =>
            {
                raw.clone()
            }
            Ok(url) => format!("{}/••••••••", url.origin().ascii_serialization()),
            Err(_) => "••••••••".into(),
        });
        json!({"command":self.command,"args":args,"env":env,"cwd":self.cwd,"url":url,"httpApi":self.http_api,"headers":headers,"enabled":self.enabled,"oauth":self.oauth,"clientId":self.client_id,
            "clientSecret":self.client_secret.as_ref().map(|_|"••••••••"),"oauthScopes":self.oauth_scopes,"oauthCallbackPort":self.oauth_callback_port,
            "includeTools":self.include_tools,"excludeTools":self.exclude_tools,"timeout":self.timeout,"startupTimeout":self.startup_timeout,"includeSpecified":self.include_specified,"resourcesDisabled":self.resources_disabled,"promptsDisabled":self.prompts_disabled,"transport":self.transport()})
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GatewayMember {
    pub connection_id: String,
    pub namespace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_tools: Option<Vec<String>>,
}

impl GatewayMember {
    pub fn tool_allowed(&self, name: &str) -> bool {
        self.include_tools
            .as_ref()
            .is_none_or(|tools| tools.iter().any(|tool| tool == name))
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<GatewayMember>>,
}

impl Connection {
    pub fn edit_version(&self) -> String {
        format!("{}:{}", self.id, self.revision)
    }

    pub fn editable_definition(&self, definition: &Definition) -> Value {
        let mut value = definition.public_value();
        value["editVersion"] = json!(self.edit_version());
        value
    }

    pub fn public_value(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_default();
        if let Some(map) = value.as_object_mut() {
            map.remove("definitionRef");
            if self.members.is_some() {
                map.insert("editVersion".into(), json!(self.edit_version()));
            }
        }
        value
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
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
    pub entry_version: Option<(i64, i64)>,
    #[serde(default)]
    pub restore_unmanaged: bool,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

impl Change {
    pub fn kind(&self) -> &str {
        match self.action.as_str() {
            "upsert" if self.before.is_none() => "add",
            "upsert" => "update",
            "remove" if self.before.is_none() => "cleanup",
            other => other,
        }
    }

    pub fn restore_effect(&self) -> &'static str {
        match (&self.before, &self.after) {
            (None, None) => "none",
            (None, Some(_)) => "remove",
            (Some(_), None) => "add",
            _ => "update",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePlan {
    pub id: String,
    pub changes: Vec<Change>,
    pub created_at: u64,
}
