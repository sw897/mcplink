use crate::model::{Agent, Definition, Detected, Source, digest};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

pub(crate) const FIELDS: &[&str] = &[
    "type",
    "command",
    "args",
    "env",
    "env_vars",
    "cwd",
    "url",
    "headers",
    "http_headers",
    "env_http_headers",
    "bearer_token_env_var",
    "enabled",
    "auth",
    "oauth",
    "tools",
    "enabled_tools",
    "disabled_tools",
    "timeout",
    "tool_timeout_sec",
    "connect_timeout",
    "startup_timeout_sec",
    "environment",
    "disabled",
    "codemode",
    "protocol",
    "pool",
];

pub fn user_home() -> Result<PathBuf> {
    absolute_env("MCPLINK_HOME")
        .or_else(|| directories::UserDirs::new().map(|dirs| dirs.home_dir().to_owned()))
        .context("无法定位当前用户目录")
}

pub(crate) fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

pub fn opencode_home() -> Result<PathBuf> {
    crate::opencode::directory()
}

pub fn default_sources() -> Result<Vec<Source>> {
    let home = user_home()?;
    let codex = absolute_env("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
    let hermes = absolute_env("HERMES_HOME").unwrap_or_else(|| home.join(".hermes"));
    let mut sources = vec![
        Source {
            id: "codex:user".into(),
            agent: Agent::Codex,
            label: "用户配置".into(),
            path: codex.join("config.toml"),
            project: None,
            read_only: false,
            plugin_root: None,
            discovery_error: None,
            scope: None,
            legacy_ids: Vec::new(),
        },
        Source {
            id: "hermes:user".into(),
            agent: Agent::Hermes,
            label: "用户配置".into(),
            path: hermes.join("config.yaml"),
            project: None,
            read_only: false,
            plugin_root: None,
            discovery_error: None,
            scope: None,
            legacy_ids: Vec::new(),
        },
        Source {
            id: "claude:user".into(),
            agent: Agent::Claude,
            label: "用户 MCP".into(),
            path: absolute_env("CLAUDE_CONFIG_DIR")
                .unwrap_or(home.clone())
                .join(".claude.json"),
            project: None,
            read_only: false,
            plugin_root: None,
            discovery_error: None,
            scope: Some("user".into()),
            legacy_ids: Vec::new(),
        },
    ];
    sources.extend(claude_plugins());
    sources.extend(crate::opencode::default_sources()?);
    sources.extend(crate::agent_config::default_sources()?);
    for source in &mut sources {
        if let Ok(path) = source_path(&source.path) {
            source.path = path;
        }
    }
    Ok(sources)
}

pub fn claude_home() -> Result<PathBuf> {
    Ok(absolute_env("CLAUDE_CONFIG_DIR").unwrap_or(user_home()?.join(".claude")))
}

fn claude_user_path() -> Result<PathBuf> {
    Ok(absolute_env("CLAUDE_CONFIG_DIR")
        .unwrap_or(user_home()?)
        .join(".claude.json"))
}

pub(crate) fn canonical_path(path: &Path) -> Result<PathBuf> {
    let canonical = path.canonicalize()?;
    #[cfg(windows)]
    {
        let text = canonical.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{rest}")));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(rest));
        }
    }
    Ok(canonical)
}

pub(crate) fn path_identity(path: &Path) -> String {
    let value = path.to_string_lossy();
    if cfg!(any(windows, target_os = "macos")) {
        value.to_lowercase()
    } else {
        value.into_owned()
    }
}

pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        path_identity(a) == path_identity(b)
    } else {
        a == b
    }
}

pub(crate) fn in_directory(path: &Path, root: &Path) -> bool {
    let normalized = source_path(&root.join(".mcplink-boundary"));
    let directory = normalized
        .as_ref()
        .ok()
        .and_then(|path| path.parent())
        .unwrap_or(root);
    let prefix = |root: &Path| {
        let mut components = path.components();
        root.components().all(|part| {
            components.next().is_some_and(|candidate| {
                same_path(
                    Path::new(part.as_os_str()),
                    Path::new(candidate.as_os_str()),
                )
            })
        })
    };
    prefix(root) || prefix(directory)
}

pub(crate) fn protected_path(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        ["managed-mcp.json", "managed-settings.json"]
            .iter()
            .any(|candidate| same_path(Path::new(name), Path::new(candidate)))
    }) || claude_home().is_ok_and(|root| in_directory(path, &root.join("plugins")))
        || crate::opencode::is_managed(path)
}

/// Configuration identity follows existing directories, even before the file
/// exists. Keep the final component for the write-time link check.
pub(crate) fn source_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let file = absolute.file_name().context("配置文件路径缺少名称")?;
    let mut parent = absolute.parent().context("配置路径缺少父目录")?;
    let mut missing = Vec::new();
    loop {
        match canonical_path(parent) {
            Ok(mut canonical) => {
                anyhow::ensure!(canonical.is_dir(), "配置父路径须为目录");
                canonical.extend(missing.into_iter().rev());
                return Ok(canonical.join(file));
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                anyhow::ensure!(
                    !fs::symlink_metadata(parent).is_ok_and(|metadata| metadata.is_symlink()),
                    "配置目录链接无法解析"
                );
                missing.push(parent.file_name().context("配置目录路径无法规范化")?);
                parent = parent.parent().context("配置父目录不存在")?;
            }
            Err(error) => return Err(error.context("配置目录无法访问")),
        }
    }
}

fn claude_project_key(project: PathBuf) -> PathBuf {
    // Claude Code keys local settings by the Git project root, including when
    // invoked from a subdirectory. Do not let inherited Git overrides select
    // an unrelated repository.
    let mut command = std::process::Command::new("git");
    command
        .args(["-C"])
        .arg(&project)
        .args(["rev-parse", "--show-toplevel"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    if let Ok(output) = command.output()
        && output.status.success()
        && let Ok(text) = String::from_utf8(output.stdout)
        && let Ok(root) = canonical_path(&PathBuf::from(text.trim_end_matches(['\r', '\n'])))
        && project.starts_with(&root)
    {
        return root;
    }
    project
}

pub(crate) fn claude_project_json_key(project: &Path) -> String {
    #[cfg(windows)]
    {
        project.to_string_lossy().replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        project.to_string_lossy().into_owned()
    }
}

pub fn source_from_params(params: &Value, sources: &[Source]) -> Result<Source> {
    if ["scope", "project", "path", "label"].iter().any(|key| {
        params
            .get(*key)
            .is_some_and(|value| !value.is_null() && !value.is_string())
    }) {
        bail!("来源参数类型无效");
    }
    let agent: Agent =
        serde_json::from_value(params.get("agent").cloned().context("请选择 Agent")?)
            .map_err(|_| anyhow::anyhow!("Agent 类型无效"))?;
    if agent == Agent::Opencode {
        return crate::opencode::source_from_params(params, sources);
    }
    if agent == Agent::Removed {
        bail!("此客户端不在支持名单中");
    }
    if agent.manual_import() {
        bail!("此客户端通过界面导入，请使用 connections export-config 生成配置");
    }
    if agent.native_adapter() {
        return crate::agent_config::source_from_params(agent, params, sources);
    }
    let scope = params.get("scope").and_then(Value::as_str);
    if scope.is_some()
        && (agent != Agent::Claude || !matches!(scope, Some("user" | "local" | "project")))
    {
        bail!("配置作用域无效；Claude Code 支持 user、local、project");
    }
    let mut project = params
        .get("project")
        .and_then(Value::as_str)
        .map(PathBuf::from);
    if let Some(path) = &project {
        if !path.is_absolute() || !path.is_dir() {
            bail!("请选择已存在的完整项目目录");
        }
        project = Some(canonical_path(path).context("项目目录无法访问")?);
    }
    if matches!(scope, Some("local" | "project")) && project.is_none() {
        bail!("此作用域需要显式项目目录");
    }
    if project.is_some() && !matches!(scope, Some("local" | "project")) {
        bail!("项目目录需要 local 或 project 作用域");
    }
    if scope == Some("local") {
        project = project.map(claude_project_key);
    }
    let supplied_path = params
        .get("path")
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let expected = match scope {
        Some("local") => Some(claude_user_path()?),
        Some("project") => Some(project.as_ref().unwrap().join(".mcp.json")),
        _ => None,
    };
    if let (Some(supplied), Some(expected)) = (&supplied_path, &expected)
        && !same_path(&source_path(supplied)?, &source_path(expected)?)
    {
        bail!("配置路径与所选 Claude Code 作用域不符");
    }
    let path = supplied_path
        .or(expected)
        .or_else(|| {
            (scope == Some("user"))
                .then(claude_user_path)
                .and_then(Result::ok)
        })
        .context("请选择完整的配置文件路径")?;
    if !path.is_absolute() {
        bail!("请选择完整的配置文件路径");
    }
    let path = source_path(&path)?;
    for existing in sources {
        if !same_path(&existing.path, &path) {
            continue;
        }
        if existing.read_only
            && (existing.plugin_root.is_some()
                || existing
                    .scope
                    .as_deref()
                    .is_some_and(|s| matches!(s, "plugin" | "managed")))
        {
            return Ok(existing.clone());
        }
        if existing.agent != agent {
            return Ok(existing.clone());
        }
        if existing.scope.as_deref() == scope && existing.project == project {
            return Ok(existing.clone());
        }
        if scope.is_none() {
            return Ok(existing.clone());
        }
        if agent != Agent::Claude
            || !matches!(scope, Some("user" | "local"))
            || !matches!(existing.scope.as_deref(), None | Some("user" | "local"))
        {
            bail!("此文件已登记为另一配置作用域，请核对已有来源");
        }
    }
    let plugin = in_directory(&path, &claude_home()?.join("plugins"));
    let managed = protected_path(&path);
    let read_only = plugin || managed || (agent == Agent::Claude && scope.is_none());
    let scope = if plugin {
        Some("plugin")
    } else if managed {
        Some("managed")
    } else {
        scope
    };
    let id = if agent == Agent::Claude && matches!(scope, Some("local" | "project")) {
        format!(
            "claude:{}:{}",
            scope.unwrap(),
            digest(project.as_ref().unwrap().to_string_lossy().as_bytes())
        )
    } else {
        format!(
            "source:{}",
            digest(
                format!(
                    "{}:{}:{}",
                    agent.label(),
                    path.display(),
                    scope.unwrap_or("")
                )
                .as_bytes()
            )
        )
    };
    let source = Source {
        id,
        agent,
        label: params
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "{}{}",
                    scope.unwrap_or("自定义配置"),
                    if read_only { "（只读）" } else { "" }
                )
            }),
        path,
        project,
        read_only,
        plugin_root: None,
        discovery_error: None,
        scope: scope.map(str::to_owned),
        legacy_ids: Vec::new(),
    };
    read(&source)?;
    Ok(source)
}

fn claude_plugins() -> Vec<Source> {
    let Ok(home) = claude_home() else {
        return Vec::new();
    };
    let registry = home.join("plugins/installed_plugins.json");
    let settings = home.join("settings.json");
    let load = |path: &Path| -> Result<Value> {
        if !path.exists() {
            return Ok(json!({}));
        }
        let text = fs::read(path).context("Claude Code 插件登记无法读取")?;
        if text.len() > 4 * 1024 * 1024 {
            bail!("Claude Code 插件登记超过 4 MiB");
        }
        serde_json::from_slice(&text).context("Claude Code 插件登记或设置不是有效 JSON")
    };
    let discovered = || -> Result<Vec<Source>> {
        let settings = load(&settings)?;
        let registry = load(&registry)?;
        if !settings.is_object()
            || !registry.is_object()
            || settings
                .get("enabledPlugins")
                .is_some_and(|v| !v.is_object())
            || registry.get("plugins").is_some_and(|v| !v.is_object())
        {
            bail!("Claude Code 插件登记或设置格式无效");
        }
        let mut sources = Vec::new();
        if let Some(plugins) = registry.get("plugins").and_then(Value::as_object) {
            for (name, installs) in plugins {
                if settings
                    .pointer("/enabledPlugins")
                    .and_then(|p| p.get(name))
                    .and_then(Value::as_bool)
                    != Some(true)
                {
                    continue;
                }
                let Some(installs) = installs.as_array() else {
                    bail!("Claude Code 插件安装登记格式无效")
                };
                for install in installs {
                    if install["scope"] != "user" {
                        continue;
                    }
                    let root = PathBuf::from(
                        install["installPath"]
                            .as_str()
                            .context("Claude Code 插件缺少安装路径")?,
                    );
                    if !root.is_absolute() {
                        bail!("Claude Code 插件安装路径必须是绝对路径");
                    }
                    let path = root.join(".mcp.json");
                    if !path.is_file() {
                        continue;
                    }
                    sources.push(Source {
                        id: format!("claude:plugin:{}", digest(name.as_bytes())),
                        agent: Agent::Claude,
                        label: format!("用户插件 · {name}（只读）"),
                        path,
                        project: None,
                        read_only: true,
                        plugin_root: Some(root),
                        discovery_error: None,
                        scope: Some("plugin".into()),
                        legacy_ids: Vec::new(),
                    });
                }
            }
        }
        Ok(sources)
    };
    discovered().unwrap_or_else(|_| {
        vec![Source {
            id: "claude:plugins:error".into(),
            agent: Agent::Claude,
            label: "插件发现异常".into(),
            path: registry,
            project: None,
            read_only: true,
            plugin_root: None,
            discovery_error: Some("Claude Code 插件登记或设置无法读取，请在原客户端核对".into()),
            scope: Some("plugin".into()),
            legacy_ids: Vec::new(),
        }]
    })
}

pub(crate) fn require_readable(source: &Source) -> Result<()> {
    if source.agent == Agent::Qwenwork {
        crate::agent_config::assert_qwen_source(source)?;
    }
    if let Some(error) = &source.discovery_error {
        bail!("{error}");
    }
    Ok(())
}

pub fn read(source: &Source) -> Result<(String, Value)> {
    require_readable(source)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let text = match options.open(&source.path) {
        Ok(file) => crate::config_file::read_text(&file)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => bail!("无法读取配置，请检查文件访问状态"),
    };
    let value = parse(source, &text)?;
    Ok((text, value))
}

pub fn parse(source: &Source, text: &str) -> Result<Value> {
    let value = if text.trim().is_empty() {
        json!({})
    } else {
        match source.agent {
            Agent::Codex => serde_json::to_value(
                toml::from_str::<toml::Value>(text)
                    .map_err(|_| anyhow::anyhow!("TOML 配置无法解析，请在来源文件中修复"))?,
            )?,
            Agent::Hermes => serde_yaml::from_str::<Value>(text)
                .map_err(|_| anyhow::anyhow!("YAML 配置无法解析，请在来源文件中修复"))?,
            Agent::Claude => {
                let raw = crate::json_config::parse(text, false)?;
                if !raw.is_object() {
                    bail!("Claude Code MCP JSON 根节点必须是映射");
                }
                let servers = if source.plugin_root.is_some() {
                    if raw.get("mcpServers").is_some() {
                        crate::json_config::table(&raw, &["mcpServers".into()])?
                    } else {
                        raw
                    }
                } else if source.scope.as_deref() == Some("local") {
                    let project = source.project.as_ref().context("local 来源缺少项目目录")?;
                    crate::json_config::table(
                        &raw,
                        &[
                            "projects".into(),
                            claude_project_json_key(project),
                            "mcpServers".into(),
                        ],
                    )?
                } else {
                    crate::json_config::table(&raw, &["mcpServers".into()])?
                };
                json!({"mcp_servers": servers})
            }
            Agent::Opencode => crate::opencode::parse(text)?,
            _ => crate::agent_config::parse(source, text)?,
        }
    };
    if !value.is_object() {
        bail!("配置根节点必须是映射");
    }
    if let Some(servers) = value.get("mcp_servers")
        && !servers.is_object()
        && !servers.is_null()
    {
        bail!("mcp_servers 必须是映射");
    }
    Ok(value)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        Some(Value::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

pub fn normalize(raw: &Value, agent: Agent) -> Result<Definition> {
    normalize_context(raw, agent, None)
}

fn normalize_context(raw: &Value, agent: Agent, project: Option<&Path>) -> Result<Definition> {
    if agent == Agent::Opencode {
        return crate::opencode::normalize(raw, project);
    }
    if agent.native_adapter() {
        return crate::agent_config::normalize(raw, &agent, project);
    }
    if !raw.is_object() {
        bail!("MCP 服务配置必须是映射");
    }
    if agent == Agent::Claude
        && raw
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| !matches!(t, "stdio" | "http"))
    {
        bail!("此 Claude Code MCP 传输尚未适配，保留原客户端配置");
    }
    if agent == Agent::Claude
        && (raw.get("headersHelper").is_some()
            || raw.get("cwd").is_some()
            || raw.get("enabled").is_some()
            || raw.get("disabled").is_some()
            || raw
                .get("oauth")
                .and_then(Value::as_object)
                .is_some_and(|oauth| {
                    oauth
                        .keys()
                        .any(|key| !matches!(key.as_str(), "clientId" | "callbackPort"))
                }))
    {
        bail!("此 Claude Code 原生字段尚不能等价托管，请保留原客户端配置或显式建立托管连接");
    }
    if agent == Agent::Claude
        && raw.as_object().is_some_and(|map| {
            map.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "type" | "command" | "args" | "env" | "url" | "headers" | "oauth"
                )
            })
        })
    {
        bail!("此 Claude Code 原生字段尚不能等价托管，请保留原客户端配置或显式建立托管连接");
    }
    if agent == Agent::Claude
        && (raw
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|value| value.contains("${"))
            || raw
                .get("headers")
                .and_then(Value::as_object)
                .is_some_and(|headers| {
                    headers
                        .values()
                        .any(|value| value.as_str().is_some_and(|value| value.contains("${")))
                }))
    {
        bail!(
            "Claude Code 远程变量有客户端专属的凭据过滤，尚不能等价托管；请保留原生配置或显式建立托管连接"
        );
    }
    for field in ["env", "headers", "http_headers", "env_http_headers"] {
        if let Some(value) = raw.get(field)
            && !value
                .as_object()
                .is_some_and(|m| m.values().all(Value::is_string))
        {
            bail!("{field} 需要字符串键值映射");
        }
    }
    let claude_oauth = if agent == Agent::Claude {
        raw.get("oauth")
            .map(|value| {
                anyhow::ensure!(
                    raw.get("url").is_some() && raw.get("command").is_none(),
                    "Claude Code OAuth 仅适用于 HTTP 服务"
                );
                value.as_object().context("Claude Code oauth 需要对象")
            })
            .transpose()?
    } else {
        None
    };
    let claude_client = claude_oauth
        .and_then(|value| value.get("clientId"))
        .map(|value| {
            value
                .as_str()
                .context("Claude Code OAuth Client ID 需要非空字符串")
        })
        .transpose()?;
    let claude_port = claude_oauth
        .and_then(|value| value.get("callbackPort"))
        .map(|value| {
            value
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port != 0)
                .context("Claude Code OAuth 回调端口须为 1–65535")
        })
        .transpose()?;
    if let Some(value) = raw.get("args")
        && !value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
    {
        bail!("args 需要字符串数组");
    }
    for field in [
        "timeout",
        "tool_timeout_sec",
        "connect_timeout",
        "startup_timeout_sec",
    ] {
        if let Some(value) = raw.get(field)
            && value.as_u64().is_none()
        {
            bail!("{field} 当前仅支持非负整数秒，请先核对来源配置");
        }
    }
    let mut headers = string_map(raw.get("headers").or_else(|| raw.get("http_headers")));
    for (key, value) in string_map(raw.get("env_http_headers")) {
        anyhow::ensure!(valid_env_name(&value), "请求头的环境变量名称无效");
        headers.insert(key, format!("${{env:{value}}}"));
    }
    if let Some(value) = raw.get("bearer_token_env_var") {
        let variable = value
            .as_str()
            .filter(|name| valid_env_name(name))
            .context("凭据环境变量名称无效")?;
        headers.retain(|key, _| !key.eq_ignore_ascii_case("Authorization"));
        headers.insert(
            "Authorization".into(),
            format!("Bearer ${{env:{variable}}}"),
        );
    }
    let mut environment = string_map(raw.get("env"));
    if let Some(values) = raw.get("env_vars") {
        anyhow::ensure!(
            values.as_array().is_some_and(|items| items
                .iter()
                .all(|item| item.as_str().is_some_and(valid_env_name))),
            "继承环境变量需要本机变量名称数组"
        );
    }
    for key in strings(raw.get("env_vars")) {
        environment
            .entry(key.clone())
            .or_insert_with(|| format!("${{env:{key}}}"));
    }
    let mut definition = Definition {
        http_api: None,
        command: raw
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned),
        args: strings(raw.get("args")),
        env: environment,
        cwd: raw.get("cwd").and_then(Value::as_str).map(str::to_owned),
        url: raw.get("url").and_then(Value::as_str).map(str::to_owned),
        private_network_origin: None,
        headers,
        enabled: raw.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        oauth: claude_oauth.is_some() || raw.get("auth").and_then(Value::as_str) == Some("oauth"),
        client_id: claude_client
            .or_else(|| raw.pointer("/oauth/client_id").and_then(Value::as_str))
            .map(str::to_owned),
        include_tools: strings(
            raw.get("enabled_tools")
                .or_else(|| raw.pointer("/tools/include")),
        ),
        exclude_tools: strings(
            raw.get("disabled_tools")
                .or_else(|| raw.pointer("/tools/exclude")),
        ),
        timeout: raw
            .get("tool_timeout_sec")
            .or_else(|| raw.get("timeout"))
            .and_then(Value::as_u64),
        startup_timeout: raw
            .get("startup_timeout_sec")
            .or_else(|| raw.get("connect_timeout"))
            .and_then(Value::as_u64),
        include_specified: raw
            .get("enabled_tools")
            .or_else(|| raw.pointer("/tools/include"))
            .is_some(),
        resources_disabled: raw.pointer("/tools/resources").and_then(Value::as_bool) == Some(false),
        prompts_disabled: raw.pointer("/tools/prompts").and_then(Value::as_bool) == Some(false),
        original: raw.clone(),
        original_agent: Some(agent),
        client_secret: None,
        oauth_scopes: Vec::new(),
        oauth_callback_port: claude_port,
        origin_project: project.map(Path::to_path_buf),
    };
    if definition.original_agent == Some(Agent::Claude) {
        crate::json_config::claude_variables(&mut definition, true)?;
    }
    validate(&definition)?;
    Ok(definition)
}

pub fn validate(def: &Definition) -> Result<()> {
    crate::outbound::validate_approval(def)?;
    crate::http_api::validate(def)?;
    if def.client_id.as_ref().is_some_and(|value| {
        value.trim().is_empty() || value.len() > 2048 || value.chars().any(char::is_control)
    }) || def.client_secret.as_ref().is_some_and(|value| {
        value.is_empty() || value.len() > 65536 || value.chars().any(char::is_control)
    }) || (def.client_secret.is_some() && def.client_id.is_none())
        || def.oauth_callback_port == Some(0)
        || def.oauth_scopes.len() > 32
        || def.oauth_scopes.iter().any(|scope| {
            scope.is_empty()
                || scope.len() > 256
                || !scope
                    .bytes()
                    .all(|c| c == 0x21 || (0x23..=0x5b).contains(&c) || (0x5d..=0x7e).contains(&c))
        })
    {
        bail!(
            "OAuth 客户端配置无效；机密须配合 clientId，scope 须为独立标识，回调端口须为 1–65535 或留空"
        );
    }
    if def.command.is_some() == def.url.is_some() {
        bail!("请填写启动命令或 MCP 地址中的一项");
    }
    if let Some(url) = &def.url {
        let resolved = resolve(url, def.origin_project.as_deref())?;
        let url = url::Url::parse(&resolved).context("MCP 地址无效")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            bail!("MCP 地址需要 HTTP 或 HTTPS");
        }
        if !url.username().is_empty() || url.password().is_some() {
            bail!("请通过认证字段配置凭据");
        }
    }
    if def
        .command
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("启动命令为空");
    }
    Ok(())
}

pub fn scan(source: &Source) -> Result<Vec<Detected>> {
    let (_, document) = read(source)?;
    let Some(servers) = document.get("mcp_servers").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    servers
        .iter()
        .map(|(name, raw)| {
            let mut raw = raw.clone();
            if let Some(root) = &source.plugin_root {
                resolve_plugin_root(&mut raw, &root.to_string_lossy());
            }
            let mut definition = normalize_source(&raw, source).unwrap_or_default();
            definition.origin_project = source.project.clone();
            Ok(Detected {
                source: source.clone(),
                name: name.clone(),
                raw: raw.clone(),
                definition,
                error: normalize_source(&raw, source).err().map(|e| e.to_string()),
                fingerprint: fingerprint(Some(&raw)),
            })
        })
        .collect()
}

fn resolve_plugin_root(value: &mut Value, root: &str) {
    match value {
        Value::String(text) => *text = text.replace("${CLAUDE_PLUGIN_ROOT}", root),
        Value::Array(values) => values.iter_mut().for_each(|v| resolve_plugin_root(v, root)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|v| resolve_plugin_root(v, root)),
        _ => {}
    }
}

pub fn fingerprint(value: Option<&Value>) -> String {
    digest(&serde_json::to_vec(&value).unwrap_or_default())
}

pub fn normalize_source(raw: &Value, source: &Source) -> Result<Definition> {
    if source.agent == Agent::Opencode {
        crate::opencode::normalize(raw, source.project.as_deref())
    } else {
        normalize_context(raw, source.agent.clone(), source.project.as_deref())
    }
}

pub fn check_source_import(source: &Source, raw: &Value) -> Result<()> {
    if source.agent == Agent::Opencode {
        crate::opencode::check_import(source)
    } else if source.agent.native_adapter() {
        crate::agent_config::check_import(source, raw)
    } else {
        Ok(())
    }
}

pub fn native(def: &Definition, source: &Source) -> Result<Value> {
    crate::adapter_policy::validate(def, &source.agent, false)?;
    if source.agent.native_adapter() {
        return crate::agent_config::native(def, source);
    }
    let target = &source.agent;
    if *target == Agent::Opencode {
        return crate::opencode::native(def);
    }
    if def
        .original_agent
        .as_ref()
        .is_some_and(|agent| agent != target)
        && ["${", "{env:", "{file:"].iter().any(|pattern| {
            serde_json::to_string(&def.original).is_ok_and(|value| value.contains(pattern))
        })
    {
        bail!("跨客户端直连不能保证变量引用等价，请使用共享接入或明确配置目标客户端的变量");
    }
    if *target == Agent::Claude {
        return crate::json_config::claude_native(def, source);
    }
    if def
        .original_agent
        .as_ref()
        .is_some_and(|source| source != target)
        && let Some(map) = def.original.as_object()
    {
        let unsupported: Vec<_> = map
            .keys()
            .filter(|key| !FIELDS.contains(&key.as_str()))
            .cloned()
            .collect();
        if !unsupported.is_empty() {
            bail!(
                "来源含目标客户端尚未适配的字段 {}，请先调整连接",
                unsupported.join(", ")
            );
        }
    }
    if def
        .original_agent
        .as_ref()
        .is_some_and(|source| source != target)
        && def
            .original
            .get("oauth")
            .and_then(Value::as_object)
            .is_some_and(|map| map.keys().any(|key| key != "client_id"))
    {
        bail!("来源包含客户端专用 OAuth 参数，请保留原生直连或使用共享认证");
    }
    let mut value = if def.original_agent.as_ref() == Some(target) {
        def.original.clone()
    } else {
        json!({})
    };
    let map = value.as_object_mut().context("服务配置格式无效")?;
    for field in FIELDS {
        if *target != Agent::Codex || *field != "tools" {
            map.remove(*field);
        }
    }
    map.insert("enabled".into(), json!(def.enabled));
    if let Some(command) = &def.command {
        map.insert("command".into(), json!(command));
        map.insert("args".into(), json!(def.args));
        if !def.env.is_empty() {
            if *target == Agent::Codex {
                let mut inherit = Vec::new();
                let mut literal = BTreeMap::new();
                for (key, value) in &def.env {
                    if env_reference(value) == Some(key.as_str()) {
                        inherit.push(key);
                    } else {
                        anyhow::ensure!(
                            !value.contains("${"),
                            "Codex 直连只支持同名环境变量继承，请使用共享接入"
                        );
                        literal.insert(key, value);
                    }
                }
                if !inherit.is_empty() {
                    map.insert("env_vars".into(), json!(inherit));
                }
                if !literal.is_empty() {
                    map.insert("env".into(), json!(literal));
                }
            } else {
                map.insert("env".into(), json!(def.env));
            }
        }
        if let Some(cwd) = def
            .cwd
            .as_ref()
            .map(PathBuf::from)
            .or(def.origin_project.clone())
        {
            map.insert("cwd".into(), json!(cwd));
        }
    }
    if let Some(url) = &def.url {
        map.insert("url".into(), json!(url));
        let header_key = if *target == Agent::Codex {
            "http_headers"
        } else {
            "headers"
        };
        let mut literal = BTreeMap::new();
        let mut environment = BTreeMap::new();
        for (key, value) in &def.headers {
            if *target == Agent::Codex {
                if let Some(variable) = bearer_reference(key, value) {
                    map.insert("bearer_token_env_var".into(), json!(variable));
                    continue;
                }
                if let Some(variable) = env_reference(value) {
                    environment.insert(key, variable);
                    continue;
                }
                anyhow::ensure!(
                    !value.contains("${"),
                    "Codex 直连请求头需要字面值或完整单变量引用，请使用共享接入"
                );
            }
            literal.insert(key, value);
        }
        if !literal.is_empty() {
            map.insert(header_key.into(), json!(literal));
        }
        if !environment.is_empty() {
            map.insert("env_http_headers".into(), json!(environment));
        }
        if def.oauth {
            map.insert("auth".into(), json!("oauth"));
        }
        if let Some(client) = &def.client_id {
            let mut oauth = if def.original_agent.as_ref() == Some(target) {
                def.original.get("oauth").cloned().unwrap_or(json!({}))
            } else {
                json!({})
            };
            oauth["client_id"] = json!(client);
            map.insert("oauth".into(), oauth);
        }
    }
    if let Some(timeout) = def.startup_timeout {
        map.insert(
            if *target == Agent::Codex {
                "startup_timeout_sec"
            } else {
                "connect_timeout"
            }
            .into(),
            json!(timeout),
        );
    }
    if *target == Agent::Codex {
        if def.include_specified || !def.include_tools.is_empty() {
            map.insert("enabled_tools".into(), json!(def.include_tools));
        }
        if !def.exclude_tools.is_empty() {
            map.insert("disabled_tools".into(), json!(def.exclude_tools));
        }
        if let Some(timeout) = def.timeout {
            map.insert("tool_timeout_sec".into(), json!(timeout));
        }
    } else {
        let mut tools = if def.original_agent.as_ref() == Some(target) {
            def.original.get("tools").cloned().unwrap_or(json!({}))
        } else {
            json!({})
        };
        tools
            .as_object_mut()
            .context("工具筛选配置格式无效")?
            .remove("include");
        tools.as_object_mut().unwrap().remove("exclude");
        if def.resources_disabled {
            tools["resources"] = json!(false);
        } else if tools["resources"] == false {
            tools["resources"] = json!(true);
        }
        if def.prompts_disabled {
            tools["prompts"] = json!(false);
        } else if tools["prompts"] == false {
            tools["prompts"] = json!(true);
        }
        if def.include_specified || !def.include_tools.is_empty() {
            tools["include"] = json!(def.include_tools);
        }
        if !def.exclude_tools.is_empty() {
            tools["exclude"] = json!(def.exclude_tools);
        }
        if tools.as_object().is_some_and(|v| !v.is_empty()) {
            map.insert("tools".into(), tools);
        }
        if let Some(timeout) = def.timeout {
            map.insert("timeout".into(), json!(timeout));
        }
    }
    Ok(value)
}

pub fn entry(source: &Source, name: &str) -> Result<Option<Value>> {
    Ok(read(source)?
        .1
        .get("mcp_servers")
        .and_then(|v| v.get(name))
        .cloned())
}

pub(crate) fn matches_written_entry(
    source: &Source,
    change: &crate::model::Change,
    current: Option<&Value>,
) -> bool {
    if current == change.after.as_ref() {
        return true;
    }
    // Earlier TOML and YAML writers left unknown fields from the previous
    // entry on disk. Only accept the exact legacy result derived from the
    // saved transaction, including the original values of those fields.
    if !matches!(source.agent, Agent::Codex | Agent::Hermes) {
        return false;
    }
    let (Some(before), Some(mut after), Some(current)) = (
        change.before.as_ref().and_then(Value::as_object),
        change.after.as_ref().and_then(Value::as_object).cloned(),
        current.and_then(Value::as_object),
    ) else {
        return false;
    };
    for (key, value) in before {
        if !FIELDS.contains(&key.as_str()) {
            after.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    current == &after
}

pub(crate) fn preserve_native_policy(
    source: &Source,
    value: &mut Value,
    before: Option<&Value>,
    shared: bool,
) -> Result<()> {
    if source.agent == Agent::Pi {
        return crate::pi_config::preserve_presentation(value, before);
    }
    let keys: &[&str] = match source.agent {
        Agent::Codex => &["required", "default_tools_approval_mode", "tools"],
        Agent::Hermes if shared => &["tools"],
        _ => &[],
    };
    for key in keys {
        if let Some(setting) = before.and_then(|entry| entry.get(*key)) {
            value[*key] = setting.clone();
        }
    }
    Ok(())
}

pub fn set_native_enabled(value: &mut Value, source: &Source, enabled: bool) -> Result<()> {
    if source.agent.native_adapter() {
        return crate::agent_config::set_enabled(value, &source.agent, enabled);
    }
    if source.agent == Agent::Claude {
        bail!("Claude Code 启停按项目偏好维护，请在原客户端 /mcp 中切换；此处可预览移除配置");
    }
    if source.agent == Agent::Opencode {
        value
            .as_object_mut()
            .context("服务配置需要映射")?
            .remove("enabled");
        value["disabled"] = json!(!enabled);
    } else {
        value["enabled"] = json!(enabled);
    }
    Ok(())
}

pub fn render(source: &Source, text: &str, name: &str, value: Option<&Value>) -> Result<String> {
    if source.read_only {
        bail!("该来源只读，请在原客户端或插件中维护配置；可将连接接入其他可写来源");
    }
    let original = parse(source, text)?;
    let before = original["mcp_servers"].get(name);
    if before == value {
        return Ok(text.to_owned());
    }
    let rendered = match source.agent {
        Agent::Codex => {
            use toml_edit::{DocumentMut, Item, Table};
            let mut document = text
                .parse::<DocumentMut>()
                .map_err(|_| anyhow::anyhow!("TOML 配置无法编辑"))?;
            if !document.contains_key("mcp_servers") {
                document["mcp_servers"] = Item::Table(Table::new());
            }
            let servers = document["mcp_servers"]
                .as_table_mut()
                .context("mcp_servers 需要标准 TOML 表");
            let servers = servers?;
            if let Some(value) = value {
                let desired =
                    toml_edit::ser::to_document(value).context("服务配置无法转换为 TOML")?;
                if !servers.contains_key(name) {
                    servers[name] = Item::Table(Table::new());
                }
                let table = servers[name]
                    .as_table_mut()
                    .context("服务配置需要标准 TOML 表")?;
                let obsolete: Vec<_> = table
                    .iter()
                    .filter(|(key, _)| value.get(*key).is_none())
                    .map(|(key, _)| key.to_owned())
                    .collect();
                for field in obsolete {
                    table.remove(&field);
                }
                for (key, item) in desired.as_table().iter() {
                    if before.and_then(|entry| entry.get(key)) == value.get(key) {
                        continue;
                    }
                    let mut replacement = item.clone();
                    if let (Some(old), Some(new)) = (
                        table.get(key).and_then(Item::as_value),
                        replacement.as_value_mut(),
                    ) {
                        *new.decor_mut() = old.decor().clone();
                    }
                    table.insert(key, replacement);
                }
            } else {
                servers.remove(name);
            }
            Ok(document.to_string())
        }
        Agent::Hermes => {
            let document =
                yaml_edit::Document::from_str(if text.trim().is_empty() { "{}\n" } else { text })
                    .map_err(|_| anyhow::anyhow!("YAML 配置无法编辑"))?;
            let root = document.as_mapping().context("YAML 配置根节点需要映射")?;
            if root.get_mapping("mcp_servers").is_none() {
                root.set("mcp_servers", yaml_edit::Document::from_str("{}")?);
            }
            let servers = root
                .get_mapping("mcp_servers")
                .context("mcp_servers 需要 YAML 映射")?;
            if let Some(value) = value {
                if servers.get_mapping(name).is_none() {
                    servers.set(name, yaml_edit::Document::from_str("{}")?);
                }
                let server = servers
                    .get_mapping(name)
                    .context("服务配置需要 YAML 映射")?;
                if let Some(previous) = before.and_then(Value::as_object) {
                    for field in previous.keys().filter(|key| value.get(*key).is_none()) {
                        server.remove(field.as_str());
                    }
                }
                for (key, value) in value.as_object().context("服务配置无效")? {
                    if before.and_then(|entry| entry.get(key)) == Some(value) {
                        continue;
                    }
                    server.set(
                        key.as_str(),
                        yaml_edit::Document::from_str(&serde_yaml::to_string(value)?)?,
                    );
                }
            } else {
                servers.remove(name);
            }
            Ok(document.to_string())
        }
        Agent::Claude => {
            let mut path = Vec::new();
            if source.scope.as_deref() == Some("local") {
                path.push("projects".to_owned());
                path.push(claude_project_json_key(
                    source.project.as_ref().context("local 来源缺少项目目录")?,
                ));
            }
            path.push("mcpServers".into());
            crate::json_config::render(text, false, &path, name, value)
        }
        Agent::Opencode => crate::opencode::render(text, name, value),
        _ => crate::agent_config::render(source, text, name, value),
    }?;
    anyhow::ensure!(
        parse(source, &rendered)?["mcp_servers"].get(name) == value,
        "生成的原生条目与预览不一致，请检查配置"
    );
    Ok(rendered)
}

pub fn safe_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("接入名称需要 1 至 80 位字母、数字、下划线或连字符");
    }
    Ok(())
}

pub fn name_key(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

pub(crate) fn valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().enumerate().all(|(index, c)| {
            c == b'_' || c.is_ascii_alphabetic() || (index > 0 && c.is_ascii_digit())
        })
}

pub(crate) fn env_reference(value: &str) -> Option<&str> {
    let inner = value.strip_prefix("${")?.strip_suffix('}')?;
    let name = inner.strip_prefix("env:").unwrap_or(inner);
    // Unqualified path references are distinct from environment variables.
    (valid_env_name(name)
        && !matches!(
            inner,
            "userHome" | "workspaceFolder" | "workspaceFolderBasename"
        ))
    .then_some(name)
}

pub(crate) fn bearer_reference<'a>(name: &str, value: &'a str) -> Option<&'a str> {
    if name.eq_ignore_ascii_case("Authorization") {
        env_reference(value.strip_prefix("Bearer ")?)
    } else {
        None
    }
}

pub fn resolve(input: &str, project: Option<&Path>) -> Result<String> {
    let mut expanded = String::with_capacity(input.len());
    let mut remaining = input;
    for _ in 0..32 {
        let Some((literal, variable)) = remaining.split_once("${") else {
            expanded.push_str(remaining);
            return Ok(expanded);
        };
        let (reference, rest) = variable.split_once('}').context("环境变量引用不完整")?;
        expanded.push_str(literal);
        match reference {
            "userHome" => expanded.push_str(&user_home()?.to_string_lossy()),
            "workspaceFolder" => expanded.push_str(
                &project
                    .context("工作目录变量需要项目来源")?
                    .to_string_lossy(),
            ),
            "workspaceFolderBasename" => expanded.push_str(
                project
                    .context("工作目录变量需要项目来源")?
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default(),
            ),
            _ => {
                let reference = reference.strip_prefix("env:").unwrap_or(reference);
                let (name, fallback) = reference
                    .split_once(":-")
                    .map_or((reference, None), |(name, value)| (name, Some(value)));
                anyhow::ensure!(
                    valid_env_name(name) && fallback.is_none_or(|text| !text.contains("${")),
                    "环境变量引用格式不受支持"
                );
                let value = std::env::var(name)
                    .ok()
                    .or_else(|| fallback.map(str::to_owned))
                    .with_context(|| format!("需要在运行环境配置变量 {name}"))?;
                expanded.push_str(&value);
            }
        }
        remaining = rest;
    }
    anyhow::ensure!(!remaining.contains("${"), "环境变量引用超过 32 项");
    expanded.push_str(remaining);
    Ok(expanded)
}
