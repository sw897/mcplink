use crate::model::{Agent, Definition, Source};
use anyhow::{Context, Result, bail};
use jsonc_parser::{
    ParseOptions,
    cst::{CstContainerNode, CstInputValue, CstNode, CstObject, CstRootNode},
};
use serde_json::{Value, json};

fn options(comments: bool) -> ParseOptions {
    ParseOptions {
        allow_comments: comments,
        allow_trailing_commas: comments,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

fn json5_options() -> ParseOptions {
    ParseOptions {
        allow_loose_object_property_names: true,
        allow_single_quoted_strings: true,
        allow_hexadecimal_numbers: true,
        allow_unary_plus_numbers: true,
        allow_bare_decimal_point_numbers: true,
        allow_extended_string_escapes: true,
        ..options(true)
    }
}

fn document_options(text: &str, options: &ParseOptions) -> Result<CstRootNode> {
    let root = CstRootNode::parse(text, options)
        .map_err(|_| anyhow::anyhow!("JSON 配置无法解析，请在来源文件中修复"))?;
    let mut pending: Vec<_> = root.value().into_iter().collect();
    while let Some(node) = pending.pop() {
        if let CstNode::Container(container) = node {
            if let CstContainerNode::Object(object) = &container {
                let mut names = std::collections::HashSet::new();
                for property in object.properties() {
                    let name = property.name().context("JSON 属性名称无效")?;
                    let name = name.decoded_value().context("JSON 属性名称无效")?;
                    if !names.insert(name) {
                        bail!("JSON 配置存在重复属性，请在来源文件中修复");
                    }
                }
            }
            pending.extend(container.children());
        }
    }
    Ok(root)
}

pub fn parse(text: &str, comments: bool) -> Result<Value> {
    parse_options(text, &options(comments))
}

pub fn parse_json5(text: &str) -> Result<Value> {
    parse_options(text, &json5_options())
}

fn parse_options(text: &str, options: &ParseOptions) -> Result<Value> {
    let root = document_options(text, options)?;
    if root.value().is_none() {
        return Ok(json!({}));
    }
    let value = jsonc_parser::parse_to_serde_value::<Value>(text, options)
        .map_err(|_| anyhow::anyhow!("JSON 配置无法解析，请在来源文件中修复"))?;
    if !value.is_object() {
        bail!("JSON 配置根节点必须是映射");
    }
    Ok(value)
}

pub fn table(root: &Value, path: &[String]) -> Result<Value> {
    let mut value = root;
    for key in path {
        let parent = value.as_object().context("JSON 配置路径需要映射")?;
        let Some(child) = parent.get(key) else {
            return Ok(json!({}));
        };
        value = child;
    }
    if !value.is_object() {
        bail!("JSON 服务表需要映射");
    }
    Ok(value.clone())
}

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(value) => CstInputValue::Bool(*value),
        Value::Number(value) => CstInputValue::Number(value.to_string()),
        Value::String(value) => CstInputValue::String(value.clone()),
        Value::Array(value) => CstInputValue::Array(value.iter().map(input).collect()),
        Value::Object(value) => CstInputValue::Object(
            value
                .iter()
                .map(|(key, value)| (key.clone(), input(value)))
                .collect(),
        ),
    }
}

fn update(object: &CstObject, desired: &serde_json::Map<String, Value>) -> Result<()> {
    for property in object.properties() {
        let name = property.name().context("JSON 属性名称无效")?;
        if !desired.contains_key(name.decoded_value()?.as_str()) {
            property.remove();
        }
    }
    for (key, value) in desired {
        if let Some(property) = object.get(key) {
            if let (Some(object), Some(value)) = (property.object_value(), value.as_object()) {
                update(&object, value)?;
            } else {
                let current = property.value().map(|node| node.to_string());
                let current = current.and_then(|text| parse_value(&text).ok());
                if current.as_ref() != Some(value) {
                    property.set_value(input(value));
                }
            }
        } else {
            object.append(key, input(value));
        }
    }
    Ok(())
}

fn parse_value(text: &str) -> Result<Value> {
    Ok(jsonc_parser::parse_to_serde_value::<Value>(
        text,
        &options(true),
    )?)
}

pub fn render(
    text: &str,
    comments: bool,
    path: &[String],
    name: &str,
    value: Option<&Value>,
) -> Result<String> {
    render_options(text, &options(comments), path, name, value)
}

pub fn render_json5(
    text: &str,
    path: &[String],
    name: &str,
    value: Option<&Value>,
) -> Result<String> {
    render_options(text, &json5_options(), path, name, value)
}

fn render_options(
    text: &str,
    options: &ParseOptions,
    path: &[String],
    name: &str,
    value: Option<&Value>,
) -> Result<String> {
    parse_options(text, options)?;
    let root = document_options(text, options)?;
    let mut object = root
        .object_value_or_create()
        .context("JSON 根节点需要映射")?;
    for key in path {
        object = object
            .object_value_or_create(key)
            .context("JSON 服务配置路径需要映射")?;
    }
    if let Some(value) = value {
        if let Some(desired) = value.as_object() {
            let target = object
                .object_value_or_create(name)
                .context("服务配置需要 JSON 映射")?;
            update(&target, desired)?;
        } else {
            bail!("服务配置需要 JSON 映射");
        }
    } else if let Some(property) = object.get(name) {
        property.remove();
    }
    let mut text = root.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

fn claude_string(input: &str, importing: bool) -> Result<String> {
    let mut output = String::new();
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail.find('}').context("变量引用不完整")?;
        let reference = &tail[..end];
        let name_and_default = if importing {
            reference
        } else {
            reference.strip_prefix("env:").unwrap_or(reference)
        };
        let (name, fallback) = name_and_default
            .split_once(":-")
            .map_or((name_and_default, None), |(name, fallback)| {
                (name, Some(fallback))
            });
        if name.is_empty()
            || !name.bytes().enumerate().all(|(index, c)| {
                c == b'_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit())
            })
            || fallback.is_some_and(|value| value.contains("${"))
        {
            bail!("此变量引用尚不能等价转换，请保留原生配置或使用共享接入");
        }
        if !importing
            && reference == name_and_default
            && matches!(
                name,
                "userHome" | "workspaceFolder" | "workspaceFolderBasename"
            )
        {
            bail!("托管项目变量不能直接写入 Claude Code，请使用共享接入");
        }
        output.push_str(if importing { "${env:" } else { "${" });
        output.push_str(name_and_default);
        output.push('}');
        rest = &tail[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

pub(crate) fn claude_variables(def: &mut Definition, importing: bool) -> Result<()> {
    for value in def
        .command
        .iter_mut()
        .chain(def.url.iter_mut())
        .chain(def.args.iter_mut())
        .chain(def.env.values_mut())
        .chain(def.headers.values_mut())
    {
        *value = claude_string(value, importing)?;
    }
    Ok(())
}

pub fn claude_native(def: &Definition, source: &Source) -> Result<Value> {
    if !def.enabled {
        bail!("Claude Code 不支持服务条目的 enabled 字段，请在原客户端 /mcp 中维护项目启停");
    }
    if def.cwd.is_some()
        || def
            .origin_project
            .as_ref()
            .is_some_and(|project| source.project.as_ref() != Some(project))
    {
        bail!("Claude Code 直连无法等价保留工作目录，请使用共享接入或原项目作用域");
    }
    if def.include_specified
        || !def.include_tools.is_empty()
        || !def.exclude_tools.is_empty()
        || def.resources_disabled
        || def.prompts_disabled
        || def.timeout.is_some()
        || def.startup_timeout.is_some()
    {
        bail!("Claude Code 服务条目无法等价保留当前工具筛选或超时，请使用共享接入");
    }
    if def
        .original_agent
        .as_ref()
        .is_some_and(|agent| *agent != Agent::Claude)
        && def.original.as_object().is_some_and(|map| {
            map.keys()
                .any(|key| !crate::config::FIELDS.contains(&key.as_str()))
        })
    {
        bail!("来源含 Claude Code 尚未适配的字段，请使用共享接入");
    }
    if def.url.as_ref().is_some_and(|value| value.contains("${"))
        || def.headers.values().any(|value| value.contains("${"))
    {
        bail!("Claude Code 远程变量过滤与托管层不同，请使用共享接入");
    }
    let mut encoded = def.clone();
    claude_variables(&mut encoded, false)?;
    let def = &encoded;
    let mut value = if def.original_agent == Some(Agent::Claude) {
        def.original.clone()
    } else {
        json!({})
    };
    let map = value.as_object_mut().context("服务配置格式无效")?;
    for key in crate::config::FIELDS {
        map.remove(*key);
    }
    if let Some(command) = &def.command {
        map.insert("type".into(), json!("stdio"));
        map.insert("command".into(), json!(command));
        map.insert("args".into(), json!(def.args));
        if !def.env.is_empty() {
            map.insert("env".into(), json!(def.env));
        }
    } else if let Some(url) = &def.url {
        map.insert("type".into(), json!("http"));
        map.insert("url".into(), json!(url));
        if !def.headers.is_empty() {
            map.insert("headers".into(), json!(def.headers));
        }
        if let Some(client) = &def.client_id {
            map.insert("oauth".into(), json!({"clientId":client}));
        }
    }
    Ok(value)
}
