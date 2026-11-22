//! Common capability checks before rendering a native MCP entry.
use crate::{
    config,
    model::{Agent, Definition},
};
use anyhow::{Result, ensure};

#[derive(PartialEq, Eq)]
enum Tools {
    None,
    Exact,
    Patterns,
}
#[derive(PartialEq, Eq)]
enum Timing {
    None,
    Shared,
    Separate,
}

pub(crate) fn validate(def: &Definition, target: &Agent, importing: bool) -> Result<()> {
    config::validate(def)?;
    let shared = if importing {
        "共享模式"
    } else {
        "共享接入"
    };
    ensure!(def.http_api.is_none(), "HTTP API Adapter 须使用{shared}");
    if importing
        || def
            .original_agent
            .as_ref()
            .is_some_and(|source| source != target)
    {
        crate::client_import::check_portable(def)?;
    }
    let (tools, timing) = match target {
        _ if importing => (Tools::None, Timing::None),
        Agent::Hermes => (Tools::Patterns, Timing::Separate),
        Agent::Codex | Agent::Openclaw | Agent::Kimicode => (Tools::Exact, Timing::Separate),
        Agent::Opencode | Agent::Deepseek => (Tools::None, Timing::Separate),
        Agent::Workbuddy => (Tools::None, Timing::Shared),
        _ => (Tools::None, Timing::None),
    };
    let has_range =
        def.include_specified || !def.include_tools.is_empty() || !def.exclude_tools.is_empty();
    if tools != Tools::Patterns
        && (tools == Tools::None || def.original_agent.as_ref() != Some(target))
    {
        ensure!(
            !def.include_tools
                .iter()
                .chain(&def.exclude_tools)
                .any(|name| name.contains(['*', '?', '['])),
            "目标客户端无法等价保留工具筛选中的通配符，请使用{shared}"
        );
    }
    ensure!(
        !has_range || tools != Tools::None,
        "目标客户端无法等价保留工具筛选，请使用{shared}"
    );
    ensure!(
        !(def.resources_disabled || def.prompts_disabled) || tools == Tools::Patterns,
        "目标客户端无法等价保留资源或提示词开关，请使用{shared}"
    );
    ensure!(
        *target != Agent::Codex
            || def.original_agent == Some(Agent::Codex)
            || !def.include_specified
            || def.exclude_tools.is_empty(),
        "Codex 直连无法等价保留组合工具筛选，请使用共享接入"
    );
    ensure!(
        match timing {
            Timing::None => def.timeout.is_none() && def.startup_timeout.is_none(),
            Timing::Shared => def.timeout == def.startup_timeout,
            Timing::Separate => true,
        },
        "目标客户端无法等价保留配置的超时，请使用{shared}"
    );
    ensure!(
        def.enabled || !(importing || matches!(target, Agent::Claude | Agent::Workbuddy)),
        "目标客户端的启停在原客户端维护，请使用{shared}保留当前状态"
    );
    let oauth = !importing
        && matches!(
            target,
            Agent::Codex | Agent::Hermes | Agent::Claude | Agent::Opencode
        );
    ensure!(
        def.client_secret.is_none()
            && def.oauth_scopes.is_empty()
            && (def.oauth_callback_port.is_none() || (oauth && *target == Agent::Claude))
            && (oauth || (!def.oauth && def.client_id.is_none())),
        "此连接含无法等价转换的 OAuth 设置，请使用{shared}"
    );
    if importing {
        ensure!(
            def.cwd.is_none() && (def.command.is_none() || def.origin_project.is_none()),
            "客户端导入无法等价保留运行目录，请使用{shared}"
        );
    }
    Ok(())
}
