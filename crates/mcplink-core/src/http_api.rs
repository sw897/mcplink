//! Declarative, same-origin HTTP operations exposed through the managed MCP call path.
use crate::{
    Engine, config, invocation,
    model::{Connection, Definition, digest, now},
};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

const MAX_RESPONSE: usize = 8 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpApi {
    pub tools: Vec<HttpTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<Probe>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Probe {
    pub tool: String,
    pub arguments: Value,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub path_params: BTreeMap<String, Value>,
    #[serde(default)]
    pub query: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    pub response: ResponseMapping,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseMapping {
    pub r#type: ResponseKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ResponseKind {
    Json,
    Text,
    Image,
    Base64Image,
}

pub fn description() -> Value {
    json!({"protocolVersion":"2025-11-25", "capabilities":{"tools":{}},
        "serverInfo":{"name":"mcplink-http-api","version":env!("CARGO_PKG_VERSION")},
        "instructions":"Configured HTTP API operations. Tool discovery is local; only an actual call or an explicitly configured probe verifies the upstream."})
}

fn valid_pointer(pointer: &str) -> bool {
    (pointer.is_empty() || pointer.starts_with('/'))
        && pointer
            .split('~')
            .skip(1)
            .all(|s| s.starts_with('0') || s.starts_with('1'))
}

fn validate_template(value: &Value, depth: usize) -> Result<()> {
    ensure!(depth < 32, "HTTP 参数模板嵌套过深");
    match value {
        Value::Object(map) if map.contains_key("$input") => {
            ensure!(
                map.keys().all(|k| k == "$input" || k == "default"),
                "HTTP 输入引用只支持 $input 和 default"
            );
            ensure!(
                map["$input"].as_str().is_some_and(valid_pointer),
                "HTTP 输入引用需要 JSON Pointer"
            );
        }
        Value::Object(map) => {
            for value in map.values() {
                validate_template(value, depth + 1)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_template(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn validate(definition: &Definition) -> Result<()> {
    let Some(api) = &definition.http_api else {
        return Ok(());
    };
    ensure!(
        definition.url.is_some() && definition.command.is_none(),
        "HTTP API 需要基础 URL"
    );
    ensure!(
        !definition.oauth
            && definition.client_id.is_none()
            && definition.client_secret.is_none()
            && definition.oauth_scopes.is_empty()
            && definition.oauth_callback_port.is_none(),
        "HTTP API 首版使用托管请求头认证，不支持 MCP OAuth 发现"
    );
    let base = endpoint(definition)?;
    ensure!(
        base.query().is_none() && base.fragment().is_none(),
        "HTTP API 基础 URL 不含 query 或 fragment；参数请放入工具映射"
    );
    ensure!(
        !api.tools.is_empty()
            && api.tools.len() <= 128
            && serde_json::to_vec(api)?.len() <= 512 * 1024,
        "HTTP API 工具定义为空或超过限制"
    );
    for key in definition.headers.keys() {
        checked_header_name(key)?;
    }
    let mut names = HashSet::new();
    for tool in &api.tools {
        config::safe_name(&tool.name)?;
        ensure!(names.insert(&tool.name), "HTTP API 工具名重复");
        ensure!(
            !tool.description.trim().is_empty() && tool.description.len() <= 8192,
            "HTTP API 工具需要有效描述"
        );
        ensure!(
            matches!(
                tool.method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
            ),
            "不支持的 HTTP 方法"
        );
        ensure!(
            !matches!(tool.method.as_str(), "GET" | "HEAD") || tool.body.is_none(),
            "GET/HEAD 不支持 JSON body"
        );
        ensure!(
            tool.path.starts_with('/')
                && !tool.path.starts_with("//")
                && !tool.path.contains(['?', '#', '\\'])
                && !tool.path.chars().any(char::is_control)
                && tool.path.len() <= 2048,
            "HTTP API 路径必须是同源绝对路径，不含 query 或 fragment"
        );
        let mut parameters = HashSet::new();
        for segment in tool.path.split('/') {
            if let Some(name) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                ensure!(
                    !name.is_empty() && tool.path_params.contains_key(name),
                    "HTTP 路径参数缺少映射"
                );
                parameters.insert(name.to_owned());
            } else {
                ensure!(
                    !segment.contains(['{', '}']),
                    "HTTP 路径参数必须占据整个路径段"
                );
            }
        }
        ensure!(
            parameters.len() == tool.path_params.len(),
            "HTTP 路径映射包含未使用的参数"
        );
        invocation::validator(&tool.input_schema)?;
        ensure!(
            tool.input_schema["type"] == "object",
            "HTTP 工具输入 Schema 需要 object 类型"
        );
        for template in tool
            .path_params
            .values()
            .chain(tool.query.values())
            .chain(tool.body.iter())
        {
            validate_template(template, 0)?;
        }
        ensure!(
            tool.response.pointer.as_deref().is_none_or(valid_pointer),
            "响应映射需要有效 JSON Pointer"
        );
        ensure!(
            tool.response.pointer.is_none()
                || matches!(
                    tool.response.r#type,
                    ResponseKind::Json | ResponseKind::Base64Image
                ),
            "此响应类型不支持 JSON Pointer"
        );
        if let Some(mime) = &tool.response.mime_type {
            ensure!(image_mime(mime), "仅支持 PNG、JPEG、WebP 图像");
        }
        if let Some(annotations) = &tool.annotations {
            ensure!(annotations.is_object(), "工具 annotations 需要对象");
            serde_json::from_value::<rmcp::model::ToolAnnotations>(annotations.clone())
                .map_err(|_| anyhow::anyhow!("工具 annotations 格式无效"))?;
        }
    }
    if let Some(probe) = &api.probe {
        let tool = api
            .tools
            .iter()
            .find(|t| t.name == probe.tool)
            .context("HTTP probe 引用不存在的工具")?;
        invocation::validate(&tool.input_schema, &probe.arguments)?;
        prepare(definition, tool, &probe.arguments)?;
    }
    Ok(())
}

fn endpoint(definition: &Definition) -> Result<url::Url> {
    let raw = crate::mcp::http_endpoint(
        definition.url.as_deref().context("HTTP API 缺少 URL")?,
        definition.origin_project.as_deref(),
    )?;
    url::Url::parse(&raw).context("HTTP API 地址无效")
}

fn render(template: &Value, input: &Value) -> Result<Option<Value>> {
    match template {
        Value::Object(map) if map.contains_key("$input") => {
            let pointer = map["$input"].as_str().context("HTTP 输入引用无效")?;
            Ok(input
                .pointer(pointer)
                .or_else(|| map.get("default"))
                .cloned())
        }
        Value::Object(map) => {
            let mut output = serde_json::Map::new();
            for (key, value) in map {
                if let Some(value) = render(value, input)? {
                    output.insert(key.clone(), value);
                }
            }
            Ok(Some(Value::Object(output)))
        }
        Value::Array(values) => Ok(Some(Value::Array(
            values
                .iter()
                .map(|v| render(v, input)?.context("HTTP 数组模板缺少输入值"))
                .collect::<Result<_>>()?,
        ))),
        value => Ok(Some(value.clone())),
    }
}

fn scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Bool(_) | Value::Number(_) => Ok(value.to_string()),
        _ => bail!("HTTP 路径和 query 参数需要字符串、数字或布尔值；数组按逗号连接"),
    }
}

fn checked_header_name(key: &str) -> Result<reqwest::header::HeaderName> {
    let key =
        reqwest::header::HeaderName::from_bytes(key.as_bytes()).context("HTTP 请求头名称无效")?;
    ensure!(
        key != reqwest::header::HOST
            && key != reqwest::header::CONTENT_LENGTH
            && key != reqwest::header::TRANSFER_ENCODING,
        "此 HTTP 请求头由运行时维护"
    );
    Ok(key)
}

fn prepare(definition: &Definition, tool: &HttpTool, input: &Value) -> Result<reqwest::Request> {
    ensure!(input.is_object(), "HTTP 工具参数需要 JSON 对象");
    invocation::validate(&tool.input_schema, input)?;
    let base = endpoint(definition)?;
    let mut segments = Vec::new();
    for segment in tool.path.split('/') {
        if let Some(name) = segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let value = render(&tool.path_params[name], input)?.context("HTTP 路径参数缺失")?;
            let value = scalar(&value)?;
            ensure!(
                !value.is_empty() && value != "." && value != "..",
                "HTTP 路径参数无效"
            );
            segments.push(
                url::form_urlencoded::byte_serialize(value.as_bytes())
                    .collect::<String>()
                    .replace('+', "%20"),
            );
        } else {
            segments.push(segment.to_owned());
        }
    }
    let mut url = base.join(&segments.join("/")).context("HTTP 路径无效")?;
    ensure!(
        url.origin() == base.origin() && url.fragment().is_none(),
        "HTTP 请求必须保持同源"
    );
    for (key, template) in &tool.query {
        if let Some(value) = render(template, input)? {
            if value.is_null() {
                continue;
            }
            let value = match value.as_array() {
                Some(values) => values
                    .iter()
                    .map(scalar)
                    .collect::<Result<Vec<_>>>()?
                    .join(","),
                None => scalar(&value)?,
            };
            url.query_pairs_mut().append_pair(key, &value);
        }
    }
    let mut request = reqwest::Request::new(
        reqwest::Method::from_bytes(tool.method.as_bytes()).context("HTTP 方法无效")?,
        url,
    );
    for (key, value) in &definition.headers {
        let key = checked_header_name(key)?;
        let value = config::resolve(value, definition.origin_project.as_deref())?;
        let value = reqwest::header::HeaderValue::from_str(&value)
            .map_err(|_| anyhow::anyhow!("HTTP 请求头值无效"))?;
        request.headers_mut().insert(key, value);
    }
    if let Some(template) = &tool.body {
        let body = render(template, input)?.context("HTTP JSON body 缺少输入")?;
        *request.body_mut() = Some(serde_json::to_vec(&body)?.into());
        request.headers_mut().insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }
    Ok(request)
}

fn image_mime(mime: &str) -> bool {
    matches!(mime, "image/png" | "image/jpeg" | "image/webp")
}

fn image_result(bytes: &[u8], mime: &str) -> Result<Value> {
    let valid = match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        _ => false,
    };
    ensure!(valid, "HTTP API 图片类型或签名无效");
    Ok(
        json!({"content":[{"type":"image","mimeType":mime,"data":STANDARD.encode(bytes)}],
        "structuredContent":{"mimeType":mime,"bytes":bytes.len(),"sha256":digest(bytes)},"isError":false}),
    )
}

async fn execute(tool: &HttpTool, request: reqwest::Request) -> Result<Value> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(15))
        .build()
        .context("无法建立 HTTP API 客户端")?;
    let mut response = client
        .execute(request)
        .await
        .map_err(|_| anyhow::anyhow!("HTTP API 通信失败；执行结果待核对，不要自动重试"))?;
    let status = response.status();
    if !status.is_success() {
        return Ok(
            json!({"content":[{"type":"text","text":format!("HTTP API 返回 HTTP {}", status.as_u16())}],"isError":true,
            "structuredContent":{"error":{"code":"http_status","status":status.as_u16()}}}),
        );
    }
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= MAX_RESPONSE as u64),
        "HTTP API 响应超过 8 MiB 限制"
    );
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("HTTP API 响应读取中断；执行结果待核对，不要自动重试"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= MAX_RESPONSE,
            "HTTP API 响应超过 8 MiB 限制"
        );
        bytes.extend_from_slice(&chunk);
    }
    let mut result = match tool.response.r#type {
        ResponseKind::Json => {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("HTTP API 未返回有效 JSON"))?;
            let value = match &tool.response.pointer {
                Some(pointer) => value
                    .pointer(pointer)
                    .context("HTTP JSON 响应缺少映射字段")?,
                None => &value,
            };
            json!({"content":[{"type":"text","text":serde_json::to_string(value)?}],"structuredContent":if value.is_object() { value.clone() } else { json!({"data":value}) },"isError":false})
        }
        ResponseKind::Text => {
            let text = String::from_utf8(bytes)
                .map_err(|_| anyhow::anyhow!("HTTP API 文本不是有效 UTF-8"))?;
            json!({"content":[{"type":"text","text":text}],"isError":false})
        }
        ResponseKind::Image => {
            ensure!(
                image_mime(&mime)
                    && tool
                        .response
                        .mime_type
                        .as_deref()
                        .is_none_or(|expected| expected == mime),
                "HTTP API 未返回预期图片类型"
            );
            image_result(&bytes, &mime)?
        }
        ResponseKind::Base64Image => {
            let value;
            let encoded = match &tool.response.pointer {
                Some(pointer) => {
                    value = serde_json::from_slice::<Value>(&bytes)
                        .map_err(|_| anyhow::anyhow!("HTTP API 未返回有效 JSON"))?;
                    value
                        .pointer(pointer)
                        .and_then(Value::as_str)
                        .context("HTTP Base64 图片字段缺失")?
                }
                None => std::str::from_utf8(&bytes)
                    .map_err(|_| anyhow::anyhow!("HTTP API Base64 图片编码无效"))?
                    .trim(),
            };
            let decoded = STANDARD
                .decode(encoded)
                .map_err(|_| anyhow::anyhow!("HTTP API Base64 图片编码无效"))?;
            image_result(
                &decoded,
                tool.response.mime_type.as_deref().unwrap_or("image/png"),
            )?
        }
    };
    result["_meta"] = json!({"mcplink":{"httpStatus":status.as_u16()}});
    Ok(result)
}

pub(crate) async fn request(
    engine: Arc<Engine>,
    connection: &Connection,
    definition: &Definition,
    request: &Value,
) -> Result<Value> {
    ensure!(connection.active, "连接已断开，请重新连接");
    let api = definition.http_api.as_ref().context("缺少 HTTP API 定义")?;
    match request["method"].as_str().unwrap_or("") {
        "tools/list" => {
            ensure!(
                request.pointer("/params/cursor").is_none_or(Value::is_null),
                "HTTP API 清单游标无效"
            );
            let tools: Vec<_> = api.tools.iter().filter(|t| crate::mcp::tool_allowed(definition, &t.name)).map(|t| {
                let mut descriptor = json!({"name":t.name,"description":t.description,"inputSchema":t.input_schema});
                if let Some(annotations) = &t.annotations { descriptor["annotations"] = annotations.clone(); }
                descriptor
            }).collect();
            return Ok(json!({"tools":tools}));
        }
        "resources/list" => return Ok(json!({"resources":[]})),
        "resources/templates/list" => return Ok(json!({"resourceTemplates":[]})),
        "prompts/list" => return Ok(json!({"prompts":[]})),
        "tools/call" => {}
        _ => bail!("HTTP API 连接未实现此 MCP 能力"),
    }
    let name = request
        .pointer("/params/name")
        .and_then(Value::as_str)
        .context("HTTP 工具名缺失")?;
    let tool = api
        .tools
        .iter()
        .find(|t| t.name == name)
        .context("HTTP 工具不存在")?;
    let input = request
        .pointer("/params/arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let prepared = prepare(definition, tool, &input)?;
    let current = engine.store.connection(&connection.id)?;
    ensure!(
        current.revision == connection.revision && current.active,
        "调用前连接已更新，请重新发现工具"
    );
    let limit = Duration::from_secs(definition.timeout.unwrap_or(120).clamp(5, 600));
    let mut shutdown = engine.shutdown.subscribe();
    let outcome = tokio::select! {
        _ = shutdown.wait_for(|stopping| *stopping) => Err(anyhow::anyhow!("后台正在退出；执行结果待核对，不要自动重试")),
        outcome = tokio::time::timeout(limit, execute(tool, prepared)) => outcome.unwrap_or_else(|_| Err(anyhow::anyhow!("HTTP API 请求超时；执行结果待核对，不要自动重试"))),
    };
    let _guard = engine.mutation.lock().await;
    let mut current = engine.store.connection(&connection.id)?;
    ensure!(
        current.revision == connection.revision && current.active,
        "调用期间连接已更新；执行结果待核对，不要自动重试"
    );
    current.checked_at = Some(now());
    let failed = outcome.as_ref().is_ok_and(|v| v["isError"] == true);
    current.status = if outcome.as_ref().is_ok_and(|v| {
        matches!(
            v.pointer("/structuredContent/error/status")
                .and_then(Value::as_u64),
            Some(401 | 403)
        )
    }) {
        "needs_auth"
    } else if outcome.is_err() || failed {
        "error"
    } else {
        "ready"
    }
    .into();
    current.last_error = match &outcome {
        Err(error) => Some(error.to_string()),
        Ok(value) if failed => Some(
            value["content"][0]["text"]
                .as_str()
                .unwrap_or("HTTP API 调用失败")
                .to_owned(),
        ),
        _ => None,
    };
    current.tool_count = Some(
        api.tools
            .iter()
            .filter(|t| crate::mcp::tool_allowed(definition, &t.name))
            .count(),
    );
    engine.store.save_connection(&current)?;
    outcome
}

pub(crate) async fn probe(engine: Arc<Engine>, id: &str, execute_probe: bool) -> Result<Value> {
    let original = engine.store.connection(id)?;
    let definition = engine.definition(&original)?;
    let api = definition.http_api.as_ref().context("缺少 HTTP API 配置")?;
    let listed = engine
        .pool
        .request(
            engine.clone(),
            id,
            json!({"method":"tools/list","params":{}}),
        )
        .await?;
    let mut verification = if original.status == "ready" {
        "previous_call"
    } else {
        "not_checked"
    };
    if execute_probe && let Some(probe) = &api.probe {
        engine.pool.request(engine.clone(), id, json!({"method":"tools/call","params":{"name":probe.tool,"arguments":probe.arguments}})).await?;
        verification = "probe_call";
    }
    let connection = engine.store.connection(id)?;
    ensure!(
        connection.revision == original.revision,
        "检查期间连接已更新，请重试"
    );
    Ok(
        json!({"connection":connection.public_value(),"tools":listed["tools"],
        "verification":{"configuration":"valid","upstream":verification},"schemaHash":digest(&serde_json::to_vec(&listed["tools"])?) }),
    )
}
