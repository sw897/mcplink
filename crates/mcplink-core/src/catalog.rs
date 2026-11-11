//! Bounded upstream discovery shared by managed invocation and aggregate entries.
use crate::{Engine, mcp::McpPool};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

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
