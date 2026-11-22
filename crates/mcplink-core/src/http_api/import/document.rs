use anyhow::{Context, Result, bail, ensure};
use saphyr_parser::{Event, Parser, ScalarStyle};
use serde_json::{Map, Number, Value};

const MAX_DEPTH: usize = 64;
const MAX_EVENTS: usize = 100_000;

enum Container {
    Sequence(Vec<Value>),
    Mapping {
        fields: Map<String, Value>,
        key: Option<String>,
    },
}

fn append(stack: &mut [Container], root: &mut Option<Value>, value: Value) -> Result<()> {
    match stack.last_mut() {
        Some(Container::Sequence(items)) => items.push(value),
        Some(Container::Mapping { fields, key }) => {
            let key = key.take().context("文档映射键需要标量")?;
            ensure!(fields.insert(key, value).is_none(), "文档含重复映射键");
        }
        None => ensure!(root.replace(value).is_none(), "仅支持单个 OpenAPI 文档"),
    }
    Ok(())
}

fn scalar(text: String, style: ScalarStyle) -> Value {
    if style != ScalarStyle::Plain || text.len() > 64 {
        return Value::String(text);
    }
    match text.as_str() {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    let integer = if let Some(hex) = text.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok().map(Number::from)
    } else if let Some(octal) = text.strip_prefix("0o") {
        u64::from_str_radix(octal, 8).ok().map(Number::from)
    } else {
        text.parse::<i64>()
            .ok()
            .map(Number::from)
            .or_else(|| text.parse::<u64>().ok().map(Number::from))
    };
    let number = integer.or_else(|| {
        text.contains(['.', 'e', 'E'])
            .then(|| text.parse::<f64>().ok().and_then(Number::from_f64))
            .flatten()
    });
    number.map_or(Value::String(text), Value::Number)
}

pub(super) fn parse(source: &str) -> Result<Value> {
    let mut stack = Vec::new();
    let mut root = None;
    let mut has_document = false;
    for (index, event) in Parser::new_from_str(source).enumerate() {
        ensure!(index < MAX_EVENTS, "导入文档超过节点容量限制");
        let (event, _) =
            event.map_err(|_| anyhow::anyhow!("文档不是有效的 JSON/YAML；错误内容未回显"))?;
        match event {
            Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {}
            Event::DocumentStart(_) => {
                ensure!(!has_document, "仅支持单个 OpenAPI 文档");
                has_document = true;
            }
            Event::SequenceStart(0, None) | Event::MappingStart(0, None) => {
                ensure!(stack.len() < MAX_DEPTH, "导入文档超过嵌套深度限制");
                let container = if matches!(event, Event::SequenceStart(..)) {
                    Container::Sequence(Vec::new())
                } else {
                    Container::Mapping {
                        fields: Map::new(),
                        key: None,
                    }
                };
                stack.push(container);
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let value = match stack.pop() {
                    Some(Container::Sequence(items)) if matches!(event, Event::SequenceEnd) => {
                        Value::Array(items)
                    }
                    Some(Container::Mapping { fields, key: None })
                        if matches!(event, Event::MappingEnd) =>
                    {
                        Value::Object(fields)
                    }
                    _ => bail!("导入文档容器格式无效"),
                };
                append(&mut stack, &mut root, value)?;
            }
            Event::Scalar(text, style, 0, None) => {
                if let Some(Container::Mapping { key, .. }) = stack.last_mut()
                    && key.is_none()
                {
                    ensure!(
                        style != ScalarStyle::Plain || text != "<<",
                        "OpenAPI YAML 不支持合并键"
                    );
                    *key = Some(text.into_owned());
                } else {
                    append(&mut stack, &mut root, scalar(text.into_owned(), style))?;
                }
            }
            _ => bail!("OpenAPI YAML 不支持锚点、别名或显式标签，请先展开为普通文档"),
        }
    }
    ensure!(stack.is_empty(), "导入文档未结束");
    root.context("导入文档为空")
}
