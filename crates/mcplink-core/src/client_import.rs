//! Public connector forms only: no guessed paths or private client databases.
use crate::{
    config,
    model::{Agent, Definition},
};
use anyhow::{Result, bail};
use serde_json::{Value, json};

pub fn check_portable(def: &Definition) -> Result<()> {
    // The original Codex/Hermes document may carry options outside the normalized model.
    if matches!(def.original_agent, Some(Agent::Codex | Agent::Hermes))
        && let Some(raw) = def.original.as_object()
        && (raw.keys().any(|key| {
            !config::FIELDS.contains(&key.as_str())
                || matches!(
                    key.as_str(),
                    "environment" | "disabled" | "codemode" | "protocol" | "pool"
                )
        }) || raw
            .get("type")
            .is_some_and(|v| !matches!(v.as_str(), Some("stdio" | "http" | "streamable-http")))
            || raw.get("tools").is_some_and(|v| {
                !v.as_object().is_some_and(|tools| {
                    tools.keys().all(|key| {
                        matches!(
                            key.as_str(),
                            "include" | "exclude" | "resources" | "prompts"
                        )
                    })
                })
            }))
    {
        bail!("来源包含不能等价导出的客户端专属选项，请在原客户端维护，或显式建立共享连接")
    }
    Ok(())
}

pub fn entry(def: &Definition, agent: &Agent) -> Result<Value> {
    if def.http_api.is_some() {
        bail!("普通 HTTP API 需要 mcplink Adapter，请使用共享模式，不能导出为原生 MCP 地址");
    }
    if !agent.supports_import() {
        bail!("此客户端使用原生配置预览，请选择千问办公或豆包工作导入")
    }
    crate::adapter_policy::validate(def, agent, true)?;
    let mut entry = if let Some(command) = &def.command {
        json!({"type":"stdio","command":command,"args":def.args,"env":def.env})
    } else {
        json!({"type":if *agent == Agent::Qwenwork { "streamable-http" } else { "http" },
            "url":def.url,"headers":def.headers})
    };
    if entry.to_string().contains("${") || entry.to_string().contains("{env:") {
        bail!("托管变量不能直接导入此客户端，请使用共享模式")
    }
    if let Some(map) = entry.as_object_mut() {
        map.retain(|_, value| !value.as_object().is_some_and(|v| v.is_empty()));
    }
    Ok(entry)
}
