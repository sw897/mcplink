//! Offline OpenAPI/Swagger conversion. Imports produce drafts, never requests or writes.
use super::{HttpApi, HttpTool, QueryEncoding, ResponseKind, ResponseMapping};
use crate::{config, model::Definition};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_OPERATIONS: usize = 1024;

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportRequest {
    pub source: String,
    pub server_url: Option<String>,
    /// Stable operation keys, e.g. `POST /notes/search`; not inferred permissions.
    pub operations: Vec<String>,
    pub all: bool,
    pub tool_names: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Operation {
    key: String,
    operation_id: Option<String>,
    name: String,
    method: String,
    path: String,
    summary: String,
    supported: bool,
    selected: bool,
    issue: Option<String>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct CredentialInput {
    header: String,
    kind: String,
    hint: String,
}

fn resolve<'a>(mut value: &'a Value, document: &'a Value) -> Result<&'a Value> {
    let mut visited = BTreeSet::new();
    while let Some(reference) = value.get("$ref") {
        let reference = reference.as_str().context("引用需要字符串")?;
        ensure!(
            reference.starts_with("#/"),
            "外部引用未读取，请先合并为本地文档"
        );
        ensure!(
            visited.insert(reference) && visited.len() <= 32,
            "引用循环或引用链过深"
        );
        ensure!(
            value.as_object().is_some_and(|o| o
                .keys()
                .all(|k| matches!(k.as_str(), "$ref" | "summary" | "description"))),
            "对象引用含不支持的同级字段"
        );
        value = document
            .pointer(&reference[1..])
            .context("本地引用不存在")?;
    }
    Ok(value)
}

fn schema(
    value: &Value,
    document: &Value,
    budget: &mut usize,
    refs: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Value> {
    ensure!(depth < 32 && *budget > 0, "Schema 展开超过深度或容量限制");
    *budget -= 1;
    if value.is_boolean() {
        return Ok(value.clone());
    }
    let object = value.as_object().context("Schema 需要对象或布尔值")?;
    if let Some(reference) = object.get("$ref") {
        let reference = reference.as_str().context("Schema 引用需要字符串")?;
        ensure!(
            reference.starts_with("#/"),
            "Schema 外部引用未读取，请先合并为本地文档"
        );
        ensure!(
            refs.insert(reference.to_owned()),
            "递归 Schema 尚不在导入范围"
        );
        let target = document
            .pointer(&reference[1..])
            .context("Schema 本地引用不存在")?;
        let resolved = schema(target, document, budget, refs, depth + 1)?;
        refs.remove(reference);
        let siblings: Map<_, _> = object
            .iter()
            .filter(|(k, _)| k.as_str() != "$ref")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if siblings.is_empty() {
            return Ok(resolved);
        }
        let siblings = schema(&Value::Object(siblings), document, budget, refs, depth + 1)?;
        return Ok(json!({"allOf":[resolved, siblings]}));
    }
    let mut out = Map::new();
    let readonly: BTreeSet<_> = object
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, v)| v.get("readOnly") == Some(&Value::Bool(true)))
        .map(|(k, _)| k.clone())
        .collect();
    for (key, value) in object {
        // Examples/defaults may contain live credentials. Defaults are annotations,
        // not executable request values; callers and the service retain control.
        if matches!(
            key.as_str(),
            "example"
                | "examples"
                | "default"
                | "nullable"
                | "readOnly"
                | "writeOnly"
                | "xml"
                | "discriminator"
        ) || key.starts_with("x-")
        {
            continue;
        }
        let value = match key.as_str() {
            "properties" | "$defs" | "definitions" | "patternProperties" | "dependentSchemas" => {
                let values = value.as_object().context("Schema 属性映射无效")?;
                let mut properties = Map::new();
                for (name, child) in values {
                    if key == "properties" && readonly.contains(name) {
                        continue;
                    }
                    properties.insert(
                        name.clone(),
                        schema(child, document, budget, refs, depth + 1)?,
                    );
                }
                Value::Object(properties)
            }
            "items"
            | "additionalProperties"
            | "unevaluatedProperties"
            | "contains"
            | "not"
            | "if"
            | "then"
            | "else"
            | "propertyNames" => schema(value, document, budget, refs, depth + 1)?,
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => Value::Array(
                value
                    .as_array()
                    .context("Schema 组合需要数组")?
                    .iter()
                    .map(|v| schema(v, document, budget, refs, depth + 1))
                    .collect::<Result<_>>()?,
            ),
            "required" => Value::Array(
                value
                    .as_array()
                    .context("required 需要数组")?
                    .iter()
                    .filter(|v| !v.as_str().is_some_and(|s| readonly.contains(s)))
                    .cloned()
                    .collect(),
            ),
            "exclusiveMinimum" | "exclusiveMaximum" if value.is_boolean() => {
                if value == &Value::Bool(false) {
                    continue;
                }
                let bound = if key == "exclusiveMinimum" {
                    "minimum"
                } else {
                    "maximum"
                };
                object
                    .get(bound)
                    .filter(|v| v.is_number())
                    .context("旧版排他边界缺少数值")?
                    .clone()
            }
            _ => value.clone(),
        };
        out.insert(key.clone(), value);
    }
    if object.get("exclusiveMinimum") == Some(&Value::Bool(true)) {
        out.remove("minimum");
    }
    if object.get("exclusiveMaximum") == Some(&Value::Bool(true)) {
        out.remove("maximum");
    }
    ensure!(
        out.get("type").and_then(Value::as_str) != Some("file"),
        "文件参数尚不在导入范围"
    );
    let result = Value::Object(out);
    if object.get("nullable") == Some(&Value::Bool(true)) {
        Ok(json!({"anyOf":[result,{"type":"null"}]}))
    } else {
        Ok(result)
    }
}

fn normalized_schema(value: &Value, document: &Value) -> Result<Value> {
    schema(value, document, &mut 50_000, &mut BTreeSet::new(), 0)
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            result.push(character);
        } else if !result.ends_with('_') {
            result.push('_');
        }
        if result.len() >= 72 {
            break;
        }
    }
    let result = result.trim_matches('_');
    if result.is_empty() {
        "http_api".into()
    } else {
        result.to_owned()
    }
}

fn server_value(value: &Value) -> Option<String> {
    let mut url = value.get("url")?.as_str()?.to_owned();
    if let Some(variables) = value.get("variables").and_then(Value::as_object) {
        for (name, definition) in variables {
            let default = definition.get("default")?.as_str()?;
            url = url.replace(&format!("{{{name}}}"), default);
        }
    }
    if url.contains(['{', '}']) {
        return None;
    }
    Some(url)
}

fn checked_server(value: &str) -> Result<(String, String)> {
    let url = url::Url::parse(value).map_err(|_| anyhow::anyhow!("请提供完整的服务地址"))?;
    ensure!(
        url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "服务地址不能包含凭据、查询参数或片段"
    );
    let origin = url.origin().ascii_serialization();
    let definition = Definition {
        url: Some(origin.clone()),
        enabled: true,
        ..Default::default()
    };
    super::endpoint(&definition)?;
    Ok((origin, url.path().trim_end_matches('/').to_owned()))
}

fn servers(document: &Value, swagger: bool) -> Vec<String> {
    let mut result = Vec::new();
    if swagger {
        if let Some(host) = document.get("host").and_then(Value::as_str) {
            let schemes = document
                .get("schemes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_else(|| vec![json!("https")]);
            let base = document
                .get("basePath")
                .and_then(Value::as_str)
                .unwrap_or("");
            for scheme in schemes.iter().filter_map(Value::as_str) {
                result.push(format!("{scheme}://{host}{base}"));
            }
        }
    } else {
        if let Some(values) = document.get("servers").and_then(Value::as_array) {
            result.extend(values.iter().filter_map(server_value));
        }
        for path in document
            .get("paths")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(_, v)| v)
        {
            if let Some(values) = path.get("servers").and_then(Value::as_array) {
                result.extend(values.iter().filter_map(server_value));
            }
            for op in path.as_object().into_iter().flatten().map(|(_, v)| v) {
                if let Some(values) = op.get("servers").and_then(Value::as_array) {
                    result.extend(values.iter().filter_map(server_value));
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    result
        .into_iter()
        .filter(|url| checked_server(url).is_ok() && seen.insert(url.clone()))
        .collect()
}

fn credentials(operation: &Value, document: &Value, swagger: bool) -> Result<Vec<CredentialInput>> {
    let Some(requirements) = operation
        .get("security")
        .or_else(|| document.get("security"))
    else {
        return Ok(vec![]);
    };
    let alternatives = requirements.as_array().context("security 需要数组")?;
    if alternatives.is_empty()
        || alternatives
            .iter()
            .any(|a| a.as_object().is_some_and(Map::is_empty))
    {
        return Ok(vec![]);
    }
    let definitions = if swagger {
        &document["securityDefinitions"]
    } else {
        &document["components"]["securitySchemes"]
    };
    for alternative in alternatives {
        let Some(entries) = alternative.as_object() else {
            continue;
        };
        let mut result = Vec::new();
        let mut supported = true;
        for name in entries.keys() {
            let Some(scheme) = definitions.get(name) else {
                supported = false;
                break;
            };
            let scheme = resolve(scheme, document)?;
            let (header, kind, hint) = match scheme["type"].as_str() {
                Some("apiKey") if scheme["in"] == "header" => (
                    scheme["name"]
                        .as_str()
                        .context("API Key 请求头名称缺失")?
                        .to_owned(),
                    "apiKey",
                    "填写此账号有权使用的 API Key 原值",
                ),
                Some("basic") => ("Authorization".into(), "basic", "填写完整 Basic 认证值"),
                Some("http")
                    if scheme["scheme"]
                        .as_str()
                        .is_some_and(|s| s.eq_ignore_ascii_case("bearer")) =>
                {
                    ("Authorization".into(), "bearer", "填写完整 Bearer 认证值")
                }
                Some("http")
                    if scheme["scheme"]
                        .as_str()
                        .is_some_and(|s| s.eq_ignore_ascii_case("basic")) =>
                {
                    ("Authorization".into(), "basic", "填写完整 Basic 认证值")
                }
                Some("oauth2" | "openIdConnect") => (
                    "Authorization".into(),
                    "oauthToken",
                    "此导入不执行 OAuth；需从有权使用的授权来源提供完整 Bearer 认证值",
                ),
                _ => {
                    supported = false;
                    break;
                }
            };
            super::checked_header_name(&header)?;
            result.push(CredentialInput {
                header,
                kind: kind.into(),
                hint: hint.into(),
            });
        }
        if supported {
            return Ok(result);
        }
    }
    bail!("鉴权方案尚不支持自动映射；仅支持请求头 API Key、Basic、Bearer 或手工 OAuth Token")
}

fn parameter_schema(parameter: &Value, document: &Value, swagger: bool) -> Result<Value> {
    if !swagger {
        return normalized_schema(
            parameter
                .get("schema")
                .context("参数缺少 schema；content 参数尚不支持")?,
            document,
        );
    }
    if parameter["in"] == "body" {
        return normalized_schema(
            parameter.get("schema").context("body 参数缺少 schema")?,
            document,
        );
    }
    let mut value = parameter.as_object().context("参数需要对象")?.clone();
    for key in [
        "name",
        "in",
        "required",
        "allowEmptyValue",
        "collectionFormat",
    ] {
        value.remove(key);
    }
    normalized_schema(&Value::Object(value), document)
}

fn simple_type(value: &Value) -> Option<&str> {
    if let Some(value) = value.get("type").and_then(Value::as_str) {
        return Some(value);
    }
    if let Some(values) = value.get("type").and_then(Value::as_array) {
        return values
            .iter()
            .filter_map(Value::as_str)
            .find(|v| *v != "null");
    }
    for key in ["allOf", "anyOf"] {
        if let Some(values) = value.get(key).and_then(Value::as_array)
            && let Some(kind) = values.iter().filter_map(simple_type).find(|v| *v != "null")
        {
            return Some(kind);
        }
    }
    None
}

fn json_body(
    operation: &Value,
    document: &Value,
    swagger: bool,
    parameters: &[Value],
) -> Result<Option<(Value, bool, String)>> {
    if swagger {
        ensure!(
            !parameters.iter().any(|p| p["in"] == "formData"),
            "formData/multipart 参数尚不支持"
        );
        let bodies: Vec<_> = parameters.iter().filter(|p| p["in"] == "body").collect();
        ensure!(bodies.len() <= 1, "请求包含多个 body 参数");
        let Some(parameter) = bodies.first() else {
            return Ok(None);
        };
        let content_types = operation
            .get("consumes")
            .or_else(|| document.get("consumes"))
            .and_then(Value::as_array);
        let mime = content_types
            .map(|types| {
                types
                    .iter()
                    .filter_map(Value::as_str)
                    .find(|s| super::json_content_type(s))
            })
            .unwrap_or(Some("application/json"))
            .context("请求类型尚不支持；需要 JSON body")?;
        return Ok(Some((
            parameter_schema(parameter, document, true)?,
            parameter["required"] == true,
            mime.into(),
        )));
    }
    let Some(body) = operation.get("requestBody") else {
        return Ok(None);
    };
    let body = resolve(body, document)?;
    let content = body["content"]
        .as_object()
        .context("requestBody 缺少 content")?;
    let (mime, entry) = content
        .get_key_value("application/json")
        .or_else(|| {
            content
                .iter()
                .find(|(mime, _)| super::json_content_type(mime))
        })
        .context("请求类型尚不支持；需要 application/json 或 application/*+json")?;
    let schema = normalized_schema(entry.get("schema").unwrap_or(&Value::Bool(true)), document)?;
    Ok(Some((schema, body["required"] == true, mime.clone())))
}

fn response(operation: &Value, document: &Value, swagger: bool) -> Result<ResponseMapping> {
    if let Some(extension) = operation.get("x-mcplink-response") {
        return serde_json::from_value(extension.clone())
            .map_err(|_| anyhow::anyhow!("x-mcplink-response 格式无效"));
    }
    let mut types = BTreeSet::new();
    if swagger {
        if let Some(values) = operation
            .get("produces")
            .or_else(|| document.get("produces"))
            .and_then(Value::as_array)
        {
            types.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
        }
    } else {
        for (status, value) in operation
            .get("responses")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if !status.starts_with('2') {
                continue;
            }
            let value = resolve(value, document)?;
            if let Some(content) = value.get("content").and_then(Value::as_object) {
                types.extend(content.keys().cloned());
            }
        }
    }
    let r#type = if types.iter().any(|s| super::json_content_type(s)) {
        ResponseKind::Json
    } else if types.iter().any(|s| s.starts_with("text/")) || types.is_empty() {
        ResponseKind::Text
    } else if types.iter().all(|s| super::image_mime(s)) {
        ResponseKind::Image
    } else {
        bail!("响应媒体类型尚不支持自动映射");
    };
    Ok(ResponseMapping {
        r#type,
        pointer: None,
        mime_type: None,
        success: None,
        sensitive_paths: vec![],
    })
}

struct ImportContext<'a> {
    document: &'a Value,
    base_path: &'a str,
    selected_server: &'a str,
    swagger: bool,
}

fn build_tool(
    context: &ImportContext<'_>,
    path_item: &Value,
    operation: &Value,
    method: &str,
    path: &str,
    name: &str,
) -> Result<(HttpTool, Vec<CredentialInput>)> {
    let ImportContext {
        document,
        base_path,
        selected_server,
        swagger,
    } = *context;
    if !swagger
        && let Some(local) = operation
            .get("servers")
            .or_else(|| path_item.get("servers"))
            .and_then(Value::as_array)
    {
        ensure!(
            local
                .iter()
                .filter_map(server_value)
                .any(|s| s.trim_end_matches('/') == selected_server.trim_end_matches('/')),
            "此操作使用不同的服务地址，请选择对应地址后单独导入"
        );
    }
    let mut merged = BTreeMap::new();
    for source in [path_item, operation] {
        if let Some(parameters) = source.get("parameters") {
            for parameter in parameters.as_array().context("parameters 需要数组")? {
                let value = resolve(parameter, document)?;
                let location = value["in"].as_str().context("参数缺少位置")?;
                let name = value["name"].as_str().context("参数缺少名称")?;
                merged.insert((location.to_owned(), name.to_owned()), value.clone());
            }
        }
    }
    let parameters: Vec<_> = merged.into_values().collect();
    let mut path_properties = Map::new();
    let mut query_properties = Map::new();
    let mut path_required = Vec::new();
    let mut query_required = Vec::new();
    let mut path_params = BTreeMap::new();
    let mut query = BTreeMap::new();
    let mut query_encoding = BTreeMap::new();
    for parameter in &parameters {
        let name = parameter["name"].as_str().context("参数名称无效")?;
        let location = parameter["in"].as_str().context("参数位置无效")?;
        if location == "body" || location == "formData" {
            continue;
        }
        ensure!(
            matches!(location, "path" | "query"),
            "请求头/cookie 参数尚不支持工具输入映射；认证请使用 securitySchemes"
        );
        ensure!(
            parameter.get("allowReserved") != Some(&Value::Bool(true)),
            "allowReserved 参数尚不支持"
        );
        let schema = parameter_schema(parameter, document, swagger)?;
        let kind = simple_type(&schema).context("参数类型无法映射为标量或标量数组")?;
        ensure!(
            matches!(kind, "string" | "number" | "integer" | "boolean" | "array"),
            "对象参数编码尚不支持"
        );
        let escaped = name.replace('~', "~0").replace('/', "~1");
        if location == "path" {
            ensure!(
                kind != "array" && parameter["required"] == true,
                "path 参数需要必填标量"
            );
            ensure!(
                parameter.get("style").is_none_or(|v| v == "simple"),
                "path 参数编码尚不支持"
            );
            path_properties.insert(name.into(), schema);
            path_required.push(json!(name));
            path_params.insert(name.into(), json!({"$input":format!("/path/{escaped}")}));
        } else {
            if kind == "array" {
                let items = schema.get("items").context("数组参数缺少 items")?;
                ensure!(
                    simple_type(items)
                        .is_some_and(|k| matches!(k, "string" | "number" | "integer" | "boolean")),
                    "query 数组需要标量元素"
                );
                let encoding = if swagger {
                    match parameter["collectionFormat"].as_str().unwrap_or("csv") {
                        "csv" => QueryEncoding::Comma,
                        "multi" => QueryEncoding::Repeated,
                        "ssv" => QueryEncoding::Space,
                        "pipes" => QueryEncoding::Pipe,
                        _ => bail!("query 数组编码尚不支持"),
                    }
                } else {
                    match parameter["style"].as_str().unwrap_or("form") {
                        "form" if parameter.get("explode") != Some(&Value::Bool(false)) => {
                            QueryEncoding::Repeated
                        }
                        "form" => QueryEncoding::Comma,
                        "spaceDelimited"
                            if parameter.get("explode") != Some(&Value::Bool(true)) =>
                        {
                            QueryEncoding::Space
                        }
                        "pipeDelimited" if parameter.get("explode") != Some(&Value::Bool(true)) => {
                            QueryEncoding::Pipe
                        }
                        _ => bail!("query 数组编码尚不支持"),
                    }
                };
                query_encoding.insert(name.into(), encoding);
            } else {
                ensure!(
                    parameter.get("style").is_none_or(|v| v == "form"),
                    "query 参数编码尚不支持"
                );
            }
            query_properties.insert(name.into(), schema);
            if parameter["required"] == true {
                query_required.push(json!(name));
            }
            query.insert(name.into(), json!({"$input":format!("/query/{escaped}")}));
        }
    }
    let mut properties = Map::new();
    let mut required = Vec::new();
    if !path_properties.is_empty() {
        properties.insert("path".into(), json!({"type":"object","properties":path_properties,"required":path_required,"additionalProperties":false}));
        required.push(json!("path"));
    }
    if !query_properties.is_empty() {
        let mandatory = !query_required.is_empty();
        properties.insert("query".into(), json!({"type":"object","properties":query_properties,"required":query_required,"additionalProperties":false}));
        if mandatory {
            required.push(json!("query"));
        }
    }
    let body = json_body(operation, document, swagger, &parameters)?;
    let (body_template, request_content_type) = if let Some((schema, mandatory, mime)) = body {
        ensure!(
            !matches!(method, "GET" | "HEAD"),
            "当前执行层不支持 GET/HEAD body"
        );
        properties.insert("body".into(), schema);
        if mandatory {
            required.push(json!("body"));
        }
        (Some(json!({"$input":"/body"})), Some(mime))
    } else {
        (None, None)
    };
    let summary = operation
        .get("summary")
        .or_else(|| operation.get("description"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.chars().take(2000).collect())
        .unwrap_or_else(|| format!("{method} {path}"));
    let tool = HttpTool {
        name: name.into(),
        description: summary,
        method: method.into(),
        path: format!("{base_path}{path}"),
        input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
        path_params,
        query,
        query_encoding,
        body: body_template,
        request_content_type,
        response: response(operation, document, swagger)?,
        annotations: None,
    };
    let auth = credentials(operation, document, swagger)?;
    Ok((tool, auth))
}

pub fn preview(request: ImportRequest) -> Result<Value> {
    ensure!(
        !request.source.trim().is_empty() && request.source.len() <= MAX_DOCUMENT_BYTES,
        "导入文档为空或超过 4 MiB"
    );
    let document: Value = serde_yaml::from_str(&request.source)
        .map_err(|_| anyhow::anyhow!("文档不是有效的 JSON/YAML；错误内容未回显"))?;
    ensure!(document.is_object(), "OpenAPI/Swagger 文档需要对象");
    let swagger = document.get("swagger").and_then(Value::as_str) == Some("2.0");
    let version = document
        .get(if swagger { "swagger" } else { "openapi" })
        .and_then(Value::as_str)
        .context("文档缺少规范版本")?;
    ensure!(
        swagger
            || ["3.0.", "3.1.", "3.2."]
                .iter()
                .any(|prefix| version.starts_with(prefix)),
        "不支持此 OpenAPI/Swagger 版本"
    );
    let available = servers(&document, swagger);
    let selected_server = request
        .server_url
        .clone()
        .or_else(|| available.first().cloned());
    let mut issues = Vec::new();
    let (origin, prefix) = match selected_server.as_deref() {
        Some(value) => checked_server(value)?,
        None => {
            issues.push(json!({"code":"server_required","message":"请选择或填写完整的服务地址"}));
            (String::new(), String::new())
        }
    };
    let paths = document["paths"]
        .as_object()
        .context("文档缺少 paths 对象")?;
    let selection: BTreeSet<_> = request.operations.iter().cloned().collect();
    let mut seen = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut operations = Vec::new();
    let mut tools = Vec::new();
    let mut inputs = BTreeSet::new();
    for (path, original_path) in paths {
        let path_item = match resolve(original_path, &document) {
            Ok(value) => value,
            Err(_) => {
                issues.push(json!({"code":"path_reference_unsupported","message":"路径引用无效、循环或不是本地引用"}));
                continue;
            }
        };
        for method in [
            "get", "post", "put", "patch", "delete", "head", "options", "trace", "query",
        ] {
            let Some(operation) = path_item.get(method) else {
                continue;
            };
            ensure!(
                operations.len() < MAX_OPERATIONS,
                "操作数量超过 1024 项限制"
            );
            let method = method.to_ascii_uppercase();
            let key = format!("{method} {path}");
            seen.insert(key.clone());
            let operation_id = operation
                .get("operationId")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let name = request
                .tool_names
                .get(&key)
                .cloned()
                .unwrap_or_else(|| slug(operation_id.as_deref().unwrap_or(&key)));
            let built = (|| {
                ensure!(
                    !matches!(method.as_str(), "OPTIONS" | "TRACE" | "QUERY"),
                    "当前执行层不支持此 HTTP 方法"
                );
                ensure!(path.starts_with('/'), "操作路径需要绝对路径");
                config::safe_name(&name)?;
                let (tool, auth) = build_tool(
                    &ImportContext {
                        document: &document,
                        base_path: &prefix,
                        selected_server: selected_server.as_deref().unwrap_or(""),
                        swagger,
                    },
                    path_item,
                    operation,
                    &method,
                    path,
                    &name,
                )?;
                let candidate = Definition {
                    url: Some(if origin.is_empty() {
                        "https://placeholder.invalid".into()
                    } else {
                        origin.clone()
                    }),
                    http_api: Some(HttpApi {
                        tools: vec![tool.clone()],
                        probe: None,
                        sensitive_headers: vec![],
                    }),
                    enabled: true,
                    ..Default::default()
                };
                super::validate(&candidate)?;
                Ok::<_, anyhow::Error>((tool, auth))
            })();
            let selected = selection.contains(&key) || (request.all && built.is_ok());
            let summary = operation
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or(&key)
                .chars()
                .take(500)
                .collect();
            let issue = built.as_ref().err().map(ToString::to_string);
            if selected {
                match built {
                    Ok((tool, auth)) => {
                        if !names.insert(tool.name.clone()) { issues.push(json!({"code":"tool_name_collision","message":"选中操作的工具名称重复，请修改名称"})); }
                        tools.push(tool);
                        inputs.extend(auth);
                    }
                    Err(_) => issues.push(json!({"code":"selected_operation_unsupported","message":"所选操作含不支持的参数、引用、鉴权或媒体类型"})),
                }
            }
            operations.push(Operation {
                key,
                operation_id,
                name,
                method,
                path: path.clone(),
                summary,
                supported: issue.is_none(),
                selected,
                issue,
            });
        }
    }
    ensure!(selection.is_subset(&seen), "选择中包含不存在的操作");
    ensure!(
        request.tool_names.keys().all(|key| seen.contains(key)),
        "工具重命名引用不存在的操作"
    );
    if tools.len() > 128 {
        issues
            .push(json!({"code":"too_many_tools","message":"一次最多导入 128 项工具，请缩小选择"}));
    }
    let mut auth_names = BTreeMap::new();
    for input in &inputs {
        if let Some(previous) = auth_names.insert(input.header.to_ascii_lowercase(), &input.kind)
            && previous != &input.kind
        {
            issues.push(json!({"code":"auth_conflict","message":"所选操作的同名认证请求头使用不同鉴权方案，请分开导入"}));
        }
    }
    let draft = if tools.is_empty() || !issues.is_empty() {
        None
    } else {
        let definition = Definition {
            url: Some(origin),
            include_specified: true,
            include_tools: tools.iter().map(|tool| tool.name.clone()).collect(),
            resources_disabled: true,
            prompts_disabled: true,
            http_api: Some(HttpApi {
                tools,
                probe: None,
                sensitive_headers: inputs.iter().map(|i| i.header.clone()).collect(),
            }),
            enabled: true,
            ..Default::default()
        };
        super::validate(&definition)?;
        Some(definition)
    };
    let title = document
        .pointer("/info/title")
        .and_then(Value::as_str)
        .unwrap_or("HTTP API")
        .chars()
        .take(120)
        .collect::<String>();
    Ok(
        json!({"format":"mcplink-openapi-preview","version":1,"sourceVersion":version,"title":title,
        "suggestedName":slug(&title),"servers":available,"serverUrl":selected_server,"operations":operations,
        "credentialInputs":inputs,"issues":issues,"definition":draft,"networkRequests":0,"configurationWrites":0,
        "notes":["输入参数按 path/query/body 分组；示例值和默认值不作为请求值导入。", "外部或递归引用、对象参数编码及未支持的媒体类型会明确标记。", "导入不验证上游；认证值在本机补齐，业务条件和流程由服务配置或 Skill 定义。"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Value {
        json!({"openapi":"3.1.0","info":{"title":"Fixture"},"servers":[{"url":"https://example.com/v1"}],
        "paths":{"/notes/{id}":{"get":{"operationId":"read_note","parameters":[{"in":"path","name":"id","required":true,"schema":{"type":"string"}},
            {"in":"query","name":"tags","schema":{"type":"array","items":{"type":"string"}}}],"responses":{"200":{"content":{"application/json":{"schema":{"type":"object"}}}}}}}}})
    }

    #[test]
    fn preview_is_unselected_and_selected_draft_keeps_base_path_and_array_serialization() {
        let source = document().to_string();
        let first = preview(ImportRequest {
            source: source.clone(),
            ..Default::default()
        })
        .unwrap();
        assert!(first["definition"].is_null());
        assert_eq!(first["operations"][0]["selected"], false);
        let selected = preview(ImportRequest {
            source,
            operations: vec!["GET /notes/{id}".into()],
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            selected.pointer("/definition/httpApi/tools/0/path"),
            Some(&json!("/v1/notes/{id}"))
        );
        assert_eq!(
            selected.pointer("/definition/httpApi/tools/0/queryEncoding/tags"),
            Some(&json!("repeated"))
        );
        assert_eq!(selected["networkRequests"], 0);
    }

    #[test]
    fn yaml_swagger_and_local_refs_are_supported_without_copying_examples_or_defaults() {
        let source = "swagger: '2.0'\ninfo: {title: Fixture}\nhost: example.com\nbasePath: /v2\npaths:\n  /search:\n    post:\n      operationId: search\n      parameters:\n        - {name: body, in: body, required: true, schema: {$ref: '#/definitions/Input'}}\n      produces: [application/json]\n      responses: {'200': {description: OK}}\ndefinitions:\n  Input:\n    type: object\n    properties:\n      query: {type: string, default: synthetic-credential-example, example: synthetic-credential-example}\n";
        let result = preview(ImportRequest {
            source: source.into(),
            all: true,
            ..Default::default()
        })
        .unwrap();
        assert!(result["definition"].is_object());
        assert!(!result.to_string().contains("synthetic-credential-example"));
        assert_eq!(
            result.pointer("/definition/httpApi/tools/0/path"),
            Some(&json!("/v2/search"))
        );
    }

    #[test]
    fn unsupported_operations_and_external_refs_do_not_silently_enter_a_draft() {
        let mut doc = document();
        doc["paths"]["/bad"] = json!({"post":{"requestBody":{"content":{"multipart/form-data":{"schema":{"type":"object"}}}}}});
        doc["paths"]["/remote"] = json!({"$ref":"https://not-requested.invalid/schema"});
        let result = preview(ImportRequest {
            source: doc.to_string(),
            operations: vec!["POST /bad".into()],
            ..Default::default()
        })
        .unwrap();
        assert!(result["definition"].is_null());
        assert!(!result["issues"].as_array().unwrap().is_empty());
        assert_eq!(result["networkRequests"], 0);
    }

    #[test]
    fn security_schemes_produce_inputs_and_declarative_response_conditions() {
        let mut doc = document();
        doc["components"] = json!({"securitySchemes":{"credential":{"type":"apiKey","in":"header","name":"X-Account-Key"}}});
        doc["security"] = json!([{"credential":[]}]);
        doc["paths"]["/notes/{id}"]["get"]["x-mcplink-response"] = json!({"type":"json","success":{"pointer":"/result","equals":"ok"},"sensitivePaths":["/temporaryCredential"]});
        let result = preview(ImportRequest {
            source: doc.to_string(),
            all: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(result["credentialInputs"][0]["header"], "X-Account-Key");
        assert_eq!(result["definition"]["headers"], json!({}));
        assert_eq!(
            result.pointer("/definition/httpApi/tools/0/response/success/equals"),
            Some(&json!("ok"))
        );
    }
}
