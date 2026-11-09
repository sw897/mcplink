//! Native MCP serializers. Mutations use the shared preview/apply/restore transaction.
use crate::{
    config, json_config,
    model::{Agent, Definition, Source, digest},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn directory(agent: &Agent) -> Result<PathBuf> {
    let home = config::user_home()?;
    if *agent == Agent::Workbuddy {
        return Ok(home.join(".workbuddy"));
    }
    let (variable, folder) = match agent {
        Agent::Zcode => ("ZCODE_HOME", ".zcode"),
        Agent::Deepseek => ("DSH_HOME", ".dsh"),
        Agent::Openclaw => ("OPENCLAW_STATE_DIR", ".openclaw"),
        Agent::Kimicode => ("KIMI_CODE_HOME", ".kimi-code"),
        _ => bail!("客户端未适配"),
    };
    Ok(std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(folder)))
}

fn source(
    agent: Agent,
    path: PathBuf,
    label: &str,
    project: Option<PathBuf>,
    read_only: bool,
) -> Source {
    #[cfg(windows)]
    let path = PathBuf::from(path.to_string_lossy().replace('/', "\\"));
    let path = config::canonical_path(&path).unwrap_or(path);
    let read_only = read_only
        || (agent == Agent::Deepseek && path.file_name().is_some_and(|name| name == "cordis.yml"));
    let id = format!(
        "native:{}:{}",
        serde_json::to_value(&agent).unwrap().as_str().unwrap(),
        digest(path.to_string_lossy().as_bytes())
    );
    Source {
        id,
        agent,
        label: label.into(),
        path,
        scope: Some(if project.is_some() { "project" } else { "user" }.into()),
        project,
        read_only,
        plugin_root: None,
        discovery_error: None,
    }
}

pub fn default_path(agent: &Agent) -> Result<PathBuf> {
    let root = directory(agent)?;
    Ok(match agent {
        Agent::Zcode => root.join("cli").join("config.json"),
        Agent::Deepseek => root.join("profiles/desktop/cordis.patch.yml"),
        Agent::Openclaw => std::env::var_os("OPENCLAW_CONFIG_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("openclaw.json")),
        Agent::Kimicode | Agent::Workbuddy => root.join("mcp.json"),
        _ => unreachable!(),
    })
}

pub fn default_sources() -> Result<Vec<Source>> {
    let mut result = Vec::new();
    for agent in [
        Agent::Zcode,
        Agent::Openclaw,
        Agent::Kimicode,
        Agent::Workbuddy,
    ] {
        result.push(source(
            agent.clone(),
            default_path(&agent)?,
            "用户 MCP",
            None,
            false,
        ));
    }
    let root = directory(&Agent::Deepseek)?;
    for profile in ["desktop", "web", "headless", "acp"] {
        let path = root.join("profiles").join(profile).join("cordis.patch.yml");
        if matches!(profile, "desktop" | "web") || path.is_file() {
            result.push(source(
                Agent::Deepseek,
                path,
                &format!("{profile} profile MCP"),
                None,
                false,
            ));
        }
        let base = root.join("profiles").join(profile).join("cordis.yml");
        if base.is_file() {
            result.push(source(
                Agent::Deepseek,
                base,
                &format!("{profile} 基础组合（只读）"),
                None,
                true,
            ));
        }
    }
    let fallback = config::user_home()?.join(".agents/mcp.json");
    if fallback.is_file() && !native_has_servers(&default_path(&Agent::Zcode)?)? {
        result.push(source(
            Agent::Zcode,
            fallback,
            ".agents 回退 MCP（只读）",
            None,
            true,
        ));
    }
    Ok(result)
}

fn native_has_servers(path: &Path) -> Result<bool> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Ok(true),
    };
    if text.len() > 4 * 1024 * 1024 {
        return Ok(true);
    }
    Ok(json_config::parse(&text, true)
        .and_then(|v| json_config::table(&v, &["mcp".into(), "servers".into()]))
        .map(|v| !v.as_object().unwrap().is_empty())
        .unwrap_or(true))
}

pub fn source_from_params(agent: Agent, params: &Value, sources: &[Source]) -> Result<Source> {
    let scope = params["scope"].as_str().unwrap_or("user");
    if !matches!(scope, "user" | "project") {
        bail!("此客户端支持 user 或 project 来源")
    }
    let project = params["project"]
        .as_str()
        .map(PathBuf::from)
        .map(|path| {
            if !path.is_absolute() || !path.is_dir() {
                bail!("请选择已存在的完整项目目录")
            }
            config::canonical_path(&path)
        })
        .transpose()?;
    if (scope == "project") != project.is_some()
        || (project.is_some()
            && !matches!(agent, Agent::Zcode | Agent::Kimicode | Agent::Workbuddy))
    {
        bail!("仅 ZCode、Kimi Code 和 WorkBuddy 支持显式项目来源，且需要项目目录")
    }
    let default = if let Some(root) = &project {
        root.join(match agent {
            Agent::Zcode => ".zcode/config.json",
            Agent::Workbuddy => ".workbuddy/mcp.json",
            _ => ".kimi-code/mcp.json",
        })
    } else {
        default_path(&agent)?
    };
    let expected = default.clone();
    let path = params["path"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or(default);
    if !path.is_absolute() {
        bail!("请选择完整的原生配置文件路径")
    }
    let path = config::canonical_path(&path).unwrap_or(path);
    if project.is_some() && path != config::canonical_path(&expected).unwrap_or(expected) {
        bail!("项目配置路径与所选项目不符")
    }
    if let Some(existing) = sources
        .iter()
        .find(|s| config::canonical_path(&s.path).unwrap_or(s.path.clone()) == path)
        && (existing.agent != agent || existing.read_only || existing.project == project)
    {
        return Ok(existing.clone());
    }
    let value = source(
        agent,
        path,
        params["label"].as_str().unwrap_or("显式 MCP 来源"),
        project,
        false,
    );
    config::read(&value)?;
    Ok(value)
}

fn table_path(source: &Source) -> Vec<String> {
    if source.agent == Agent::Openclaw
        || (source.agent == Agent::Zcode
            && source
                .path
                .file_name()
                .is_none_or(|name| name != "mcp.json"))
    {
        vec!["mcp".into(), "servers".into()]
    } else {
        vec!["mcpServers".into()]
    }
}

pub fn parse(source: &Source, text: &str) -> Result<Value> {
    if source.agent == Agent::Deepseek {
        return crate::dsh_config::parse(text);
    }
    let value = if source.agent == Agent::Openclaw {
        json_config::parse_json5(text)?
    } else {
        json_config::parse(text, true)?
    };
    if value.get("$include").is_some() || value.pointer("/mcp/$include").is_some() {
        bail!("MCP 来源使用外部包含，请显式登记实际配置文件")
    }
    Ok(json!({"mcp_servers":json_config::table(&value,&table_path(source))?}))
}

pub fn render(source: &Source, text: &str, name: &str, value: Option<&Value>) -> Result<String> {
    if source.agent == Agent::Openclaw && name == "__proto__" {
        bail!("OpenClaw 保留此服务名称")
    }
    if source.agent == Agent::Deepseek {
        return crate::dsh_config::render(text, name, value);
    }
    parse(source, text)?;
    if source.agent == Agent::Zcode
        && source.path == default_path(&Agent::Zcode)?
        && value.is_some()
        && !native_has_servers(&source.path)?
    {
        let fallback = config::user_home()?.join(".agents/mcp.json");
        if fallback.is_file() {
            let raw = json_config::parse(
                &std::fs::read_to_string(fallback).context("ZCode 回退配置无法读取")?,
                true,
            )?;
            if !json_config::table(&raw, &["mcpServers".into()])?
                .as_object()
                .unwrap()
                .is_empty()
            {
                bail!("请先在 ZCode 将非空 .agents 回退配置合并到原生配置，避免新入口遮蔽现有服务")
            }
        }
    }
    if source.agent == Agent::Openclaw {
        json_config::render_json5(text, &table_path(source), name, value)
    } else {
        json_config::render(text, true, &table_path(source), name, value)
    }
}

pub fn normalize(raw: &Value, agent: &Agent, project: Option<&Path>) -> Result<Definition> {
    let fields = raw.as_object().context("MCP 服务需要映射")?;
    let allowed = match agent {
        Agent::Workbuddy => vec![
            "type",
            "command",
            "args",
            "env",
            "cwd",
            "url",
            "headers",
            "timeout",
            "disabledTools",
        ],
        Agent::Zcode => vec![
            "type", "command", "args", "env", "cwd", "url", "headers", "enable",
        ],
        Agent::Openclaw => vec![
            "command",
            "args",
            "env",
            "cwd",
            "url",
            "headers",
            "transport",
            "enabled",
            "connectionTimeoutMs",
            "requestTimeoutMs",
            "toolFilter",
        ],
        Agent::Kimicode => vec![
            "command",
            "args",
            "env",
            "cwd",
            "url",
            "headers",
            "transport",
            "enabled",
            "bearerTokenEnvVar",
            "startupTimeoutMs",
            "toolTimeoutMs",
            "enabledTools",
            "disabledTools",
            "environment_id",
        ],
        Agent::Deepseek => vec![
            "command",
            "args",
            "env",
            "cwd",
            "url",
            "headers",
            "transport",
            "enabled",
            "startupTimeoutMs",
            "toolCallTimeoutMs",
        ],
        _ => bail!("客户端未适配"),
    };
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        bail!("原生执行选项尚不能等价托管，请保留原生配置或显式建立共享连接")
    }
    for key in [
        "command",
        "url",
        "type",
        "transport",
        "cwd",
        "environment_id",
        "bearerTokenEnvVar",
    ] {
        if raw.get(key).is_some_and(|value| !value.is_string()) {
            bail!("原生 MCP 字符串字段类型无效")
        }
    }
    for key in ["enabledTools", "disabledTools"] {
        if raw.get(key).is_some_and(|value| {
            !value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        }) {
            bail!("原生工具过滤需要字符串数组")
        }
    }
    if let Some(variable) = raw["bearerTokenEnvVar"].as_str()
        && (variable.is_empty()
            || !variable.bytes().enumerate().all(|(index, c)| {
                c == b'_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit())
            }))
    {
        bail!("凭据环境变量名称无效")
    }
    if raw["environment_id"].as_str().is_some_and(|v| v != "local") {
        bail!("远程运行环境不能作为本机 MCP 执行")
    }
    let mut value = raw.clone();
    let map = value.as_object_mut().unwrap();
    let transport = raw["transport"].as_str().or_else(|| raw["type"].as_str());
    let transport = transport.map(|v| {
        if *agent == Agent::Workbuddy && v == "streamableHttp" {
            "streamable-http"
        } else {
            v
        }
    });
    if transport.is_some_and(|v| !matches!(v, "stdio" | "http" | "streamable-http"))
        || (transport == Some("stdio") && raw.get("url").is_some())
        || (transport.is_some_and(|v| v != "stdio") && raw.get("command").is_some())
    {
        bail!("此 MCP 传输不能等价托管，请使用 stdio 或 Streamable HTTP")
    }
    map.remove("type");
    map.remove("transport");
    map.remove("environment_id");
    let enabled = raw.get(if *agent == Agent::Zcode {
        "enable"
    } else {
        "enabled"
    });
    if enabled.is_some_and(|v| !v.is_boolean()) {
        bail!("MCP 启用状态需要布尔值")
    }
    map.remove("enable");
    map.insert(
        "enabled".into(),
        json!(enabled.and_then(Value::as_bool).unwrap_or(true)),
    );
    if *agent == Agent::Workbuddy
        && let Some(value) = map.remove("timeout")
    {
        let millis = value.as_u64().context("原生连接超时需要正整数毫秒")?;
        if millis == 0 || millis % 1000 != 0 {
            bail!("毫秒连接超时不能等价转换为托管秒数，请显式建立共享连接")
        }
        map.insert("startup_timeout_sec".into(), json!(millis / 1000));
    }
    for (native, canonical) in [
        ("connectionTimeoutMs", "startup_timeout_sec"),
        ("startupTimeoutMs", "startup_timeout_sec"),
        ("requestTimeoutMs", "tool_timeout_sec"),
        ("toolTimeoutMs", "tool_timeout_sec"),
        ("toolCallTimeoutMs", "tool_timeout_sec"),
    ] {
        if let Some(value) = map.remove(native) {
            let millis = value.as_u64().context("原生超时需要正整数毫秒")?;
            if millis == 0
                || millis % 1000 != 0
                || (*agent == Agent::Kimicode && millis > 2_147_483_647)
            {
                bail!("毫秒超时不能等价转换为托管秒数，请显式建立共享连接")
            }
            map.insert(canonical.into(), json!(millis / 1000));
        }
    }
    if let Some(v) = map.remove("enabledTools") {
        map.insert("enabled_tools".into(), v);
    }
    if let Some(v) = map.remove("disabledTools") {
        map.insert("disabled_tools".into(), v);
    }
    if let Some(v) = map.remove("toolFilter") {
        let filter = v.as_object().context("toolFilter 需要映射")?;
        if filter
            .keys()
            .any(|key| !matches!(key.as_str(), "include" | "exclude"))
        {
            bail!("未知的原生工具过滤选项")
        }
        if filter.values().any(|value| {
            !value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        }) {
            bail!("原生工具过滤需要字符串数组")
        }
        if let Some(v) = filter.get("include") {
            map.insert("enabled_tools".into(), v.clone());
        }
        if let Some(v) = filter.get("exclude") {
            map.insert("disabled_tools".into(), v.clone());
        }
    }
    if let Some(v) = map.remove("bearerTokenEnvVar") {
        map.insert("bearer_token_env_var".into(), v);
    }
    if value.to_string().contains("${") || value.to_string().contains("{env:") {
        bail!("原生变量或凭据模板请在原客户端维护，或配置托管凭据后使用共享接入")
    }
    let mut def = config::normalize(&value, Agent::Codex)?;
    if def
        .cwd
        .as_deref()
        .is_some_and(|cwd| !Path::new(cwd).is_absolute())
        && project.is_none()
    {
        bail!("相对工作目录需要显式项目来源")
    }
    def.original = raw.clone();
    def.original_agent = Some(agent.clone());
    def.origin_project = project.map(Path::to_path_buf);
    Ok(def)
}

pub fn native(def: &Definition, source: &Source) -> Result<Value> {
    config::validate(def)?;
    if source.agent == Agent::Workbuddy {
        crate::client_import::check_portable(def)?;
        if def.original_agent == Some(Agent::Hermes)
            && def
                .exclude_tools
                .iter()
                .any(|v| v.contains(['*', '?', '[']))
        {
            bail!("WorkBuddy 无等价的工具通配符过滤，请使用共享代理")
        }
    }
    if def.command.is_some()
        && def
            .origin_project
            .as_ref()
            .is_some_and(|project| source.project.as_ref() != Some(project))
    {
        bail!("项目命令不能直接迁移到其他运行目录，请使用共享代理")
    }
    if def.oauth
        || def.client_id.is_some()
        || def.client_secret.is_some()
        || !def.oauth_scopes.is_empty()
        || def.oauth_callback_port.is_some()
    {
        bail!("此客户端 OAuth 请使用共享代理，由 mcplink 维护认证")
    }
    if def.resources_disabled || def.prompts_disabled {
        bail!("原生资源或提示词过滤请使用共享代理")
    }
    if source.agent == Agent::Workbuddy
        && (!def.enabled
            || def.timeout.is_some()
            || def.include_specified
            || !def.include_tools.is_empty())
    {
        bail!(
            "WorkBuddy 无等价的启停、请求超时或工具白名单字段，请在原客户端维护启停，或使用共享代理保留限制"
        )
    }
    if matches!(source.agent, Agent::Zcode | Agent::Deepseek)
        && (def.include_specified || !def.exclude_tools.is_empty())
    {
        bail!("此客户端原生工具过滤请使用共享代理")
    }
    let mut value = json!({});
    if let Some(command) = &def.command {
        value["command"] = json!(command);
        value["args"] = json!(def.args);
        if !def.env.is_empty() {
            value["env"] = json!(def.env);
        }
        if let Some(cwd) = &def.cwd {
            value["cwd"] = json!(cwd);
        }
    } else {
        value["url"] = json!(def.url);
        if !def.headers.is_empty() {
            value["headers"] = json!(def.headers);
        }
    }
    if value.to_string().contains("${") {
        bail!("托管变量不能直接写入此客户端，请使用共享代理")
    }
    if source.agent == Agent::Workbuddy {
        value["type"] = json!(if def.url.is_some() {
            "streamableHttp"
        } else {
            "stdio"
        });
        if let Some(seconds) = def.startup_timeout {
            let millis = seconds
                .checked_mul(1000)
                .filter(|v| *v > 0)
                .context("连接超时超出范围")?;
            value["timeout"] = json!(millis);
        }
        if !def.exclude_tools.is_empty() {
            value["disabledTools"] = json!(def.exclude_tools);
        }
        return Ok(value);
    }
    if source.agent == Agent::Zcode {
        value["type"] = json!(if def.url.is_some() { "http" } else { "stdio" });
        value["enable"] = json!(def.enabled);
    } else {
        value["enabled"] = json!(def.enabled);
        if source.agent != Agent::Kimicode {
            value["transport"] = json!(if def.url.is_some() {
                "streamable-http"
            } else {
                "stdio"
            });
        }
    }
    for (timeout, key) in [
        (
            def.startup_timeout,
            if source.agent == Agent::Openclaw {
                "connectionTimeoutMs"
            } else {
                "startupTimeoutMs"
            },
        ),
        (
            def.timeout,
            if source.agent == Agent::Openclaw {
                "requestTimeoutMs"
            } else if source.agent == Agent::Kimicode {
                "toolTimeoutMs"
            } else {
                "toolCallTimeoutMs"
            },
        ),
    ] {
        if let Some(seconds) = timeout {
            if source.agent == Agent::Zcode {
                bail!("ZCode 原生超时请使用共享代理")
            }
            let millis = seconds.checked_mul(1000).context("超时过大")?;
            if millis == 0 || (source.agent == Agent::Kimicode && millis > 2_147_483_647) {
                bail!("超时超出客户端范围")
            }
            value[key] = json!(millis);
        }
    }
    if source.agent == Agent::Kimicode {
        if def.include_specified {
            value["enabledTools"] = json!(def.include_tools);
        }
        if !def.exclude_tools.is_empty() {
            value["disabledTools"] = json!(def.exclude_tools);
        }
    } else if source.agent == Agent::Openclaw {
        if def.include_specified {
            value["toolFilter"]["include"] = json!(def.include_tools);
        }
        if !def.exclude_tools.is_empty() {
            value["toolFilter"]["exclude"] = json!(def.exclude_tools);
        }
    }
    Ok(value)
}

pub fn set_enabled(value: &mut Value, agent: &Agent, enabled: bool) -> Result<()> {
    if *agent == Agent::Workbuddy {
        bail!("WorkBuddy 启停请在原客户端维护；此处可预览移除配置")
    }
    value.as_object_mut().context("MCP 服务需要映射")?.insert(
        if *agent == Agent::Zcode {
            "enable"
        } else {
            "enabled"
        }
        .into(),
        json!(enabled),
    );
    Ok(())
}

pub fn check_import(source: &Source, _raw: &Value) -> Result<()> {
    if source.agent == Agent::Kimicode
        && let Ok(text) = std::fs::read_to_string(directory(&Agent::Kimicode)?.join("config.toml"))
    {
        let value: toml::Value =
            toml::from_str(&text).map_err(|_| anyhow::anyhow!("Kimi Code 配置无法解析"))?;
        if value.get("mcp").is_some() {
            bail!("Kimi Code 全局 MCP 选项请在原客户端维护；可显式建立托管连接避免改变语义")
        }
    }
    Ok(())
}
