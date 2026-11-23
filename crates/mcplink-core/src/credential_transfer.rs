//! Explicit, same-connection credential relocation without a plaintext export.
use crate::model::Definition;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
enum Field {
    Env,
    Headers,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Slot {
    field: Field,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    from: Slot,
    to: Slot,
}

pub(crate) fn apply(
    request: &Value,
    previous: &Definition,
    next: &mut Definition,
    confirmed_origin: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let mappings: Vec<Mapping> = serde_json::from_value(request.clone())
        .map_err(|_| anyhow::anyhow!("凭据字段迁移声明无效"))?;
    ensure!(
        !mappings.is_empty() && mappings.len() <= 32,
        "凭据迁移需要 1–32 个字段"
    );
    let (from, to) = match (previous.url.is_some(), next.url.is_some()) {
        (true, true) => {
            ensure!(
                !next.changed_origin(previous),
                "更改服务来源时不能迁移旧凭据"
            );
            (Field::Headers, Field::Headers)
        }
        (false, false) => {
            ensure!(
                !next.changed_origin(previous),
                "更改启动程序时不能迁移旧凭据"
            );
            (Field::Env, Field::Env)
        }
        (false, true) if next.http_api.is_some() => {
            let origin = next.http_origin()?;
            ensure!(
                confirmed_origin == Some(origin.as_str()),
                "从本地进程迁移已存环境变量需要在桌面确认目标服务"
            );
            (Field::Env, Field::Headers)
        }
        _ => anyhow::bail!("所选传输之间不能迁移旧凭据，请填写新的认证字段"),
    };
    let mut destinations = HashSet::new();
    for mapping in &mappings {
        ensure!(
            mapping.from.field == from && mapping.to.field == to,
            "凭据映射与连接传输不符"
        );
        for slot in [&mapping.from, &mapping.to] {
            ensure!(
                !slot.name.is_empty()
                    && slot.name.len() <= 256
                    && !slot.name.chars().any(|c| c.is_control() || c == '='),
                "凭据字段名称无效"
            );
            if slot.field == Field::Headers {
                crate::http_api::checked_header_name(&slot.name)?;
            }
        }
        let identity = if mapping.to.field == Field::Headers {
            mapping.to.name.to_ascii_lowercase()
        } else {
            mapping.to.name.clone()
        };
        ensure!(
            destinations.insert((mapping.to.field, identity)),
            "凭据迁移目标重复"
        );
        let source = match mapping.from.field {
            Field::Env => &previous.env,
            Field::Headers => &previous.headers,
        };
        let value = source
            .iter()
            .find(|(key, _)| {
                if mapping.from.field == Field::Headers {
                    key.eq_ignore_ascii_case(&mapping.from.name)
                } else {
                    *key == &mapping.from.name
                }
            })
            .map(|(_, value)| value)
            .filter(|value| !value.is_empty() && value.as_str() != "••••••••")
            .context("原连接没有可迁移的凭据字段")?;
        let target = match mapping.to.field {
            Field::Env => &mut next.env,
            Field::Headers => &mut next.headers,
        };
        let name = if mapping.to.field == Field::Headers {
            target
                .keys()
                .find(|key| key.eq_ignore_ascii_case(&mapping.to.name))
                .cloned()
                .unwrap_or_else(|| mapping.to.name.clone())
        } else {
            mapping.to.name.clone()
        };
        ensure!(
            target.get(&name).is_none_or(|s| s == "••••••••"),
            "凭据迁移不能覆盖同时填写的新值"
        );
        target.insert(name, value.clone());
    }
    Ok(mappings
        .into_iter()
        .map(|mapping| (mapping.from.name, mapping.to.name))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn protected_fields_move_without_touching_the_original_or_accepting_public_destinations() {
        let old = Definition {
            env: [("KEY".into(), "synthetic-private-value".into())].into(),
            ..Default::default()
        };
        let mut next = Definition::default();
        apply(&json!([{"from":{"field":"env","name":"KEY"},"to":{"field":"env","name":"CREDENTIAL"}}]), &old, &mut next, None).unwrap();
        assert_eq!(next.env["CREDENTIAL"], old.env["KEY"]);
        assert!(
            apply(
                &json!([{"from":{"field":"env","name":"KEY"},"to":{"field":"url","name":"value"}}]),
                &old,
                &mut next,
                None
            )
            .is_err()
        );
        assert!(apply(&json!([{"from":{"field":"env","name":"MISSING"},"to":{"field":"env","name":"Other"}}]), &old, &mut next, None).is_err());
    }
}
