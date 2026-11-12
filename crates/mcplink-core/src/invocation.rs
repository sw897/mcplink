//! Discover and invoke exact managed tools without a second connection or credential store.
use crate::{Engine, model::digest};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

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

pub(crate) struct ToolContract {
    pub connection_id: String,
    pub entry_revision: u64,
    pub tool: Value,
    pub schema_hash: String,
    route: Option<crate::gateway::ToolRoute>,
}

impl ToolContract {
    fn new(
        id: &str,
        revision: u64,
        mut tool: Value,
        route: Option<crate::gateway::ToolRoute>,
    ) -> Result<Self> {
        let schema_hash = schema_hash(&tool)?;
        if let Some(route) = &route {
            route.public_metadata(&mut tool);
        }
        Ok(Self {
            connection_id: id.to_owned(),
            entry_revision: revision,
            tool,
            schema_hash,
            route,
        })
    }

    fn public_value(&self) -> Value {
        json!({"connectionId":self.connection_id,"connectionRevision":self.entry_revision,
            "schemaHash":self.schema_hash,"tool":self.tool,
            "validation":{"drafts":["draft-04","draft-06","draft-07","2019-09","2020-12"],
            "defaultDraft":"2020-12","externalReferences":false,"formatAssertions":false,"customVocabularies":false}})
    }
}

pub(crate) async fn contracts(engine: Arc<Engine>, id: &str) -> Result<Vec<ToolContract>> {
    let (tools, _) = contract_report(engine, id).await?;
    Ok(tools)
}

pub(crate) async fn contract_report(
    engine: Arc<Engine>,
    id: &str,
) -> Result<(Vec<ToolContract>, Vec<Value>)> {
    let (_, tools, diagnostics) = contracts_with_pool(engine.clone(), id, &engine.pool).await?;
    Ok((tools, diagnostics))
}

async fn contracts_with_pool(
    engine: Arc<Engine>,
    id: &str,
    pool: &crate::mcp::McpPool,
) -> Result<(u64, Vec<ToolContract>, Vec<Value>)> {
    let connection = engine.store.connection(id)?;
    if connection.members.is_some() {
        let mut catalog =
            crate::gateway::collect_entry(engine, &connection, crate::catalog::CatalogKind::Tools)
                .await?;
        let tools = catalog
            .rows
            .into_iter()
            .map(|tool| {
                let route = tool["name"]
                    .as_str()
                    .and_then(|name| catalog.routes.remove(name))
                    .context("聚合工具路由缺失")?;
                ToolContract::new(id, connection.revision, tool, Some(route))
            })
            .collect::<Result<_>>()?;
        return Ok((connection.revision, tools, catalog.errors));
    }
    let catalog = crate::catalog::collect(engine, pool, id, crate::catalog::CatalogKind::Tools)
        .await
        .map_err(
            |error| match error.downcast_ref::<crate::catalog::CatalogFailure>() {
                Some(failure) => anyhow::Error::new(ToolFailure::new(match failure {
                    crate::catalog::CatalogFailure::Changed => ToolErrorCode::ConnectionChanged,
                    crate::catalog::CatalogFailure::Invalid => ToolErrorCode::ToolDiscoveryFailed,
                })),
                None => error,
            },
        )?;
    let tools = catalog
        .items
        .into_iter()
        .map(|tool| ToolContract::new(id, catalog.revision, tool, None))
        .collect::<Result<_>>()?;
    Ok((catalog.revision, tools, catalog.diagnostics))
}

pub(crate) async fn discover_report(
    engine: Arc<Engine>,
    id: &str,
    pool: &crate::mcp::McpPool,
) -> Result<(u64, Vec<Value>, Vec<Value>)> {
    let (revision, tools, diagnostics) = contracts_with_pool(engine, id, pool).await?;
    Ok((
        revision,
        tools.into_iter().map(|contract| contract.tool).collect(),
        diagnostics,
    ))
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

async fn find_contract(engine: Arc<Engine>, id: &str, name: &str) -> Result<ToolContract> {
    contracts(engine, id)
        .await?
        .into_iter()
        .find(|contract| contract.tool["name"] == name)
        .context(ToolFailure::new(ToolErrorCode::ToolNotFound))
}

pub async fn describe(engine: Arc<Engine>, id: &str, name: &str) -> Result<Value> {
    Ok(find_contract(engine, id, name).await?.public_value())
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
    let contract = find_contract(engine.clone(), id, name).await?;
    if expected_hash.is_some_and(|expected| expected != contract.schema_hash) {
        bail!(ToolFailure::new(ToolErrorCode::ToolSchemaChanged));
    }
    let schema = contract.tool["inputSchema"].clone();
    let input = arguments.clone();
    tokio::task::spawn_blocking(move || validate(&schema, &input))
        .await
        .map_err(|_| ToolFailure::new(ToolErrorCode::InvalidToolSchema))??;
    let current = engine.store.connection(id)?;
    if current.revision != contract.entry_revision {
        bail!(ToolFailure::new(ToolErrorCode::ConnectionChanged));
    }
    let hash = contract.schema_hash;
    let output = if let Some(route) = contract.route {
        crate::gateway::call_tool(engine.clone(), &current, route, hash.clone(), arguments).await?
    } else {
        engine
            .pool
            .request_at_revision(
                engine.clone(),
                id,
                json!({"method":"tools/call","params":{"name":name,"arguments":arguments}}),
                Some(contract.entry_revision),
            )
            .await?
    };
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

pub(crate) fn validate(schema: &Value, input: &Value) -> Result<()> {
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
