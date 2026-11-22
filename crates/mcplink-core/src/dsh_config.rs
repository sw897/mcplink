//! DeepSeek Harness static MCP plugin rows in a profile's Cordis patch layer.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use serde_yaml::Value as Yaml;
use std::str::FromStr;

const CLIENT: &str = "@deepseek-ai/dsh-mcp-client";
const MANAGER: &str = "@deepseek-ai/dsh-mcp-manager";

fn is_manager(row: &Yaml) -> bool {
    row["name"].as_str() == Some(MANAGER)
        || (row["name"].is_null() && matches!(row["id"].as_str(), Some("mcp" | "mcp-manager")))
}

fn entries(text: &str) -> Result<Vec<Yaml>> {
    let value: Yaml = serde_yaml::from_str(text)
        .map_err(|_| anyhow::anyhow!("DeepSeek Harness YAML 无法解析"))?;
    match value {
        Yaml::Null => Ok(Vec::new()),
        Yaml::Sequence(rows) => Ok(rows),
        _ => bail!("DeepSeek Harness profile 需要 YAML 序列"),
    }
}

fn plugin_name(row: &Yaml) -> Option<&str> {
    (row["name"].as_str() == Some(CLIENT))
        .then(|| row["config"]["serverName"].as_str())
        .flatten()
}

fn rows(entries: &[Yaml]) -> impl Iterator<Item = (usize, Option<usize>, &Yaml)> {
    entries.iter().enumerate().flat_map(|(index, row)| {
        std::iter::once((index, None, row)).chain(
            row["insert"]
                .as_sequence()
                .into_iter()
                .flatten()
                .enumerate()
                .map(move |(child, row)| (index, Some(child), row)),
        )
    })
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 32
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        bail!("DeepSeek Harness 服务名需要 1–32 个英文字母、数字、下划线或连字符")
    }
    Ok(())
}

fn disabled(row: &Yaml) -> Result<bool> {
    match &row["disabled"] {
        Yaml::Null => Ok(false),
        Yaml::Bool(value) => Ok(*value),
        _ => bail!("DeepSeek MCP 插件 disabled 需要布尔值"),
    }
}

pub fn parse(text: &str) -> Result<Value> {
    let entries = entries(text)?;
    let mut servers = serde_json::Map::new();
    for (_, _, row) in rows(&entries) {
        if is_manager(row) && !row["config"]["servers"].is_null() {
            let configured = row["config"]["servers"]
                .as_mapping()
                .context("DeepSeek MCP manager servers 需要映射")?;
            for (name, config) in configured {
                let name = name.as_str().context("DeepSeek MCP 名称需要字符串")?;
                check_name(name)?;
                if servers.contains_key(name) {
                    bail!("DeepSeek Harness profile 存在重复 MCP 名称")
                }
                let mut config = serde_json::to_value(config)
                    .unwrap_or_else(|_| json!({"unsupportedNativeExpression":true}));
                let map = config
                    .as_object_mut()
                    .context("DeepSeek MCP 服务需要映射")?;
                let enabled = map
                    .get("enabled")
                    .map(|value| value.as_bool().context("DeepSeek MCP enabled 需要布尔值"))
                    .transpose()?
                    .unwrap_or(true)
                    && !disabled(row)?;
                map.insert("enabled".into(), json!(enabled));
                servers.insert(name.into(), config);
            }
        }
        let Some(name) = plugin_name(row) else {
            continue;
        };
        check_name(name)?;
        if servers.contains_key(name) {
            bail!("DeepSeek Harness profile 存在重复 MCP 名称")
        }
        let mut config = serde_json::to_value(&row["config"])
            .unwrap_or_else(|_| json!({"unsupportedNativeExpression":true}));
        let map = config
            .as_object_mut()
            .context("DeepSeek Harness MCP config 需要映射")?;
        map.remove("serverName");
        map.insert("enabled".into(), json!(!disabled(row)?));
        servers.insert(name.to_owned(), config);
    }
    Ok(json!({"mcp_servers":servers}))
}

fn require_static(node: &yaml_edit::YamlNode) -> Result<()> {
    use yaml_edit::{AsYaml, SyntaxKind};
    let syntax = node.as_node().context("无法检查原生 YAML 条目")?;
    let dynamic = |kind| {
        matches!(
            kind,
            SyntaxKind::ANCHOR | SyntaxKind::TAG | SyntaxKind::ALIAS
        )
    };
    anyhow::ensure!(
        !syntax
            .descendants_with_tokens()
            .any(|part| dynamic(part.kind()))
            && !syntax.ancestors().any(|ancestor| ancestor
                .children_with_tokens()
                .any(|part| part.as_token().is_some_and(|token| dynamic(token.kind())))),
        "此 DeepSeek Harness 条目含动态 YAML、锚点或别名，请在原客户端维护"
    );
    Ok(())
}

fn update_mapping(mapping: &yaml_edit::Mapping, previous: &Yaml, desired: &Value) -> Result<()> {
    let desired = desired.as_object().context("MCP 配置需要映射")?;
    for key in previous
        .as_mapping()
        .into_iter()
        .flatten()
        .filter_map(|(key, _)| key.as_str())
    {
        if !desired.contains_key(key) {
            mapping.remove(key);
        }
    }
    for (key, value) in desired {
        let old = &previous[key];
        if serde_yaml::to_value(value)? == *old {
            continue;
        }
        if value.is_object()
            && let Some(child) = mapping.get_mapping(key.as_str())
        {
            update_mapping(&child, old, value)?;
        } else {
            mapping.set(
                key.as_str(),
                yaml_edit::Document::from_str(&serde_yaml::to_string(value)?)?,
            );
        }
    }
    Ok(())
}

pub fn render(text: &str, name: &str, value: Option<&Value>) -> Result<String> {
    check_name(name)?;
    let entries = entries(text)?;
    parse(text)?;
    let manager = rows(&entries).find(|(_, _, row)| {
        is_manager(row)
            && row["config"]["servers"]
                .as_mapping()
                .is_some_and(|map| map.contains_key(Yaml::from(name)))
    });
    if let Some((index, child, row)) = manager {
        if row["disabled"].as_bool() == Some(true) {
            bail!("MCP manager 插件整体已停用，请在原客户端启用")
        }
        let document = yaml_edit::Document::from_str(text)?;
        let root = document
            .as_sequence()
            .context("DeepSeek Harness profile 需要序列")?;
        let node = root.get(index).context("MCP manager 已变化")?;
        let mut mapping = node.as_mapping().context("MCP manager 需要映射")?.clone();
        if let Some(child) = child {
            mapping = mapping
                .get_sequence("insert")
                .and_then(|seq| seq.get(child))
                .and_then(|node| node.as_mapping().cloned())
                .context("MCP manager insert 已变化")?;
        }
        let servers = mapping
            .get_mapping("config")
            .and_then(|map| map.get_mapping("servers"))
            .context("MCP manager servers 需要映射")?;
        let existing = servers.get(name).context("MCP 定义已变化")?;
        require_static(&existing)?;
        if let Some(value) = value {
            update_mapping(
                existing.as_mapping().context("MCP 服务需要映射")?,
                &row["config"]["servers"][name],
                value,
            )?;
        } else {
            servers.remove(name).context("MCP 定义已变化")?;
        }
        let mut output = document.to_string();
        if !output.ends_with('\n') {
            output.push('\n');
        }
        parse(&output)?;
        return Ok(output);
    }
    let matches: Vec<_> = rows(&entries)
        .filter(|(_, _, row)| plugin_name(row) == Some(name))
        .collect();
    if matches.len() > 1 {
        bail!("DeepSeek Harness profile 存在重复 MCP 名称")
    }
    let existing = matches.first().copied();
    if value.is_none() && existing.is_none() {
        return Ok(text.to_owned());
    }
    let document =
        yaml_edit::Document::from_str(if text.trim().is_empty() { "[]\n" } else { text })
            .map_err(|_| anyhow::anyhow!("DeepSeek Harness YAML 无法编辑"))?;
    let sequence = document
        .as_sequence()
        .context("DeepSeek Harness profile 需要 YAML 序列")?;
    let mut destination = sequence.clone();
    let index = if let Some((index, child, _)) = existing {
        if let Some(child) = child {
            destination = sequence
                .get(index)
                .and_then(|node| node.as_mapping().cloned())
                .and_then(|node| node.get_sequence("insert"))
                .context("原生 insert 结构无法编辑")?;
            child
        } else {
            index
        }
    } else {
        sequence.values().count()
    };
    let original = existing
        .map(|_| destination.get(index).context("原生条目已变化"))
        .transpose()?;
    if let Some(original) = &original {
        require_static(original)?;
    }
    if let Some(value) = value {
        let id = existing
            .and_then(|(_, _, row)| row["id"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("mcplink-mcp-{name}"));
        if rows(&entries)
            .any(|(_, _, row)| row["id"].as_str() == Some(&id) && plugin_name(row) != Some(name))
        {
            bail!("DeepSeek Harness 原生插件 ID 已被占用")
        }
        let mut config = value.clone();
        let config_map = config.as_object_mut().context("MCP 服务需要映射")?;
        let enabled = config_map
            .remove("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        config_map.insert("serverName".into(), json!(name));
        if let (Some(original), Some((_, _, row))) = (&original, existing) {
            let mapping = original.as_mapping().context("MCP 插件条目需要映射")?;
            let live_config = mapping.get_mapping("config").context("MCP 配置需要映射")?;
            update_mapping(&live_config, &row["config"], &config)?;
            if enabled && disabled(row)? {
                mapping.set("disabled", false);
            } else if !enabled && !disabled(row)? {
                mapping.set("disabled", true);
            }
        } else {
            let mut row = json!({"id":id,"name":CLIENT,"config":config});
            if !enabled {
                row["disabled"] = json!(true);
            }
            let replacement = json!({"insert":[row]});
            sequence.push(yaml_edit::Document::from_str(&serde_yaml::to_string(
                &replacement,
            )?)?);
        }
    } else {
        destination.remove(index).context("原生条目已变化")?;
        if let Some((parent, Some(_), _)) = existing
            && destination.is_empty()
        {
            let wrapper = sequence.get(parent).context("原生 insert 已变化")?;
            let mapping = wrapper.as_mapping().context("原生 insert 需要映射")?;
            if mapping.len() == 1 {
                sequence.remove(parent).context("原生 insert 已变化")?;
            } else {
                mapping.remove("insert");
            }
        }
    }
    let mut output = document.to_string();
    if !output.ends_with('\n') {
        output.push('\n');
    }
    parse(&output)?;
    Ok(output)
}
