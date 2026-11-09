use anyhow::{Context, Result, bail};
use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use mcplink_core::{
    config, generated,
    runtime::{self, Client},
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs,
    io::{IsTerminal, Write},
    path::PathBuf,
};
use tokio::io::AsyncReadExt;

const SKILL: &str = include_str!("../../../skills/mcplink/SKILL.md");
const INPUT_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Parser)]
#[command(
    name = "mcplink",
    version,
    about = "MCP 管理 CLI；默认输出 JSON，不启动桌面或浏览器"
)]
struct Args {
    /// 管理数据目录；默认使用 MCPLINK_DATA_DIR 或系统应用目录
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 离线输出命令、参数和标准输入契约，不连接后台
    Schema,
    /// 扫描原生配置并返回连接、绑定、来源、历史和后台状态
    Status,
    /// 校验 stdin 参数后调用指定连接的精确工具名，不自动重试
    Call {
        id: String,
        tool: String,
        #[arg(long, required = true)]
        stdin: bool,
        /// 可选的固定工具 Schema 摘要；不匹配时拒绝派发
        #[arg(long)]
        schema_hash: Option<String>,
    },
    /// 生成、检查、重新绑定或运行不含凭据的轻量工具 CLI
    Cli {
        #[command(subcommand)]
        command: CliCommand,
    },
    /// 管理八类支持客户端的原生 MCP 配置来源
    Sources {
        #[command(subcommand)]
        command: SourceCommand,
    },
    /// 管理托管连接；list 包含尚未导入的 detected 条目
    Connections {
        #[command(subcommand)]
        command: ConnectionCommand,
    },
    /// 更新或断开托管凭据，获取 OAuth 授权地址
    Credentials {
        #[command(subcommand)]
        command: CredentialCommand,
    },
    /// 预览、应用、恢复原生配置变更
    Changes {
        #[command(subcommand)]
        command: ChangeCommand,
    },
    /// 查看、暂停或恢复共享 Gateway
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
    /// 查看上游能力或发送一条 MCP 请求；不会自动重试
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// 导出或安装 mcplink 自身的管理 Skill
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    /// 底层管理 RPC；成功响应保持 {"ok":...} 兼容格式
    Rpc {
        #[arg(long, conflicts_with_all = ["method", "params"])]
        stdin: bool,
        method: Option<String>,
        #[arg(long, default_value = "{}")]
        params: String,
    },
    /// 前台运行独立后台（通常由 CLI 按需启动）
    Daemon {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Agent MCP STDIO 入口，stdout 专用于 MCP 协议
    Bridge {
        #[arg(long)]
        connection: String,
    },
}

#[derive(Subcommand)]
enum SourceCommand {
    List,
    /// 添加真实来源；Claude 按作用域定位，OpenCode 使用当前配置格式
    Add {
        #[arg(long, value_parser = ["codex", "hermes", "claude", "opencode", "zcode", "deepseek", "openclaw", "kimicode"])]
        agent: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long, value_parser = ["user", "local", "project"])]
        scope: Option<String>,
        #[arg(long)]
        project: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum CliCommand {
    /// 为明确选定的工具生成 POSIX/Windows 入口和版本化清单
    Generate {
        id: String,
        #[arg(long)]
        name: String,
        /// 尚不存在的目录；父目录须已存在
        #[arg(long)]
        output: PathBuf,
        #[arg(long, required = true)]
        tool: Vec<String>,
    },
    /// 离线检查清单，不打开凭据库或后台
    Inspect { manifest: PathBuf },
    /// 核对全部工具契约后，显式绑定当前数据目录的连接
    Rebind {
        manifest: PathBuf,
        #[arg(long)]
        connection: String,
    },
    /// 生成入口使用的执行命令；-- 后传递工具子命令和参数
    Run {
        #[arg(long)]
        manifest: PathBuf,
        #[arg(last = true)]
        arguments: Vec<OsString>,
    },
}

#[derive(Subcommand)]
enum ConnectionCommand {
    List,
    Show {
        id: String,
    },
    Import {
        #[arg(long)]
        source: String,
        #[arg(long)]
        name: String,
    },
    /// 解除托管绑定并保留 Agent 的原生配置
    Detach {
        id: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        name: String,
    },
    /// 重新启用已断开的连接
    Reconnect {
        id: String,
    },
    /// 采用已绑定直连条目的外部修改
    Adopt {
        #[arg(long)]
        source: String,
        #[arg(long)]
        name: String,
    },
    /// 从 stdin JSON 新建或替换完整连接定义，不写 Agent 配置
    Save {
        #[arg(long, required = true)]
        stdin: bool,
    },
    /// 删除托管连接及其托管凭据
    Delete {
        id: String,
        /// 保留原生文件，并解除与登记内容一致的直连绑定
        #[arg(long)]
        keep_native: bool,
    },
    /// 主动检查上游；非 ready 状态退出 1
    Probe {
        id: String,
    },
    /// 主动检查并返回工具 Schema；非 ready 状态退出 1
    Tools {
        id: String,
        /// 只查看指定工具的 Schema 和摘要，不执行工具
        #[arg(long)]
        tool: Option<String>,
    },
}

#[derive(Subcommand)]
enum CredentialCommand {
    /// stdin JSON: 可选 clientId/clientSecret/scopes/callbackPort；未提供的字段保持原值
    OauthConfig {
        id: String,
        #[arg(long, required = true)]
        stdin: bool,
    },
    /// stdin JSON: {"value":"原样凭据","header":"Authorization"}
    Update {
        id: String,
        #[arg(long, required = true)]
        stdin: bool,
    },
    Disconnect {
        id: String,
    },
    /// 返回授权 URL，不打开浏览器；授权完成后另行 probe
    Login {
        id: String,
    },
}

#[derive(Subcommand)]
enum ChangeCommand {
    /// 生成有效期 10 分钟的脱敏计划，不写原生配置
    Preview {
        #[arg(long, required = true)]
        connection: Vec<String>,
        #[arg(long, required = true)]
        target: Vec<String>,
        #[arg(long, default_value = "direct", value_parser = ["direct", "shared"])]
        mode: String,
        #[arg(long, default_value = "upsert", value_parser = ["upsert", "disable", "enable", "remove"])]
        action: String,
    },
    /// 应用指定预览；逐目标返回结果，任一失败退出 1
    Apply { plan_id: String },
    /// 恢复指定操作的单个条目，保留无关后续修改
    Restore { operation_id: String },
    /// 核对待完成的写入记录
    Reconcile,
    /// 分页读取操作历史；before 使用上一页末项的 cursor
    History {
        #[arg(long)]
        before: Option<i64>,
    },
}

#[derive(Subcommand)]
enum GatewayCommand {
    Status,
    Pause,
    Resume,
    /// 停止已有后台并等待清理；不启动缺席的后台，不改暂停偏好
    Stop,
}

#[derive(Subcommand)]
enum McpCommand {
    Describe {
        id: String,
    },
    /// stdin JSON: {"method":"tools/list","params":{}}
    Request {
        id: String,
        #[arg(long, required = true)]
        stdin: bool,
    },
}

#[derive(Subcommand)]
enum SkillCommand {
    Show,
    /// 安装独立副本；已存在不同内容时拒绝覆盖
    Install {
        #[arg(long, value_parser = ["codex", "hermes", "claude", "opencode"])]
        agent: String,
    },
}

fn command_schema(command: &clap::Command) -> Value {
    let arguments: Vec<_> = command.get_arguments().filter(|arg| arg.get_id() != "help" && arg.get_id() != "version").map(|arg| {
        let values = arg.get_value_parser().possible_values().map(|items| items.map(|v| v.get_name().to_owned()).collect::<Vec<_>>());
        json!({"name":arg.get_id().as_str(),"long":arg.get_long(),"required":arg.is_required_set(),
            "repeatable":matches!(arg.get_action(), ArgAction::Append),"takesValue":matches!(arg.get_action(),ArgAction::Set|ArgAction::Append),
            "default":arg.get_default_values().iter().map(|v|v.to_string_lossy()).collect::<Vec<_>>(),"values":values,"description":arg.get_help().map(ToString::to_string)})
    }).collect();
    json!({"name":command.get_name(),"description":command.get_about().map(ToString::to_string),"arguments":arguments,
        "commands":command.get_subcommands().filter(|c| c.get_name()!="help").map(command_schema).collect::<Vec<_>>()})
}

fn schema() -> Value {
    let mut command = Args::command();
    command.build();
    json!({"schemaVersion":1,"version":env!("CARGO_PKG_VERSION"),"executable":std::env::current_exe().ok(),
        "output":{"success":{"ok":"result"},"failure":{"error":{"code":"error category","message":"safe diagnostic"},"result":"optional partial result"}},
        "exitCodes":{"0":"success","1":"operation failure, non-ready probe, upstream error or partial apply","2":"invalid CLI arguments or stdin"},
        "stdin":{"limitBytes":INPUT_LIMIT,"encoding":"UTF-8 JSON object; close stdin after one request",
            "connections save":{"required":["name","definition"],"optional":["id","title","accountLabel"],
                "definition":{"command":"string (stdio)","args":"string[]","env":"object<string,string>","cwd":"string","url":"string (HTTP)","headers":"object<string,string>","enabled":"boolean (default true)","oauth":"boolean","clientId":"string or null","clientSecret":"string or null; masked on output","oauthScopes":"string[] (empty uses server suggestions)","oauthCallbackPort":"integer 1–65535 or null (automatic)","includeTools":"string[]","excludeTools":"string[]","includeSpecified":"boolean (true to restrict even when includeTools is empty)","timeout":"integer seconds","startupTimeout":"integer seconds","resourcesDisabled":"boolean","promptsDisabled":"boolean"},
                "semantics":"Edit the normalized definition from connections show. Masked credentials stay with their origin. Changing origin suspends access. Metadata edits preserve connection state."},
            "credentials update":{"required":["value"],"optional":["header (default Authorization)"],"semantics":"Value stored verbatim, no Bearer prefix added"},
            "credentials oauth-config":{"optional":{"clientId":"string or null","clientSecret":"string, retained mask or null","scopes":"string[]; empty uses server suggestions","callbackPort":"integer 1–65535 or null"},"semantics":"Partial update. Omitted fields retain values; null clears optional fields. Does not start login or switch Token mode; changes invalidate an existing OAuth grant. Secrets use stdin and the system credential store; direct native export is unavailable for managed OAuth parameters."},
            "mcp request":{"required":["method"],"optional":["params"]},
            "call":{"encoding":"tool arguments JSON object, preserved without string coercion","schema":"upstream inputSchema; drafts 4/6/7/2019-09/2020-12, default 2020-12; external retrieval disabled; format is annotation","invalidArgumentsExitCode":2},
            "rpc --stdin":{"required":["method"],"optional":["params"],"semantics":"Legacy RPC envelope preserved; inspect nested results/status as well as process exit"}},
        "command":command_schema(&command)})
}

async fn stdin_json() -> Result<Value> {
    if std::io::stdin().is_terminal() {
        bail!("请通过标准输入管道传入 JSON，并在一条请求后关闭管道");
    }
    let mut input = Vec::new();
    tokio::io::stdin()
        .take(INPUT_LIMIT + 1)
        .read_to_end(&mut input)
        .await
        .context("无法读取标准输入")?;
    if input.len() as u64 > INPUT_LIMIT {
        bail!("标准输入超过 8 MiB");
    }
    let value: Value = serde_json::from_slice(&input)
        .map_err(|_| anyhow::anyhow!("标准输入必须是一个有效 JSON 对象"))?;
    if !value.is_object() {
        bail!("标准输入必须是一个 JSON 对象");
    }
    Ok(value)
}

fn install_skill(agent: &str) -> Result<Value> {
    let agent_label = if agent == "claude" {
        "Claude Code"
    } else {
        agent
    };
    let root = if agent == "claude" {
        config::claude_home()?.join("skills")
    } else if agent == "opencode" {
        config::opencode_home()?.join("skills")
    } else {
        let source = config::default_sources()?
            .into_iter()
            .find(|s| s.agent.label().eq_ignore_ascii_case(agent_label))
            .context("不支持该 Agent")?;
        source
            .path
            .parent()
            .context("Skill 目录无效")?
            .join("skills")
    };
    let destination = root.join("mcplink");
    for path in [&root, &destination, &destination.join("SKILL.md")] {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("Skill 目标为符号链接，请先核对原有安装");
        }
    }
    let file = destination.join("SKILL.md");
    if file.exists() {
        if fs::read(&file).context("无法读取已有 Skill")? == SKILL.as_bytes() {
            return Ok(json!({"agent":agent,"path":destination,"status":"unchanged"}));
        }
        bail!("目标已有不同的 mcplink Skill，请核对后通过原有 Git/资产流程更新");
    }
    fs::create_dir_all(&destination).context("无法建立 Skill 目录")?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file)
        .context("无法创建 Skill；请核对目标是否发生变化")?;
    output
        .write_all(SKILL.as_bytes())
        .context("无法写入 Skill")?;
    output.sync_all().context("无法保存 Skill")?;
    Ok(
        json!({"agent":agent,"path":destination,"status":"installed","reload":"请重新发现 Skill 或开启新的 Agent 会话"}),
    )
}

struct Failure {
    code: &'static str,
    message: String,
    result: Option<Value>,
}
impl Failure {
    fn input(error: anyhow::Error) -> Self {
        Self {
            code: "invalid_input",
            message: error.to_string(),
            result: None,
        }
    }
    fn operation(error: anyhow::Error) -> Self {
        if let Some(failure) = error.downcast_ref::<mcplink_core::invocation::ToolFailure>() {
            return Self {
                code: failure.code.as_str(),
                message: failure.to_string(),
                result: (!failure.details.is_null()).then(|| failure.details.clone()),
            };
        }
        Self {
            code: "operation_failed",
            message: error.to_string(),
            result: None,
        }
    }
}

async fn execute(command: Command, data: PathBuf) -> Result<Value, Failure> {
    let (method, params, projection, check) = match command {
        Command::Schema => return Ok(schema()),
        Command::Skill {
            command: SkillCommand::Show,
        } => return Ok(json!({"name":"mcplink","content":SKILL})),
        Command::Skill {
            command: SkillCommand::Install { agent },
        } => return install_skill(&agent).map_err(Failure::operation),
        Command::Cli { command } => {
            return match command {
                CliCommand::Inspect { manifest } => generated::Manifest::read(&manifest)
                    .and_then(|manifest| Ok(serde_json::to_value(manifest)?))
                    .map_err(Failure::operation),
                CliCommand::Generate {
                    id,
                    name,
                    output,
                    tool,
                } => {
                    generated::validate_selection(&name, &tool).map_err(Failure::input)?;
                    let client = Client::connect(data).await.map_err(Failure::operation)?;
                    generated::generate(&client, &id, name, &output, &tool)
                        .await
                        .map_err(Failure::operation)
                }
                CliCommand::Rebind {
                    manifest,
                    connection,
                } => {
                    // Validate locally before starting a backend or reading credentials.
                    generated::Manifest::read(&manifest).map_err(Failure::operation)?;
                    let client = Client::connect(data).await.map_err(Failure::operation)?;
                    generated::rebind(&client, &manifest, &connection)
                        .await
                        .map_err(Failure::operation)
                }
                CliCommand::Run { .. } => unreachable!(),
            };
        }
        Command::Status => ("snapshot", json!({}), None, false),
        Command::Call {
            id,
            tool,
            schema_hash,
            ..
        } => (
            "tools.call",
            json!({"id":id,"tool":tool,"arguments":stdin_json().await.map_err(Failure::input)?,"schemaHash":schema_hash}),
            None,
            true,
        ),
        Command::Sources { command } => match command {
            SourceCommand::List => ("snapshot", json!({}), Some("sources"), false),
            SourceCommand::Add {
                agent,
                path,
                label,
                scope,
                project,
            } => (
                "sources.add",
                json!({"agent":agent,"path":path,"label":label,"scope":scope,"project":project}),
                None,
                false,
            ),
        },
        Command::Connections { command } => match command {
            ConnectionCommand::List => ("snapshot", json!({}), Some("connections"), false),
            ConnectionCommand::Show { id } => ("connections.detail", json!({"id":id}), None, false),
            ConnectionCommand::Import { source, name } => (
                "connections.import",
                json!({"sourceId":source,"name":name}),
                None,
                false,
            ),
            ConnectionCommand::Detach { id, source, name } => (
                "connections.detach",
                json!({"id":id,"sourceId":source,"name":name}),
                None,
                false,
            ),
            ConnectionCommand::Reconnect { id } => {
                ("connections.reconnect", json!({"id":id}), None, false)
            }
            ConnectionCommand::Adopt { source, name } => (
                "connections.adopt_source",
                json!({"sourceId":source,"name":name}),
                None,
                false,
            ),
            ConnectionCommand::Save { .. } => (
                "connections.save",
                stdin_json().await.map_err(Failure::input)?,
                None,
                false,
            ),
            ConnectionCommand::Delete { id, keep_native } => (
                "connections.delete",
                json!({"id":id,"keepNative":keep_native}),
                None,
                false,
            ),
            ConnectionCommand::Probe { id } => ("connections.probe", json!({"id":id}), None, true),
            ConnectionCommand::Tools { id, tool: None } => {
                ("connections.tools", json!({"id":id}), None, true)
            }
            ConnectionCommand::Tools {
                id,
                tool: Some(tool),
            } => ("tools.describe", json!({"id":id,"tool":tool}), None, false),
        },
        Command::Credentials { command } => match command {
            CredentialCommand::OauthConfig { id, .. } => (
                "oauth.configure",
                json!({"id":id,"settings":stdin_json().await.map_err(Failure::input)?}),
                None,
                false,
            ),
            CredentialCommand::Update { id, .. } => {
                let mut value = stdin_json().await.map_err(Failure::input)?;
                value["id"] = json!(id);
                ("credentials.update", value, None, false)
            }
            CredentialCommand::Disconnect { id } => {
                ("credentials.disconnect", json!({"id":id}), None, false)
            }
            CredentialCommand::Login { id } => ("oauth.start", json!({"id":id}), None, false),
        },
        Command::Changes { command } => match command {
            ChangeCommand::Preview {
                connection,
                target,
                mode,
                action,
            } => (
                "changes.preview",
                json!({"connectionIds":connection,"targetSourceIds":target,"mode":mode,"action":action}),
                None,
                false,
            ),
            ChangeCommand::Apply { plan_id } => {
                ("changes.apply", json!({"planId":plan_id}), None, true)
            }
            ChangeCommand::Restore { operation_id } => (
                "changes.restore",
                json!({"operationId":operation_id}),
                None,
                false,
            ),
            ChangeCommand::Reconcile => ("changes.reconcile", json!({}), None, false),
            ChangeCommand::History { before } => {
                ("changes.history", json!({"before":before}), None, false)
            }
        },
        Command::Gateway { command } => match command {
            GatewayCommand::Status => ("gateway.status", json!({}), None, false),
            GatewayCommand::Pause => ("gateway.pause", json!({}), None, false),
            GatewayCommand::Resume => ("gateway.resume", json!({}), None, false),
            GatewayCommand::Stop => ("gateway.stop", json!({}), None, false),
        },
        Command::Mcp { command } => match command {
            McpCommand::Describe { id } => ("mcp.describe", json!({"id":id}), None, false),
            McpCommand::Request { id, .. } => (
                "mcp.request",
                json!({"id":id,"request":stdin_json().await.map_err(Failure::input)?}),
                None,
                true,
            ),
        },
        Command::Rpc {
            stdin,
            method,
            params,
        } => {
            let (method, params) = if stdin {
                let value = stdin_json().await.map_err(Failure::input)?;
                (
                    value["method"]
                        .as_str()
                        .context("缺少操作名称")
                        .map_err(Failure::input)?
                        .to_owned(),
                    value.get("params").cloned().unwrap_or_else(|| json!({})),
                )
            } else {
                (
                    method.context("缺少操作名称").map_err(Failure::input)?,
                    serde_json::from_str(&params)
                        .map_err(|_| Failure::input(anyhow::anyhow!("请求参数必须是有效 JSON")))?,
                )
            };
            return call(&data, &method, params)
                .await
                .map_err(Failure::operation);
        }
        Command::Daemon { .. } | Command::Bridge { .. } => unreachable!(),
    };
    if method == "credentials.update" && params["value"].as_str().is_none_or(str::is_empty) {
        return Err(Failure::input(anyhow::anyhow!(
            "凭据 value 必须是非空字符串"
        )));
    }
    let value = call(&data, method, params)
        .await
        .map_err(Failure::operation)?;
    if check {
        let failure = match method {
            "changes.apply"
                if value["results"]
                    .as_array()
                    .is_none_or(|r| r.iter().any(|r| r["status"] != "applied")) =>
            {
                Some((
                    "partial_failure",
                    "部分或全部目标未应用，请核对逐目标结果；不要自动重放",
                ))
            }
            "connections.probe" | "connections.tools"
                if value["connection"]["status"] != "ready" =>
            {
                Some(("connection_unready", "连接尚未就绪，请核对认证及连接状态"))
            }
            "mcp.request" if value.get("rpcError").is_some() || value["isError"] == true => Some((
                "upstream_failed",
                "上游请求失败，执行结果需核对；不要自动重试",
            )),
            "tools.call"
                if value["output"].get("rpcError").is_some()
                    || value["output"]["isError"] == true =>
            {
                Some((
                    "upstream_failed",
                    "上游工具返回失败，执行结果需核对；不要自动重试",
                ))
            }
            _ => None,
        };
        if let Some((code, message)) = failure {
            return Err(Failure {
                code,
                message: message.into(),
                result: Some(value),
            });
        }
    }
    Ok(projection.map(|key| value[key].clone()).unwrap_or(value))
}

async fn call(data: &std::path::Path, method: &str, mut params: Value) -> Result<Value> {
    if !params.is_object() {
        bail!("请求参数必须是 JSON 对象");
    }
    if method == "gateway.stop" {
        return Client::stop_existing(data).await;
    }
    if method == "changes.preview" {
        params["_bridgeExecutable"] =
            json!(std::env::current_exe().context("无法定位共享入口程序")?);
    }
    Client::connect(data.to_owned())
        .await?
        .call(method, params)
        .await
}

async fn generated_run(
    data: PathBuf,
    path: PathBuf,
    args: Vec<OsString>,
) -> Result<Option<Value>, Failure> {
    let manifest = generated::Manifest::read(&path).map_err(Failure::operation)?;
    let (tool, arguments) = match manifest.parse(args).map_err(Failure::input)? {
        generated::Parsed::Help(help) => {
            println!("{help}");
            return Ok(None);
        }
        generated::Parsed::Call { tool, arguments } => (tool, arguments),
    };
    let arguments = match arguments {
        Some(value) => value,
        None => stdin_json().await.map_err(Failure::input)?,
    };
    if serde_json::to_vec(&arguments)
        .map_err(|_| Failure::input(anyhow::anyhow!("参数无法编码")))?
        .len() as u64
        > INPUT_LIMIT
    {
        return Err(Failure::input(anyhow::anyhow!("参数超过 8 MiB")));
    }
    let client = Client::connect(data).await.map_err(Failure::operation)?;
    let value = generated::invoke(&client, &manifest, &tool, arguments)
        .await
        .map_err(Failure::operation)?;
    if value["output"].get("rpcError").is_some() || value["output"]["isError"] == true {
        return Err(Failure {
            code: "upstream_failed",
            message: "上游工具返回失败，执行结果需核对；不要自动重试".into(),
            result: Some(value),
        });
    }
    Ok(Some(value))
}

fn print_result(result: Result<Option<Value>, Failure>) -> i32 {
    match result {
        Ok(Some(value)) => {
            println!("{}", json!({"ok":value}));
            0
        }
        Ok(None) => 0,
        Err(failure) => {
            let mut response = json!({"error":{"code":failure.code,"message":failure.message}});
            if let Some(result) = failure.result {
                response["result"] = result;
            }
            println!("{response}");
            if matches!(failure.code, "invalid_input" | "invalid_tool_arguments") {
                2
            } else {
                1
            }
        }
    }
}

pub async fn run() -> i32 {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            print!("{error}");
            return 0;
        }
        Err(_) => {
            println!(
                "{}",
                json!({"error":{"code":"invalid_arguments","message":"命令参数无效，请运行 mcplink --help 或 mcplink schema"}})
            );
            return 2;
        }
    };
    // Offline discovery and Skill installation must work without Keychain or a daemon.
    let data = match args.data_dir.map(Ok).unwrap_or_else(runtime::data_dir) {
        Ok(path) => path,
        Err(error) => {
            println!(
                "{}",
                json!({"error":{"code":"operation_failed","message":error.to_string()}})
            );
            return 1;
        }
    };
    let result = match args.command {
        Command::Daemon { port } => runtime::daemon(data, port).await,
        Command::Bridge { connection } => crate::bridge(data, connection).await,
        Command::Cli {
            command:
                CliCommand::Run {
                    manifest,
                    arguments,
                },
        } => {
            return print_result(generated_run(data, manifest, arguments).await);
        }
        command => {
            return print_result(execute(command, data).await.map(Some));
        }
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}
