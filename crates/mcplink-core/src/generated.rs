//! Small, credential-free CLI manifests. All execution stays in the managed backend.
use crate::{Engine, invocation, model::digest, runtime::Client, vault::private_write};
use anyhow::{Context, Result, bail};
use clap::{Arg, ArgAction, Command};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const MANIFEST_LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    manifest_version: u32,
    generator_version: String,
    pub name: String,
    pub dataset_id: String,
    pub connection_id: String,
    pub tools: Vec<Tool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub schema_hash: String,
    stdin_only: bool,
    parameters: Vec<Parameter>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameter {
    name: String,
    kind: Kind,
    required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    String,
    Boolean,
    Number,
    Integer,
    Json,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::String => "STRING",
            Self::Boolean => "BOOLEAN",
            Self::Number => "NUMBER",
            Self::Integer => "INTEGER",
            Self::Json => "JSON",
        }
    }

    fn parse(self, input: &str) -> Result<Value> {
        if self == Self::String {
            return Ok(Value::String(input.into()));
        }
        let value: Value = serde_json::from_str(input).context("参数必须是声明类型的 JSON 值")?;
        let valid = match self {
            Self::String => unreachable!(),
            Self::Boolean => value.is_boolean(),
            Self::Number => value.is_number(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Json => true,
        };
        if !valid {
            bail!("参数类型无效；请查看工具帮助，或使用 --stdin");
        }
        Ok(value)
    }
}

pub enum Parsed {
    Help(String),
    Call {
        tool: Tool,
        arguments: Option<Value>,
    },
}

fn identifier(value: &str, max: usize, dots: bool) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || (dots && c == b'.'))
}

fn parameter_name(value: &str) -> bool {
    identifier(value, 128, false) && !matches!(value, "stdin" | "help" | "version")
}

pub fn validate_selection(name: &str, tools: &[String]) -> Result<()> {
    let upper = name.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "MCPLINK" | "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit());
    if !identifier(name, 64, false) || reserved {
        bail!("入口名称须为安全的 ASCII 命令名，且不能是 mcplink 或系统保留名");
    }
    if tools.is_empty() || tools.len() > 128 {
        bail!("请明确选择 1 至 128 个工具");
    }
    let mut unique = HashSet::new();
    if tools.iter().any(|tool| {
        !identifier(tool, 128, true)
            || matches!(tool.as_str(), "help" | "version")
            || !unique.insert(tool)
    }) {
        bail!("工具名包含命令冲突、重复项或不能直接映射的字符");
    }
    Ok(())
}

fn checked_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    if absolute.to_string_lossy().chars().any(char::is_control) {
        bail!("路径不能包含控制字符");
    }
    let mut current = PathBuf::new();
    for component in absolute.components() {
        if component == Component::ParentDir {
            bail!("请使用不含 .. 的路径");
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("CLI 路径包含符号链接，请核对真实位置");
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("无法检查 CLI 路径"),
        }
    }
    Ok(absolute)
}

fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    let path = checked_path(path)?;
    let metadata = fs::symlink_metadata(&path).context("无法读取 CLI 清单")?;
    if !metadata.is_file() || metadata.len() > MANIFEST_LIMIT {
        bail!("CLI 清单必须是不超过 1 MiB 的普通文件");
    }
    let file = File::open(path).context("无法读取 CLI 清单")?;
    let mut bytes = Vec::new();
    file.take(MANIFEST_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MANIFEST_LIMIT {
        bail!("CLI 清单超过 1 MiB");
    }
    Ok(bytes)
}

impl Manifest {
    fn validate(&self) -> Result<()> {
        if self.manifest_version != 1
            || self.generator_version.is_empty()
            || self.generator_version.len() > 64
            || !self
                .generator_version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
            || uuid::Uuid::parse_str(&self.dataset_id).is_err()
            || uuid::Uuid::parse_str(&self.connection_id).is_err()
        {
            bail!("CLI 清单版本或连接身份无效");
        }
        validate_selection(
            &self.name,
            &self
                .tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect::<Vec<_>>(),
        )?;
        for tool in &self.tools {
            if tool.schema_hash.len() != 64
                || !tool
                    .schema_hash
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                || tool.parameters.len() > 256
                || (tool.stdin_only && !tool.parameters.is_empty())
            {
                bail!("CLI 工具契约无效");
            }
            let mut names = HashSet::new();
            if tool
                .parameters
                .iter()
                .any(|parameter| !parameter_name(&parameter.name) || !names.insert(&parameter.name))
            {
                bail!("CLI 参数契约无效");
            }
        }
        Ok(())
    }

    pub fn read(path: &Path) -> Result<Self> {
        let manifest: Self = serde_json::from_slice(&read_bytes(path)?)
            .map_err(|_| anyhow::anyhow!("CLI 清单损坏、含未知字段或格式不受支持"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Building help uses the allowlisted manifest alone, with no network or Vault access.
    pub fn parse(&self, args: Vec<OsString>) -> Result<Parsed> {
        let mut command = Command::new(self.name.clone())
            .about("通过 mcplink 托管连接调用选定工具；不自动重试")
            .disable_help_subcommand(true)
            .subcommand_required(true);
        for tool in &self.tools {
            let mut subcommand = Command::new(tool.name.clone())
                .about(if tool.stdin_only {
                    "参数通过 --stdin JSON 对象传入"
                } else {
                    "使用类型化参数或 --stdin JSON 对象"
                })
                .arg(
                    Arg::new("stdin")
                        .long("stdin")
                        .action(ArgAction::SetTrue)
                        .required(tool.stdin_only)
                        .help("从 stdin 读取工具参数 JSON 对象"),
                );
            for parameter in &tool.parameters {
                let mut arg = Arg::new(parameter.name.clone())
                    .long(parameter.name.clone())
                    .value_name(parameter.kind.label())
                    .allow_hyphen_values(true)
                    .action(ArgAction::Set)
                    .conflicts_with("stdin");
                if parameter.required {
                    arg = arg.required_unless_present("stdin");
                }
                subcommand = subcommand.arg(arg);
            }
            command = command.subcommand(subcommand);
        }
        if args.is_empty() {
            return Ok(Parsed::Help(command.render_long_help().to_string()));
        }
        let matches = match command
            .try_get_matches_from_mut(std::iter::once(OsString::from(&self.name)).chain(args))
        {
            Ok(matches) => matches,
            Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
                return Ok(Parsed::Help(error.to_string()));
            }
            Err(_) => bail!("生成命令的参数无效；请查看 --help，或使用 --stdin"),
        };
        let (name, matches) = matches.subcommand().context("请选择工具")?;
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name == name)
            .context("工具未导出")?
            .clone();
        let arguments = if matches.get_flag("stdin") {
            None
        } else {
            let mut object = serde_json::Map::new();
            for parameter in &tool.parameters {
                if let Some(value) = matches.get_one::<String>(&parameter.name) {
                    // Parsing errors must never include business values or JSON excerpts.
                    object.insert(
                        parameter.name.clone(),
                        parameter.kind.parse(value).map_err(|_| {
                            anyhow::anyhow!("参数类型无效；请查看工具帮助，或使用 --stdin")
                        })?,
                    );
                }
            }
            Some(Value::Object(object))
        };
        Ok(Parsed::Call { tool, arguments })
    }
}

async fn dataset(client: &Client) -> Result<String> {
    let value = client.call("workspace.identity", json!({})).await?;
    let id = value["id"].as_str().context("后台数据目录身份缺失")?;
    uuid::Uuid::parse_str(id).context("后台数据目录身份无效")?;
    Ok(id.into())
}

async fn describe(client: &Client, connection: &str, names: &[String]) -> Result<Vec<Tool>> {
    let mut tools = Vec::new();
    let mut revision = None;
    for name in names {
        let value = client
            .call("tools.describe", json!({"id":connection,"tool":name}))
            .await?;
        let current = value["connectionRevision"]
            .as_u64()
            .context("连接修订缺失")?;
        if revision.is_some_and(|previous| previous != current) {
            bail!("生成期间连接已变化，请重新核对后生成");
        }
        revision = Some(current);
        tools.push(
            contract(
                name.clone(),
                value["tool"]["inputSchema"].clone(),
                value["schemaHash"]
                    .as_str()
                    .context("工具摘要缺失")?
                    .to_owned(),
            )
            .await?,
        );
    }
    Ok(tools)
}

async fn contract(name: String, schema: Value, schema_hash: String) -> Result<Tool> {
    tokio::task::spawn_blocking({
        let schema = schema.clone();
        move || invocation::validator(&schema).map(|_| ())
    })
    .await
    .context("无法检查工具 Schema")??;
    let required: HashSet<_> = schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let properties = schema["properties"].as_object();
    let stdin_only = schema.get("$ref").is_some()
        || properties.is_some_and(|properties| {
            properties.len() > 256 || properties.keys().any(|key| !parameter_name(key))
        })
        || required
            .iter()
            .any(|name| properties.is_none_or(|properties| !properties.contains_key(*name)));
    let parameters = if stdin_only {
        Vec::new()
    } else {
        properties
            .into_iter()
            .flatten()
            .map(|(name, schema)| Parameter {
                name: name.clone(),
                kind: match schema["type"].as_str() {
                    Some("string") => Kind::String,
                    Some("boolean") => Kind::Boolean,
                    Some("number") => Kind::Number,
                    Some("integer") => Kind::Integer,
                    _ => Kind::Json,
                },
                required: required.contains(name.as_str()),
            })
            .collect()
    };
    Ok(Tool {
        name,
        schema_hash,
        stdin_only,
        parameters,
    })
}

fn write_new(
    path: &Path,
    bytes: &[u8],
    executable: bool,
    created: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o755 } else { 0o600 });
    }
    #[cfg(not(unix))]
    let _ = executable;
    let mut file = options
        .open(path)
        .context("无法创建 CLI 文件；不会覆盖已有文件")?;
    created.push(path.to_owned());
    file.write_all(bytes).context("无法写入 CLI 文件")?;
    file.sync_all().context("无法保存 CLI 文件")?;
    Ok(())
}

pub async fn generate(
    client: &Client,
    connection: &str,
    name: String,
    output: &Path,
    names: &[String],
) -> Result<Value> {
    let output = checked_path(output)?;
    client
        .call(
            "cli.generate",
            json!({"id":connection,"name":name,"output":output,"tools":names}),
        )
        .await
}

pub async fn generate_managed(
    engine: Arc<Engine>,
    connection: &str,
    name: String,
    output: &Path,
    names: &[String],
) -> Result<Value> {
    validate_selection(&name, names)?;
    let output = checked_path(output)?;
    if output.exists() || !output.parent().is_some_and(Path::is_dir) {
        bail!("输出目录必须尚不存在，且父目录必须已存在");
    }
    let fingerprint = crate::gateway::catalog_version(&engine, connection)?;
    let available = invocation::contracts(engine.clone(), connection).await?;
    let mut tools = Vec::new();
    for name in names {
        let tool = available
            .iter()
            .find(|tool| tool.tool["name"] == *name)
            .context("工具不存在或未在此入口中启用")?;
        tools.push(
            contract(
                name.clone(),
                tool.tool["inputSchema"].clone(),
                tool.schema_hash.clone(),
            )
            .await?,
        );
    }
    if crate::gateway::catalog_version(&engine, connection)? != fingerprint {
        bail!("生成期间入口或服务已变化，请重新核对后生成");
    }
    let dataset_id = engine.dataset_id().await?;
    let manifest = Manifest {
        manifest_version: 1,
        generator_version: env!("CARGO_PKG_VERSION").into(),
        dataset_id,
        connection_id: connection.into(),
        tools,
        name,
    };
    manifest.validate()?;
    let manifest_name = format!("{}.mcplink.json", manifest.name);
    let shell = format!(
        "#!/bin/sh\nset -eu\nbase=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"${{MCPLINK_BIN:-mcplink}}\" cli run --manifest \"$base/{manifest_name}\" -- \"$@\"\n"
    );
    // Avoid a second cmd parsing pass and PowerShell's legacy native argv binder.
    let powershell = format!(
        r#"$ErrorActionPreference = 'Stop'
function Quote-Argument([string] $text) {{
    $escaped = [regex]::Replace($text, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    return '"' + $escaped + '"'
}}
$binary = if ($env:MCPLINK_BIN) {{ $env:MCPLINK_BIN }} else {{ 'mcplink.exe' }}
$nativeArguments = @('cli', 'run', '--manifest', (Join-Path $PSScriptRoot '{manifest_name}'), '--') + $args
$start = New-Object System.Diagnostics.ProcessStartInfo
$start.FileName = $binary
$start.UseShellExecute = $false
$start.Arguments = ($nativeArguments | ForEach-Object {{ Quote-Argument ([string] $_) }}) -join ' '
try {{
    $process = [System.Diagnostics.Process]::Start($start)
    $process.WaitForExit()
    exit $process.ExitCode
}} catch {{
    [Console]::Out.WriteLine('{{"error":{{"code":"launcher_failed","message":"Cannot start mcplink; check PATH or MCPLINK_BIN"}}}}')
    exit 1
}}
"#
    );
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    if bytes.len() as u64 > MANIFEST_LIMIT {
        bail!("CLI 清单超过 1 MiB，请减少导出工具");
    }
    // Reserve a new directory exclusively. The manifest is the last file written.
    fs::create_dir(&output).context("无法创建输出目录；不会覆盖已有目录")?;
    let files = [
        output.join(&manifest.name),
        output.join(format!("{}.ps1", manifest.name)),
        output.join(&manifest_name),
    ];
    let mut created = Vec::new();
    let result = (|| {
        write_new(&files[0], shell.as_bytes(), true, &mut created)?;
        write_new(&files[1], powershell.as_bytes(), false, &mut created)?;
        if std::env::var("MCPLINK_FAILPOINT").ok().as_deref() == Some("cli-after-launchers")
            && std::env::var("MCPLINK_VAULT").ok().as_deref() == Some("fixture")
            && std::env::var_os("MCPLINK_HOME")
                .is_some_and(|home| PathBuf::from(home).join(".mcplink-test-fixture").is_file())
        {
            bail!("合成验收：中断 CLI 文件写入");
        }
        write_new(&files[2], &bytes, false, &mut created)
    })();
    if let Err(error) = result {
        for file in &created {
            let _ = fs::remove_file(file);
        }
        let _ = fs::remove_dir(&output);
        return Err(error);
    }
    Ok(
        json!({"name":manifest.name,"directory":output,"manifest":files[2],"tools":names,"runtime":"mcplink (PATH or MCPLINK_BIN)"}),
    )
}

fn changes(previous: &Tool, current: &Tool) -> Value {
    let mut parameters = Vec::new();
    // A stdin-only manifest intentionally omits parameter metadata.
    if !previous.stdin_only && !current.stdin_only {
        let names: std::collections::BTreeSet<_> = previous
            .parameters
            .iter()
            .chain(&current.parameters)
            .map(|parameter| &parameter.name)
            .collect();
        for name in names {
            let before = previous.parameters.iter().find(|p| &p.name == name);
            let after = current.parameters.iter().find(|p| &p.name == name);
            if before == after {
                continue;
            }
            let value = match (before, after) {
                (None, Some(after)) => json!({"name":name,"change":"added","after":after}),
                (Some(before), None) => json!({"name":name,"change":"removed","before":before}),
                (Some(before), Some(after)) => {
                    json!({"name":name,"change":"modified","before":before,"after":after})
                }
                (None, None) => unreachable!(),
            };
            parameters.push(value);
        }
    }
    json!({"schemaChanged":previous.schema_hash != current.schema_hash,
        "stdinOnly":if previous.stdin_only == current.stdin_only {Value::Null} else {json!({"before":previous.stdin_only,"after":current.stdin_only})},
        "parameters":parameters})
}

pub async fn check_managed(engine: Arc<Engine>, path: &Path) -> Result<Value> {
    let manifest = Manifest::read(path)?;
    let dataset_matches = engine.dataset_id().await? == manifest.dataset_id;
    let (discovery_status, discovered, diagnostics) = if dataset_matches {
        let result = async {
            let fingerprint = crate::gateway::catalog_version(&engine, &manifest.connection_id)?;
            let report =
                invocation::contract_report(engine.clone(), &manifest.connection_id).await?;
            if crate::gateway::catalog_version(&engine, &manifest.connection_id)? != fingerprint {
                bail!(invocation::ToolFailure {
                    code: invocation::ToolErrorCode::ConnectionChanged,
                    details: Value::Null
                });
            }
            Ok::<_, anyhow::Error>(report)
        }
        .await;
        match result {
            Ok((tools, diagnostics)) => (
                if diagnostics.is_empty() {
                    "complete"
                } else {
                    "partial"
                },
                tools,
                diagnostics,
            ),
            Err(error) => (
                "failed",
                Vec::new(),
                vec![crate::catalog::discovery_diagnostic(
                    &engine,
                    &manifest.connection_id,
                    &error,
                )],
            ),
        }
    } else {
        (
            "not_checked",
            Vec::new(),
            vec![
                json!({"code":"dataset_mismatch","message":"此 CLI 属于另一数据目录，请明确选择本机连接后 cli rebind"}),
            ],
        )
    };
    let mut results = Vec::new();
    for tool in &manifest.tools {
        let actual = discovered
            .iter()
            .find(|actual| actual.tool["name"] == tool.name);
        let mut result = json!({"name":tool.name});
        match actual {
            Some(actual) => {
                let current = contract(
                    tool.name.clone(),
                    actual.tool["inputSchema"].clone(),
                    actual.schema_hash.clone(),
                )
                .await;
                match current {
                    Ok(current) if &current == tool => result["status"] = json!("current"),
                    Ok(current) => {
                        result["status"] = json!("changed");
                        result["changes"] = changes(tool, &current);
                    }
                    Err(_) => {
                        result["status"] = json!("changed");
                        result["reason"] = json!({"code":"invalid_tool_schema","message":"当前工具 Schema 无法验证，请修复上游后重新检查"});
                    }
                }
            }
            None => {
                let diagnostic = diagnostics.iter().find(|diagnostic| {
                    matches!(discovery_status, "failed" | "not_checked")
                        || diagnostic["namespace"].as_str().is_some_and(|namespace| {
                            tool.name.starts_with(&format!("{namespace}__"))
                        })
                });
                if let Some(diagnostic) = diagnostic {
                    result["status"] = json!("unchecked");
                    result["reason"] = diagnostic.clone();
                } else {
                    result["status"] = json!("unavailable");
                    result["reason"] = json!({"code":"tool_unavailable","message":"当前工具清单不含此工具：已移除或被工具范围排除"});
                }
            }
        }
        results.push(result);
    }
    Ok(
        json!({"compatible":dataset_matches && results.iter().all(|tool| tool["status"] == "current"),"datasetMatches":dataset_matches,"connectionId":manifest.connection_id,
            "discovery":{"status":discovery_status,"diagnostics":diagnostics},"tools":results}),
    )
}

pub async fn rebind(client: &Client, path: &Path, connection: &str) -> Result<Value> {
    let path = checked_path(path)?;
    let mut lock_name = OsString::from(".");
    lock_name.push(path.file_name().context("CLI 清单位置无效")?);
    lock_name.push(".lock");
    let lock_path = checked_path(&path.with_file_name(lock_name))?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(lock_path).context("无法打开 CLI 重绑锁文件")?;
    fs2::FileExt::try_lock_exclusive(&file).context("CLI 清单正被其他进程使用")?;
    if !file.metadata()?.is_file() || file.metadata()?.len() != 0 {
        bail!("CLI 锁文件存在异常内容，已保留原文件");
    }
    // Keep an empty, stable lock file beside the manifest. LockFileEx is
    // mandatory on Windows; locking the manifest itself blocks its readers
    // and editors. Do not unlink this lock on release (that races waiters).
    let original = read_bytes(&path)?;
    let mut manifest = Manifest::read(&path)?;
    let names: Vec<_> = manifest
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    let tools = describe(client, connection, &names).await?;
    if manifest.tools != tools {
        bail!("目标工具契约不同；请核对并重新生成 CLI");
    }
    manifest.dataset_id = dataset(client).await?;
    manifest.connection_id = connection.into();
    manifest.validate()?;
    if digest(&read_bytes(&path)?) != digest(&original) {
        bail!("CLI 清单已被修改，已保留并发修改");
    }
    private_write(&path, &serde_json::to_vec_pretty(&manifest)?)?;
    Ok(json!({"status":"rebound","manifest":manifest}))
}

pub async fn invoke(
    client: &Client,
    manifest: &Manifest,
    tool: &Tool,
    arguments: Value,
) -> Result<Value> {
    if dataset(client).await? != manifest.dataset_id {
        bail!("此 CLI 绑定另一数据目录；请使用 mcplink cli rebind 显式绑定本机连接");
    }
    client.call("tools.call", json!({"id":manifest.connection_id,"tool":tool.name,"schemaHash":tool.schema_hash,"arguments":arguments})).await
}
