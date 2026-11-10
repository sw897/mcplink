//! Discover and invoke exact managed tools without a second connection or credential store.
use crate::{Engine, model::digest};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode {
    InvalidToolArguments,
    ToolNotFound,
    InvalidToolSchema,
    ToolSchemaChanged,
    ConnectionChanged,
    ToolDiscoveryFailed,
}

impl ToolErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidToolArguments => "invalid_tool_arguments",
            Self::ToolNotFound => "tool_not_found",
            Self::InvalidToolSchema => "invalid_tool_schema",
            Self::ToolSchemaChanged => "tool_schema_changed",
            Self::ConnectionChanged => "connection_changed",
            Self::ToolDiscoveryFailed => "tool_discovery_failed",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolFailure {
    pub code: ToolErrorCode,
    #[serde(default)]
    pub details: Value,
}

impl ToolFailure {
    fn new(code: ToolErrorCode) -> Self {
        Self {
            code,
            details: Value::Null,
        }
    }
}

impl std::fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.code {
            ToolErrorCode::InvalidToolArguments => "参数不符合工具的输入 Schema；工具尚未派发",
            ToolErrorCode::ToolNotFound => "工具不存在或未在此连接中启用",
            ToolErrorCode::InvalidToolSchema => {
                "工具 Schema 无效或含不可用的外部引用；工具尚未派发"
            }
            ToolErrorCode::ToolSchemaChanged => {
                "工具 Schema 已变化，请重新发现或生成入口；工具尚未派发"
            }
            ToolErrorCode::ConnectionChanged => {
                "发现期间连接已变化，请重新核对后调用；工具尚未派发"
            }
            ToolErrorCode::ToolDiscoveryFailed => "工具发现失败：重复名称、分页异常或上游响应无效",
        })
    }
}

impl std::error::Error for ToolFailure {}

pub async fn discover(engine: Arc<Engine>, id: &str) -> Result<(u64, Vec<Value>)> {
    discover_with_pool(engine.clone(), id, &engine.pool).await
}

pub(crate) async fn discover_with_pool(
    engine: Arc<Engine>,
    id: &str,
    pool: &crate::mcp::McpPool,
) -> Result<(u64, Vec<Value>)> {
    let (revision, tools, _) = discover_report(engine, id, pool).await?;
    Ok((revision, tools))
}

pub(crate) async fn discover_report(
    engine: Arc<Engine>,
    id: &str,
    pool: &crate::mcp::McpPool,
) -> Result<(u64, Vec<Value>, Vec<Value>)> {
    let connection = engine.store.connection(id)?;
    let revision = connection.revision;
    let mut tools = Vec::new();
    let mut names = HashSet::new();
    let mut cursors = HashSet::new();
    let mut params = json!({});
    let mut errors = Vec::new();
    for _ in 0..100 {
        let page = pool
            .request(
                engine.clone(),
                id,
                json!({"method":"tools/list","params":params}),
            )
            .await?;
        if engine.store.connection(id)?.revision != revision {
            bail!(ToolFailure::new(ToolErrorCode::ConnectionChanged));
        }
        if connection.members.is_some()
            && let Some(upstreams) = page
                .pointer("/_meta/mcplink/upstreamErrors")
                .and_then(Value::as_array)
        {
            for upstream in upstreams {
                if !errors
                    .iter()
                    .any(|error: &Value| error["connectionId"] == upstream["connectionId"])
                {
                    errors.push(upstream.clone());
                }
            }
        }
        let rows = page
            .get("tools")
            .and_then(Value::as_array)
            .context(ToolFailure::new(ToolErrorCode::ToolDiscoveryFailed))?;
        for tool in rows {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .context(ToolFailure::new(ToolErrorCode::ToolDiscoveryFailed))?;
            if name.is_empty() || !names.insert(name.to_owned()) || tools.len() >= 10_000 {
                bail!(ToolFailure::new(ToolErrorCode::ToolDiscoveryFailed));
            }
            tools.push(tool.clone());
        }
        match page.get("nextCursor") {
            None | Some(Value::Null) => return Ok((revision, tools, errors)),
            Some(Value::String(cursor)) if cursors.insert(cursor.clone()) => {
                params = json!({"cursor":cursor});
            }
            _ => bail!(ToolFailure::new(ToolErrorCode::ToolDiscoveryFailed)),
        }
    }
    bail!(ToolFailure::new(ToolErrorCode::ToolDiscoveryFailed))
}

pub fn schema_hash(tool: &Value) -> Result<String> {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), canonical(&object[key])))
                        .collect(),
                )
            }
            Value::Array(array) => Value::Array(array.iter().map(canonical).collect()),
            value => value.clone(),
        }
    }
    Ok(digest(&serde_json::to_vec(&canonical(
        &json!({"name":tool["name"],"inputSchema":tool["inputSchema"]}),
    ))?))
}

pub async fn describe(engine: Arc<Engine>, id: &str, name: &str) -> Result<Value> {
    let (revision, tools) = discover(engine, id).await?;
    let tool = tools
        .into_iter()
        .find(|tool| tool["name"] == name)
        .context(ToolFailure::new(ToolErrorCode::ToolNotFound))?;
    Ok(
        json!({"connectionId":id,"connectionRevision":revision,"schemaHash":schema_hash(&tool)?,"tool":tool,
        "validation":{"drafts":["draft-04","draft-06","draft-07","2019-09","2020-12"],
        "defaultDraft":"2020-12","externalReferences":false,"formatAssertions":false,"customVocabularies":false}}),
    )
}

pub async fn call(
    engine: Arc<Engine>,
    id: &str,
    name: &str,
    arguments: Value,
    expected_hash: Option<&str>,
) -> Result<Value> {
    if !arguments.is_object() {
        bail!(ToolFailure::new(ToolErrorCode::InvalidToolArguments));
    }
    let descriptor = describe(engine.clone(), id, name).await?;
    let hash = descriptor["schemaHash"]
        .as_str()
        .context("工具 Schema 摘要缺失")?;
    if expected_hash.is_some_and(|expected| expected != hash) {
        bail!(ToolFailure::new(ToolErrorCode::ToolSchemaChanged));
    }
    let schema = descriptor["tool"]["inputSchema"].clone();
    let input = arguments.clone();
    tokio::task::spawn_blocking(move || validate(&schema, &input))
        .await
        .map_err(|_| ToolFailure::new(ToolErrorCode::InvalidToolSchema))??;
    let revision = descriptor["connectionRevision"]
        .as_u64()
        .context("连接修订缺失")?;
    if engine.store.connection(id)?.revision != revision {
        bail!(ToolFailure::new(ToolErrorCode::ConnectionChanged));
    }
    let mut params = json!({"name":name,"arguments":arguments});
    if engine.store.connection(id)?.members.is_some()
        && let Some(upstream_revision) = descriptor.pointer("/tool/_meta/mcplink/upstreamRevision")
    {
        params["_meta"] = json!({"mcplink":{"expectedUpstreamRevision":upstream_revision,"expectedSchemaHash":hash}});
    }
    let output = engine
        .pool
        .request_at_revision(
            engine.clone(),
            id,
            json!({"method":"tools/call","params":params}),
            Some(revision),
        )
        .await?;
    Ok(json!({"connectionId":id,"tool":name,"schemaHash":hash,"output":output}))
}

pub(crate) fn validator(schema: &Value) -> Result<jsonschema::Validator> {
    if !schema.is_object() || serde_json::to_vec(schema)?.len() > 1024 * 1024 {
        bail!(ToolFailure::new(ToolErrorCode::InvalidToolSchema));
    }
    let mut options = jsonschema::options()
        .should_validate_formats(false)
        .offline();
    if schema.get("$schema").is_none() {
        options = options.with_draft(jsonschema::Draft::Draft202012);
    }
    options
        .build(schema)
        .map_err(|_| ToolFailure::new(ToolErrorCode::InvalidToolSchema).into())
}

fn validate(schema: &Value, input: &Value) -> Result<()> {
    let validator = validator(schema)?;
    let issues: Vec<_> = validator
        .iter_errors(input)
        .take(16)
        .map(|error| json!({"schemaPath":error.schema_path().to_string()}))
        .collect();
    if !issues.is_empty() {
        return Err(ToolFailure {
            code: ToolErrorCode::InvalidToolArguments,
            details: json!({"issues":issues}),
        }
        .into());
    }
    Ok(())
}
