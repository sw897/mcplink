use crate::{
    config, json_config,
    model::{Agent, Definition, Source, digest},
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) fn directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("OPENCODE_CONFIG_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(config::user_home()?.join(".config"))
        .join("opencode"))
}

fn source_id(path: &Path) -> String {
    format!(
        "opencode:source:{}",
        digest(path.to_string_lossy().as_bytes())
    )
}

pub(crate) fn is_managed(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        path.starts_with("/Library/Application Support/opencode")
            || path.starts_with("/Library/Managed Preferences")
    }
    #[cfg(target_os = "windows")]
    {
        path.starts_with(
            PathBuf::from(
                std::env::var_os("ProgramData").unwrap_or_else(|| r"C:\ProgramData".into()),
            )
            .join("opencode"),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        path.starts_with("/etc/opencode")
    }
}

fn validate_config(raw: &Value) -> Result<()> {
    let Some(mcp) = raw.get("mcp") else {
        return Ok(());
    };
    let mcp = mcp.as_object().context("OpenCode mcp 需要映射")?;
    if mcp
        .keys()
        .any(|key| !matches!(key.as_str(), "servers" | "timeout"))
    {
        bail!("仅支持当前 OpenCode mcp.servers 配置，不支持旧版服务表");
    }
    if let Some(servers) = mcp.get("servers") {
        let servers = servers
            .as_object()
            .context("OpenCode mcp.servers 需要映射")?;
        if matches!(
            servers.get("type").and_then(Value::as_str),
            Some("local" | "remote")
        ) || servers.get("enabled").is_some_and(Value::is_boolean)
        {
            bail!("仅支持当前 OpenCode mcp.servers 服务表，不支持旧版服务条目");
        }
    }
    if let Some(timeouts) = mcp.get("timeout") {
        let timeouts = timeouts
            .as_object()
            .context("OpenCode mcp.timeout 需要映射")?;
        if timeouts
            .keys()
            .any(|key| !matches!(key.as_str(), "startup" | "catalog" | "execution"))
            || timeouts.values().any(|value| {
                value
                    .as_u64()
                    .is_none_or(|millis| millis == 0 || millis > 9_007_199_254_740_991)
            })
        {
            bail!("OpenCode mcp.timeout 需要当前格式的正整数毫秒设置");
        }
    }
    Ok(())
}

pub fn default_sources() -> Result<Vec<Source>> {
    let mut sources = Vec::new();
    for name in ["opencode.json", "opencode.jsonc"] {
        let path = directory()?.join(name);
        if !path.is_file() {
            continue;
        }
        let managed = is_managed(&path);
        sources.push(Source {
            id: source_id(&path),
            agent: Agent::Opencode,
            label: format!("{}配置 · {name}", if managed { "托管" } else { "用户" }),
            path,
            project: None,
            read_only: managed,
            plugin_root: None,
            discovery_error: None,
            scope: Some(if managed { "managed" } else { "user" }.into()),
        });
    }
    if sources.is_empty() {
        sources.push(source_from_params(&json!({"agent":"opencode"}), &[])?);
    }
    Ok(sources)
}

pub fn source_from_params(params: &Value, sources: &[Source]) -> Result<Source> {
    let scope = params
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("user");
    if !matches!(scope, "user" | "project") {
        bail!("OpenCode 来源支持 user 或 project 作用域");
    }
    let project = params
        .get("project")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .map(|path| {
            if !path.is_absolute() || !path.is_dir() {
                bail!("请选择已存在的完整项目目录");
            }
            config::canonical_path(&path)
        })
        .transpose()?;
    if (scope == "project") != project.is_some() {
        bail!("OpenCode project 来源需要显式项目目录；user 来源不接受项目目录");
    }
    if params.get("layout").is_some_and(|value| !value.is_null()) {
        bail!("OpenCode 仅支持当前配置格式，无需指定 layout 参数");
    }
    let base = project.clone().map(Ok).unwrap_or_else(directory)?;
    let supplied = params
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let path = supplied.unwrap_or_else(|| {
        let jsonc = base.join("opencode.jsonc");
        let json = base.join("opencode.json");
        if jsonc.exists() || !json.exists() {
            jsonc
        } else {
            json
        }
    });
    if !path.is_absolute()
        || !path
            .file_name()
            .is_some_and(|name| name == "opencode.json" || name == "opencode.jsonc")
    {
        bail!("请选择完整的 opencode.json 或 opencode.jsonc 路径");
    }
    let path = config::canonical_path(&path).unwrap_or(path);
    if let Some(project) = &project
        && path.parent() != Some(project.as_path())
        && path.parent() != Some(project.join(".opencode").as_path())
    {
        bail!("OpenCode 项目配置须位于所选项目目录或其 .opencode 目录");
    }
    let existing = sources
        .iter()
        .find(|s| config::canonical_path(&s.path).unwrap_or(s.path.clone()) == path);
    if let Some(existing) = existing
        && (existing.agent != Agent::Opencode || existing.read_only)
    {
        return Ok(existing.clone());
    }
    let managed = is_managed(&path);
    let source = Source {
        id: source_id(&path),
        agent: Agent::Opencode,
        label: params
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(scope)
            .into(),
        path,
        project,
        read_only: managed,
        plugin_root: None,
        discovery_error: None,
        scope: Some(if managed { "managed" } else { scope }.into()),
    };
    let text = match std::fs::read_to_string(&source.path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => bail!("OpenCode 配置无法读取"),
    };
    if text.len() > 4 * 1024 * 1024 {
        bail!("配置超过 4 MiB");
    }
    validate_config(&json_config::parse(&text, true)?)?;
    Ok(source)
}

pub fn parse(text: &str) -> Result<Value> {
    let raw = json_config::parse(text, true)?;
    validate_config(&raw)?;
    let table = raw
        .get("mcp")
        .and_then(|mcp| mcp.get("servers"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !table.is_object() {
        bail!("OpenCode MCP 服务表需要映射");
    }
    Ok(json!({"mcp_servers":table}))
}

pub(crate) fn check_import(source: &Source) -> Result<()> {
    let text = std::fs::read_to_string(&source.path).context("OpenCode 配置无法读取")?;
    if text.len() > 4 * 1024 * 1024 {
        bail!("配置超过 4 MiB");
    }
    let raw = json_config::parse(&text, true)?;
    if raw.pointer("/mcp/timeout").is_some() || raw.pointer("/experimental/mcp_timeout").is_some() {
        bail!("此来源含 OpenCode 全局毫秒超时，尚不能完整导入；请保留原生配置或显式建立托管连接");
    }
    Ok(())
}

pub fn render(text: &str, name: &str, value: Option<&Value>) -> Result<String> {
    validate_config(&json_config::parse(text, true)?)?;
    json_config::render(text, true, &["mcp".into(), "servers".into()], name, value)
}

fn managed_string(value: &str) -> Result<String> {
    if value.contains("${") {
        bail!(
            "OpenCode 字面变量与托管变量语法冲突，尚不能等价导入；请保留原生配置或显式建立托管连接"
        );
    }
    if value.contains("{file:") {
        bail!("OpenCode 文件变量尚不能等价托管，请保留原生配置或显式配置托管凭据");
    }
    let mut rest = value;
    while let Some(start) = rest.find("{env:") {
        let tail = &rest[start + 5..];
        let end = tail.find('}').context("OpenCode 环境变量引用不完整")?;
        let name = &tail[..end];
        if name.is_empty()
            || !name.bytes().enumerate().all(|(index, c)| {
                c == b'_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit())
            })
        {
            bail!("OpenCode 环境变量引用格式不受支持");
        }
        rest = &tail[end + 1..];
    }
    Ok(value.replace("{env:", "${env:"))
}

fn native_string(value: &str) -> Result<String> {
    let mut result = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail.find('}').context("变量引用不完整")?;
        let name = tail[..end].strip_prefix("env:").unwrap_or(&tail[..end]);
        if name.is_empty()
            || matches!(
                name,
                "userHome" | "workspaceFolder" | "workspaceFolderBasename"
            )
            || !name.bytes().enumerate().all(|(index, c)| {
                c == b'_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit())
            })
        {
            bail!("此变量引用不能等价转换到 OpenCode，请使用共享接入");
        }
        result.push_str(&format!("{{env:{name}}}"));
        rest = &tail[end + 1..];
    }
    result.push_str(rest);
    Ok(result)
}

fn strings(
    raw: Option<&Value>,
    convert: fn(&str) -> Result<String>,
) -> Result<BTreeMap<String, String>> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    raw.as_object()
        .context("OpenCode 环境变量/HTTP 头需要字符串映射")?
        .iter()
        .map(|(key, value)| {
            Ok((
                key.clone(),
                convert(
                    value
                        .as_str()
                        .context("OpenCode 环境变量/HTTP 头需要字符串映射")?,
                )?,
            ))
        })
        .collect()
}

pub fn normalize(raw: &Value, project: Option<&Path>) -> Result<Definition> {
    let map = raw.as_object().context("OpenCode 服务需要映射")?;
    if map.contains_key("enabled") {
        bail!("当前 OpenCode 配置使用 disabled，不支持旧版 enabled 字段");
    }
    if map.keys().any(|key| {
        !matches!(
            key.as_str(),
            "type"
                | "command"
                | "environment"
                | "cwd"
                | "url"
                | "headers"
                | "oauth"
                | "disabled"
                | "timeout"
                | "protocol"
                | "pool"
                | "codemode"
        )
    }) || map.get("codemode").is_some_and(|value| value != false)
    {
        bail!("此 OpenCode 原生字段尚不能等价托管，请保留原生配置或显式建立托管连接");
    }
    let disabled = map
        .get("disabled")
        .map(|value| value.as_bool().context("OpenCode 启停字段需要布尔值"))
        .transpose()?;
    let enabled = !disabled.unwrap_or(false);
    if map.contains_key("timeout") {
        bail!("OpenCode 原生毫秒超时尚不能完整托管，请保留原生配置或显式建立托管连接");
    }
    if map.get("protocol").is_some_and(|v| v != "legacy")
        || map.get("pool").is_some_and(|v| v != false)
    {
        bail!("此 OpenCode 连接要求尚未实现的协议生命周期，请保留原生配置");
    }
    let mut definition = Definition {
        enabled,
        original: raw.clone(),
        original_agent: Some(Agent::Opencode),
        origin_project: project.map(Path::to_path_buf),
        ..Definition::default()
    };
    match map.get("type").and_then(Value::as_str) {
        Some("local") => {
            let command = map
                .get("command")
                .and_then(Value::as_array)
                .filter(|v| !v.is_empty())
                .context("OpenCode 本地 command 需要非空字符串数组")?;
            let command = command
                .iter()
                .map(|v| managed_string(v.as_str().context("OpenCode command 需要字符串数组")?))
                .collect::<Result<Vec<_>>>()?;
            definition.command = Some(command[0].clone());
            definition.args = command[1..].to_vec();
            definition.env = strings(map.get("environment"), managed_string)?;
            definition.cwd = map
                .get("cwd")
                .map(|v| managed_string(v.as_str().context("OpenCode cwd 需要字符串")?))
                .transpose()?;
        }
        Some("remote") => {
            definition.url = Some(managed_string(
                map.get("url")
                    .and_then(Value::as_str)
                    .context("OpenCode 远程 url 需要字符串")?,
            )?);
            definition.headers = strings(map.get("headers"), managed_string)?;
            if let Some(oauth) = map.get("oauth")
                && oauth != false
            {
                let oauth = oauth
                    .as_object()
                    .context("OpenCode oauth 需要对象或 false")?;
                let key = "client_id";
                if oauth.keys().any(|name| name != key) {
                    bail!(
                        "此 OpenCode 原生 OAuth 专用参数尚不能等价导入，请另行配置托管 OAuth 客户端并授权"
                    );
                }
                definition.oauth = true;
                definition.client_id = oauth
                    .get(key)
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .context("OpenCode OAuth Client ID 需要字符串")
                    })
                    .transpose()?;
            }
        }
        _ => bail!("OpenCode 服务 type 须为 local 或 remote"),
    }
    config::validate(&definition)?;
    Ok(definition)
}

pub fn native(def: &Definition) -> Result<Value> {
    if def.client_secret.is_some()
        || !def.oauth_scopes.is_empty()
        || def.oauth_callback_port.is_some()
    {
        bail!("托管 OAuth 客户端参数不能等价导出到 OpenCode，请使用共享接入");
    }
    if def.include_specified
        || !def.include_tools.is_empty()
        || !def.exclude_tools.is_empty()
        || def.resources_disabled
        || def.prompts_disabled
    {
        bail!("OpenCode 直连不能等价保留当前工具筛选，请使用共享接入");
    }
    const FIELDS: &[&str] = &[
        "type",
        "command",
        "args",
        "env",
        "environment",
        "cwd",
        "url",
        "headers",
        "http_headers",
        "env_http_headers",
        "bearer_token_env_var",
        "oauth",
        "auth",
        "enabled",
        "disabled",
        "timeout",
        "startup_timeout_sec",
        "connect_timeout",
        "tool_timeout_sec",
        "codemode",
        "protocol",
        "pool",
    ];
    if def.original_agent.as_ref() != Some(&Agent::Opencode)
        && def
            .original
            .as_object()
            .is_some_and(|map| map.keys().any(|key| !FIELDS.contains(&key.as_str())))
    {
        bail!("来源含目标 OpenCode 版本尚未适配的字段，请使用共享接入");
    }
    let mut value = if def.original_agent == Some(Agent::Opencode) {
        def.original.clone()
    } else {
        json!({})
    };
    let map = value.as_object_mut().context("服务配置格式无效")?;
    for key in FIELDS {
        if !matches!(*key, "codemode" | "protocol" | "pool") {
            map.remove(*key);
        }
    }
    map.insert("disabled".into(), json!(!def.enabled));
    if let Some(command) = &def.command {
        let mut command = vec![native_string(command)?];
        command.extend(
            def.args
                .iter()
                .map(|v| native_string(v))
                .collect::<Result<Vec<_>>>()?,
        );
        map.insert("type".into(), json!("local"));
        map.insert("command".into(), json!(command));
        if !def.env.is_empty() {
            map.insert(
                "environment".into(),
                json!(
                    def.env
                        .iter()
                        .map(|(key, value)| Ok((key.clone(), native_string(value)?)))
                        .collect::<Result<BTreeMap<_, _>>>()?
                ),
            );
        }
        if let Some(cwd) = def.cwd.clone().or_else(|| {
            def.origin_project
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
        }) {
            map.insert("cwd".into(), json!(native_string(&cwd)?));
        }
    } else if let Some(url) = &def.url {
        map.insert("type".into(), json!("remote"));
        map.insert("url".into(), json!(native_string(url)?));
        if !def.headers.is_empty() {
            map.insert(
                "headers".into(),
                json!(
                    def.headers
                        .iter()
                        .map(|(key, value)| Ok((key.clone(), native_string(value)?)))
                        .collect::<Result<BTreeMap<_, _>>>()?
                ),
            );
        }
        if def.oauth {
            let mut oauth = json!({});
            if let Some(client) = &def.client_id {
                oauth["client_id"] = json!(native_string(client)?);
            }
            map.insert("oauth".into(), oauth);
        } else if def.original_agent != Some(Agent::Opencode)
            || def.original.get("oauth") == Some(&Value::Bool(false))
        {
            map.insert("oauth".into(), json!(false));
        }
    }
    let mut timeouts = json!({});
    if let Some(seconds) = def.startup_timeout {
        timeouts["startup"] = json!(
            seconds
                .checked_mul(1000)
                .context("超时超出 OpenCode 范围")?
        );
    }
    if let Some(seconds) = def.timeout {
        let millis = seconds
            .checked_mul(1000)
            .context("超时超出 OpenCode 范围")?;
        timeouts["catalog"] = json!(millis);
        timeouts["execution"] = json!(millis);
    }
    if !timeouts.as_object().unwrap().is_empty() {
        map.insert("timeout".into(), timeouts);
    }
    Ok(value)
}
