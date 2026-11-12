//! Bounded upstream discovery shared by managed invocation and aggregate entries.
use crate::{Engine, mcp::McpPool};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

/// Fixed diagnostics only: upstream error text can contain URLs or credentials.
pub(crate) fn discovery_diagnostic(engine: &Engine, id: &str, error: &anyhow::Error) -> Value {
    use crate::{invocation::ToolErrorCode, mcp::ConnectionFailure};
    let connection = engine.store.connection(id);
    let (code, message) = if engine.paused.load(std::sync::atomic::Ordering::SeqCst) {
        ("gateway_paused", "共享后台已暂停，请恢复后重新检查")
    } else if connection.is_err()
        && engine
            .store
            .connections()
            .is_ok_and(|rows| rows.iter().all(|c| c.id != id))
    {
        (
            "connection_missing",
            "托管连接已移除，请明确选择本机连接并重新绑定",
        )
    } else if connection.as_ref().is_ok_and(|c| !c.active) {
        ("connection_inactive", "连接已断开，请重新连接后检查")
    } else if connection.as_ref().is_ok_and(|c| c.status == "needs_auth") {
        (
            "authentication_required",
            "授权不可用，请在账号页设置凭据或重新登录",
        )
    } else if let Some(failure) = error.downcast_ref::<ConnectionFailure>() {
        match failure {
            ConnectionFailure::Authentication => (
                "authentication_required",
                "授权不可用，请在账号页设置凭据或重新登录",
            ),
            ConnectionFailure::Transport => (
                "transport_failed",
                "连接中断或协议不兼容，请检查网络、端点及服务状态",
            ),
            ConnectionFailure::Timeout => (
                "discovery_timeout",
                "清单发现超时，请检查服务状态后重新检查",
            ),
        }
    } else if let Some(failure) = error.downcast_ref::<crate::invocation::ToolFailure>() {
        match failure.code {
            ToolErrorCode::ConnectionChanged => {
                ("connection_changed", "发现期间连接已变化，请重新检查")
            }
            _ => (
                "tool_discovery_failed",
                "清单发现失败，请检查上游清单、分页及支持能力",
            ),
        }
    } else if let Some(failure) = error.downcast_ref::<CatalogFailure>() {
        match failure {
            CatalogFailure::Changed => ("connection_changed", "发现期间连接已变化，请重新检查"),
            CatalogFailure::Invalid => (
                "tool_discovery_failed",
                "清单发现失败，请检查上游清单、分页及支持能力",
            ),
        }
    } else {
        (
            "tool_discovery_failed",
            "无法核验工具，请检查连接配置、授权及上游服务",
        )
    };
    json!({"code":code,"message":message})
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CatalogKind {
    Tools,
    Resources,
    ResourceTemplates,
    Prompts,
}

impl CatalogKind {
    pub fn from_method(method: &str) -> Option<Self> {
        Some(match method {
            "tools/list" => Self::Tools,
            "resources/list" => Self::Resources,
            "resources/templates/list" => Self::ResourceTemplates,
            "prompts/list" => Self::Prompts,
            _ => return None,
        })
    }

    pub fn method(self) -> &'static str {
        match self {
            Self::Tools => "tools/list",
            Self::Resources => "resources/list",
            Self::ResourceTemplates => "resources/templates/list",
            Self::Prompts => "prompts/list",
        }
    }

    pub fn field(self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Resources => "resources",
            Self::ResourceTemplates => "resourceTemplates",
            Self::Prompts => "prompts",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Tools | Self::Prompts => "name",
            Self::Resources => "uri",
            Self::ResourceTemplates => "uriTemplate",
        }
    }
}

#[derive(Debug)]
pub(crate) enum CatalogFailure {
    Changed,
    Invalid,
}

impl std::fmt::Display for CatalogFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Changed => "发现期间服务已更新",
            Self::Invalid => "上游清单重复、分页异常、超出限制或格式无效",
        })
    }
}

impl std::error::Error for CatalogFailure {}

pub(crate) struct CollectedCatalog {
    pub revision: u64,
    pub items: Vec<Value>,
    pub diagnostics: Vec<Value>,
}

pub(crate) async fn collect(
    engine: Arc<Engine>,
    pool: &McpPool,
    id: &str,
    kind: CatalogKind,
) -> Result<CollectedCatalog> {
    let connection = engine.store.connection(id)?;
    let revision = connection.revision;
    let mut items = Vec::new();
    let mut identifiers = HashSet::new();
    let mut cursors = HashSet::new();
    let mut params = json!({});
    let mut diagnostics = Vec::new();
    for _ in 0..100 {
        let page = Box::pin(pool.request_at_revision(
            engine.clone(),
            id,
            json!({"method":kind.method(),"params":params}),
            Some(revision),
        ))
        .await?;
        if engine.store.connection(id)?.revision != revision {
            bail!(CatalogFailure::Changed);
        }
        if page.get("rpcError").is_some() {
            bail!(CatalogFailure::Invalid);
        }
        // Only mcplink aggregate entries produce trusted member diagnostics.
        if connection.members.is_some()
            && let Some(errors) = page
                .pointer("/_meta/mcplink/upstreamErrors")
                .and_then(Value::as_array)
        {
            for error in errors {
                if !diagnostics
                    .iter()
                    .any(|existing: &Value| existing["connectionId"] == error["connectionId"])
                {
                    diagnostics.push(error.clone());
                }
            }
        }
        let rows = page
            .get(kind.field())
            .and_then(Value::as_array)
            .ok_or(CatalogFailure::Invalid)?;
        for item in rows {
            let identifier = item
                .get(kind.key())
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or(CatalogFailure::Invalid)?;
            if !identifiers.insert(identifier.to_owned()) || items.len() >= 10_000 {
                bail!(CatalogFailure::Invalid);
            }
            items.push(item.clone());
        }
        match page.get("nextCursor") {
            None | Some(Value::Null) => {
                return Ok(CollectedCatalog {
                    revision,
                    items,
                    diagnostics,
                });
            }
            Some(Value::String(cursor)) if cursors.insert(cursor.clone()) => {
                params = json!({"cursor":cursor})
            }
            _ => bail!(CatalogFailure::Invalid),
        }
    }
    bail!(CatalogFailure::Invalid)
}
