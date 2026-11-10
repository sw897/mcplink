//! Pi 1.x built-in MCP. Third-party MCP extensions have separate contracts.
use crate::{
    config, json_config,
    model::{Agent, Definition, Source},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

const PRESENTATION: [&str; 3] = ["description", "exposure", "toolExposure"];

pub fn parse(text: &str) -> Result<Value> {
    let value = json_config::parse(text.strip_prefix('\u{feff}').unwrap_or(text), false)?;
    if value
        .get("autoEnableCodemode")
        .is_some_and(|v| !v.is_boolean())
    {
        bail!("Pi Agent autoEnableCodemode 需要布尔值");
    }
    let servers = json_config::table(&value, &["mcpServers".into()])?;
    let mut names = BTreeSet::new();
    for name in servers.as_object().unwrap().keys() {
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            bail!("Pi Agent 服务名称仅允许英文字母、数字、下划线和连字符");
        }
        if !names.insert(config::name_key(name)) {
            bail!("Pi Agent 服务名称存在连字符/下划线冲突，请先在客户端处理");
        }
    }
    Ok(json!({"mcp_servers":servers}))
}

pub fn render(text: &str, name: &str, value: Option<&Value>) -> Result<String> {
    config::safe_name(name)?;
    let document = parse(text)?;
    if value.is_some()
        && document["mcp_servers"]
            .as_object()
            .unwrap()
            .keys()
            .any(|other| other != name && config::name_key(other) == config::name_key(name))
    {
        bail!("Pi Agent 服务名称存在连字符/下划线冲突");
    }
    let rendered = json_config::render(
        text.strip_prefix('\u{feff}').unwrap_or(text),
        false,
        &["mcpServers".into()],
        name,
        value,
    )?;
    Ok(if text.starts_with('\u{feff}') {
        format!("\u{feff}{rendered}")
    } else {
        rendered
    })
}

fn presentation(raw: &Value) -> Result<()> {
    let valid = |v: &Value| {
        matches!(
            v.as_str(),
            Some("codemode" | "codemode-deferred" | "deferred" | "direct" | "hidden")
        )
    };
    if raw.get("description").is_some_and(|v| !v.is_string())
        || raw.get("exposure").is_some_and(|v| !valid(v))
        || raw
            .get("toolExposure")
            .is_some_and(|v| !v.as_object().is_some_and(|m| m.values().all(valid)))
    {
        bail!("Pi Agent 工具暴露或描述字段无效，请在原客户端核对");
    }
    Ok(())
}

fn literal(value: &Value) -> Result<()> {
    match value {
        Value::String(v)
            if v.contains("${")
                || v.contains("{env:")
                || v.starts_with('!')
                || v.starts_with("~/")
                || v.starts_with("~\\") =>
        {
            bail!(
                "Pi Agent 的变量、命令替换或 ~/ 扩展不能等价托管，请保留原生配置或显式建立共享连接"
            );
        }
        Value::Array(items) => {
            for item in items {
                literal(item)?;
            }
        }
        Value::Object(items) => {
            for item in items.values() {
                literal(item)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn normalize(raw: &Value, project: Option<&Path>) -> Result<Definition> {
    let fields = raw.as_object().context("Pi Agent MCP 服务需要映射")?;
    if fields.keys().any(|key| {
        ![
            "type",
            "command",
            "args",
            "env",
            "cwd",
            "url",
            "headers",
            "enabled",
            "description",
            "exposure",
            "toolExposure",
        ]
        .contains(&key.as_str())
    }) {
        bail!(
            "Pi Agent 的认证、超时或其他原生选项尚不能等价托管，请保留原生配置或显式建立共享连接"
        );
    }
    if raw.get("command").is_none() && raw.get("url").is_none() {
        bail!("Pi Agent 项目偏好覆盖不是完整 MCP 连接，请在用户来源管理上游");
    }
    if raw.get("enabled").is_some_and(|v| !v.is_boolean())
        || ["command", "url", "cwd"]
            .iter()
            .any(|key| raw.get(key).is_some_and(|v| !v.is_string()))
    {
        bail!("Pi Agent 启用状态或执行字段类型无效，请在原客户端核对");
    }
    presentation(raw)?;
    if raw["exposure"] == "hidden" || raw.get("toolExposure").is_some() {
        bail!("Pi Agent 原生工具暴露限制请在原客户端维护，不能作为无过滤上游托管");
    }
    if raw
        .get("type")
        .is_some_and(|v| !matches!(v.as_str(), Some("stdio" | "http" | "streamable-http")))
        || (raw["type"] == "stdio" && raw.get("url").is_some())
        || (matches!(raw["type"].as_str(), Some("http" | "streamable-http"))
            && raw.get("command").is_some())
    {
        bail!("Pi Agent 仅支持 STDIO 或 Streamable HTTP");
    }
    let mut portable = raw.clone();
    for key in PRESENTATION {
        portable.as_object_mut().unwrap().remove(key);
    }
    literal(&portable)?;
    let mut definition = config::normalize(&portable, Agent::Codex)?;
    if definition
        .cwd
        .as_deref()
        .is_some_and(|cwd| !Path::new(cwd).is_absolute())
    {
        bail!("Pi Agent 相对工作目录取决于会话位置，请使用绝对目录或保留原生配置");
    }
    definition.original = raw.clone();
    definition.original_agent = Some(Agent::Pi);
    definition.origin_project = project.map(Path::to_path_buf);
    Ok(definition)
}

pub fn native(def: &Definition, source: &Source) -> Result<Value> {
    config::validate(def)?;
    crate::client_import::check_portable(def)?;
    if def.oauth
        || def.client_id.is_some()
        || def.client_secret.is_some()
        || !def.oauth_scopes.is_empty()
        || def.oauth_callback_port.is_some()
        || def.timeout.is_some()
        || def.startup_timeout.is_some()
        || def.include_specified
        || !def.include_tools.is_empty()
        || !def.exclude_tools.is_empty()
        || def.resources_disabled
        || def.prompts_disabled
    {
        bail!("Pi Agent 的原生认证、进度超时或工具限制不能等价直连，请使用共享代理");
    }
    if def
        .cwd
        .as_deref()
        .is_some_and(|cwd| !Path::new(cwd).is_absolute())
    {
        bail!("Pi Agent 直连需要绝对工作目录，请使用共享代理");
    }
    if def.command.is_some()
        && def
            .origin_project
            .as_ref()
            .is_some_and(|p| source.project.as_ref() != Some(p))
    {
        bail!("项目命令不能直接迁移到其他运行目录，请使用共享代理");
    }
    let mut value = if let Some(command) = &def.command {
        json!({"type":"stdio","command":command,"args":def.args,"env":def.env,"enabled":def.enabled})
    } else {
        json!({"type":"http","url":def.url,"headers":def.headers,"enabled":def.enabled})
    };
    if let Some(cwd) = &def.cwd {
        value["cwd"] = json!(cwd);
    }
    literal(&value)?;
    if def.original_agent == Some(Agent::Pi) {
        preserve_presentation(&mut value, Some(&def.original))?;
    }
    Ok(value)
}

/// Keep client-owned visibility settings in both the plan and its fingerprint.
pub fn preserve_presentation(value: &mut Value, before: Option<&Value>) -> Result<()> {
    if let Some(before) = before {
        presentation(before)?;
        for key in PRESENTATION {
            if let Some(setting) = before.get(key) {
                value[key] = setting.clone();
            }
        }
    }
    Ok(())
}
