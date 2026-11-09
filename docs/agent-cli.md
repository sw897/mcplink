# Agent 无 UI 管理

`mcplink` CLI 与桌面使用同一个管理核心、后台、系统凭据库和配置事务。命令不启动桌面或浏览器；后台在首次管理调用时按需启动。`schema` 和 `skill` 离线可用。

当前 MCP 接入支持 `codex|hermes|claude|opencode|zcode|deepseek|openclaw|kimicode` 八类客户端，未安装时显示但不可配置，安装后刷新即可接入。配置写入与安装状态校验需要 management API 9，原生路径与逐来源限制见[八类 Agent 接入](agent-discovery.md)。管理 Skill 的复制安装目标另见下文。

## 安装与发现

从源码安装独立 CLI（需要项目 README 中的 Rust 构建环境）：

```sh
cargo install --path crates/mcplink-core --locked
mcplink schema
mcplink skill install --agent codex
mcplink skill install --agent hermes
```

Cargo 默认将程序放入 `~/.cargo/bin`，该目录需在 PATH 中。也可直接使用构建产物 `target/release/mcplink`，或 macOS 应用内的 `/Applications/mcplink.app/Contents/MacOS/mcplink`。共享配置会固定使用发起预览的程序绝对路径，请使用长期保留的位置。

管理 Skill 的源文件为 [`skills/mcplink/SKILL.md`](../skills/mcplink/SKILL.md)，随二进制嵌入，安装时不需要源码目录或网络。安装目标为 Codex、Hermes 或 Claude 的 Skill 目录；指定 `CODEX_HOME`、`HERMES_HOME`、`CLAUDE_CONFIG_DIR` 时使用对应目录，未指定时使用客户端默认目录。源码还支持 OpenCode 配置目录下的 `skills/mcplink`，目录选择见 [Agent 适配](agent-adapters.md)。安装独立副本；相同内容重复执行返回 `unchanged`，不同内容或目标符号链接会被拒绝，不覆盖已有包。后续 Skill 版本可通过用户选择的版本管理流程维护。`mcplink skill show` 返回当前二进制内的 Skill 内容。

安装后在新会话中使用 `$mcplink`，或让支持自动发现的 Agent 按任务选择它。例如：“用 mcplink 把已有文档服务以共享方式接给 Hermes，先核对变更预览，再按本次授权应用。”

## 输出与调用约定

普通命令的 stdout 是一个 JSON 对象，诊断不会夹杂在成功结果中：

```json
{"ok":{"status":"ready"}}
```

```json
{"error":{"code":"partial_failure","message":"部分或全部目标未应用，请核对逐目标结果；不要自动重放"},"result":{"results":[]}}
```

| 退出码 | 含义 |
| --- | --- |
| `0` | 操作成功。 |
| `1` | 操作失败、检查后连接未就绪、上游工具错误或配置批次部分失败。部分结果保存在 `result`。 |
| `2` | CLI 参数或 stdin 格式无效。错误信息不回显输入。 |

`--help`、`--version` 输出普通文本。`bridge` 的 stdout 仅用于 MCP 协议，`daemon` 是前台后台进程。底层 `rpc` 保留原有成功 envelope：管理 RPC 出错现在退出 `1`，但兼容路径不重新解释嵌套的检查状态或批次结果，调用者还须核对它们；新脚本优先使用具名命令。

工具错误退出码依据 MCP 的 RPC 错误或 `isError`。部分服务把业务 HTTP 错误包装在成功的工具正文中，此时 `call` 会原样返回并退出 `0`；自动化须按该服务的契约另行校验业务状态。已观察到的腾讯会议行为见[真实上游验收](real-upstreams.md#mcp-成功与业务成功)。

`mcplink schema` 提供版本化 JSON，包括命令树、参数、允许值、stdin 对象字段、输出和退出码。每条命令也支持 `--help`。

可用全局 `--data-dir /absolute/path` 或 `MCPLINK_DATA_DIR` 指定管理数据。`CODEX_HOME`、`HERMES_HOME` 决定默认原生配置来源；自定义项目/profile 用 `sources add` 显式添加。`MCPLINK_HOME`、`MCPLINK_VAULT=fixture` 是隔离验收环境约定，不应在真实账号环境启用 fixture Vault。

## 命令

| 命令 | 用途 |
| --- | --- |
| `status` | 连接、绑定、来源、最近历史和 Gateway 的完整快照。 |
| `sources list` / `sources add --agent AGENT [--path PATH] [--scope SCOPE] [--project DIR]` | 查看来源或显式添加配置；OpenCode 固定使用当前格式，无版本选择参数。 |
| `connections list` / `connections show ID` | 查看连接摘要或脱敏定义。未导入条目带 `detected:true`。 |
| `connections import --source SOURCE_ID --name NAME` | 将已有原生条目纳入托管，返回托管 ID，不改写原生文件。 |
| `connections adopt --source SOURCE_ID --name NAME` | 明确采用已绑定直连条目的外部修改。 |
| `connections save --stdin` | 新建或完整替换托管定义，不写原生配置。 |
| `connections probe ID` / `connections tools ID` | 连接上游、检查认证并返回工具 Schema；非 `ready` 退出 `1`。 |
| `connections delete ID` | 删除无绑定的托管连接；有绑定时拒绝。 |
| `credentials update ID --stdin` / `credentials disconnect ID` | 更新原样 HTTP 凭据，或断开托管认证。 |
| `credentials login ID` | 返回 OAuth URL 和过期时间，不打开浏览器。 |
| `credentials oauth-config ID --stdin` | 源码开发能力：配置预注册 OAuth 应用、scope 与回调端口，不启动登录。 |
| `changes preview --connection ID --target SOURCE_ID [--mode direct\|shared] [--action ACTION]` | 生成 10 分钟有效的脱敏计划，不写原生配置。两个标志都可重复。 |
| `changes apply PLAN_ID` | 应用指定计划，检查修订与文件哈希，逐目标报告结果。 |
| `changes reconcile` | 核对已写入但尚未完成记账的记录，返回仍待核对的数量，不重写配置文件。 |
| `changes history [--before CURSOR]` / `changes restore OPERATION_ID` | 每页最多 25 条，用末项 cursor 读取更早记录；按操作 ID 恢复。 |
| `gateway status` / `gateway pause` / `gateway resume` / `gateway stop` | 查看或切换共享状态，或等待当前后台清理退出。 |
| `mcp describe ID` / `mcp request ID --stdin` | 查看上游能力，或派发一条协议请求，不自动重试。 |
| `connections tools ID --tool TOOL` | 源码开发能力：发现指定工具，返回 Schema、描述和 `schemaHash`。 |
| `call ID TOOL --stdin [--schema-hash HASH]` | 源码开发能力：校验参数后调用精确工具，不自动重试。 |
| `cli generate ID --name NAME --output NEW_DIRECTORY --tool TOOL ...` | 源码开发能力：为明确选定的工具生成轻量入口。 |
| `cli inspect MANIFEST` / `cli rebind MANIFEST --connection ID` | 离线检查清单，或核对契约后显式重新绑定当前数据目录中的连接。 |
| `skill show` / `skill install --agent AGENT` | 导出或安装 mcplink 自身管理 Skill。 |

`AGENT` 为 `codex`、`hermes`、`claude`、`opencode`、`zcode`、`deepseek`、`openclaw`、`kimicode`。`ACTION` 为 `upsert`、`disable`、`enable`、`remove`。目标使用来源 ID（如 `hermes:user`），连接使用导入或保存返回的托管 ID。

Claude Code 支持可写 user，以及显式添加的 local/project 作用域；插件、托管和未明确作用域的其他 JSON 仍只读。OpenCode 仅支持当前 `mcp.servers` JSON/JSONC 来源，空 MCP 配置无需选择版本。参数、覆盖关系、保留格式与转换限制见 [Agent 配置适配](agent-adapters.md)。只读来源不会因以其他 Agent 或路径重复登记而变为可写。

此路径使用 Claude 的原生配置入口和插件目录结构；配置位置与插件格式参照 [Claude MCP 文档](https://code.claude.com/docs/en/mcp) 和 [插件清单文档](https://code.claude.com/docs/en/plugins-reference)。

## 标准输入

编辑已有连接时，取 `connections show ID` 返回的 `definition`，连同其中的 `editVersion` 提交。带路径或参数的地址使用保留标记，启动参数按原编号显示为 `••••••••[1]` 等引用。可以移动引用、删除参数或填写新值。后台按读取时的连接与版本还原，版本过期须重新读取。完整的新定义可以直接通过 stdin 传入。

标准输入必须是一条 UTF-8 JSON 对象，最多 8 MiB，传完后关闭管道。凭据及可能含凭据的完整定义经 stdin 传入，不使用 argv、Shell 字面量、临时凭据文件或日志。

## 通用工具调用

先运行 `schema` 确认命令契约，并让 CLI 与后台使用同一构建。

```sh
mcplink connections tools CONNECTION_ID --tool read_document
```

返回 `connectionId`、`connectionRevision`、指定工具的完整 `tool` 及 `schemaHash`。工具发现遍历分页，使用连接当前筛选；重复名称、循环游标、超过 100 页或 10,000 项时拒绝调用。连接 ID 必须是已有托管 ID，不按显示名回退选择账号。

调用参数直接是工具的输入对象，而不是 MCP 请求 envelope。例如，用程序构造 stdin：

```python
import json
import subprocess

connection_id = "CONNECTION_ID"  # 使用导入或保存返回的托管 ID
result = subprocess.run(
    ["mcplink", "call", connection_id, "read_document", "--stdin"],
    input=json.dumps({"id": "example-document"}),
    text=True, capture_output=True, check=False,
)
response = json.loads(result.stdout)
```

成功结果位于 `.ok`，含 `connectionId`、`tool`、`schemaHash` 和未改写的 MCP `output`。上游 `isError` 或 RPC 错误退出 `1`，MCP 结果保存在 `.result.output`。CLI/工具参数错误退出 `2`；Schema 不可验证、摘要变化、工具不可用、连接变化或通信失败退出 `1`。

参数保留嵌套对象、数组、布尔值、数值和 null。使用 jsonschema 0.60.2 实现的 Draft 4/6/7/2019-09/2020-12 校验，未声明时使用 2020-12；支持文档内引用，拒绝获取外部 HTTP/文件引用和不可解析的自定义方言。`format` 作为注解，不强制断言；不注入默认值、不转换参数类型，不实现自定义词汇的校验逻辑。Schema 最大 1 MiB，错误最多给出 16 个 Schema 位置，不回显参数值。

`--schema-hash` 可固定发现时的输入 Schema 摘要；不匹配时在工具派发前失败。发现期间连接修订变化同样失败。固定摘要采用保守策略：输入 Schema 的任何变化均需重新核对，即使变化可能兼容。

终端调用不继承原生 Agent 的工具确认流程。按当前任务授权核对工具与参数，不能把工具的 `readOnlyHint` 当作授权依据。超时或连接中断后先核对执行结果，不自动重发。

## 按连接生成 CLI

先核对当前二进制的 `schema`，并让后台使用同一构建。

```sh
mcplink cli generate CONNECTION_ID --name notes --output ./notes-cli --tool search_documents --tool read_document
./notes-cli/notes --help
./notes-cli/notes read_document --help
./notes-cli/notes read_document --id example-document
mcplink cli inspect ./notes-cli/notes.mcplink.json
```

输出目录须尚不存在、父目录须已存在；生成器拒绝符号链接、覆盖和并发占用。目录包含 POSIX 入口 `notes`、Windows PowerShell 入口 `notes.ps1` 和 `notes.mcplink.json`，移动时一起保留。入口使用 PATH 中的 `mcplink`；可用环境变量 `MCPLINK_BIN` 指定可信的二进制完整路径，不导出该路径到清单。Windows 可用系统 PowerShell 运行 `powershell.exe -NoProfile -File ./notes-cli/notes.ps1 read_document --id example-document`，脚本执行遵循系统策略。入口的帮助离线可用，实际调用才连接后台。

参数名称保持原 Schema 属性名。`STRING` 原样传递，`BOOLEAN` 使用 `true`/`false`，`NUMBER`/`INTEGER` 使用 JSON 数值；数组、对象、联合类型以及未声明类型的参数使用一个 JSON 值，帮助标记为 `JSON`。不注入默认值、不转换类型。属性名与 `--stdin`、`--help` 等冲突或不能安全映射时，该工具只提供 `--stdin`。每次调用都由后台按当前 Schema 和工具筛选再次验证。

`--stdin` 与参数标志互斥，输入同样是最多 8 MiB 的 JSON 对象，输出和退出码与 `call` 相同。含 Shell 特殊字符、业务正文或敏感值时优先经 stdin；自动化也可直接调用原生命令 `mcplink cli run --manifest MANIFEST -- TOOL --stdin`，用参数数组及 `input=` 传递，避免 Shell 解析。

版本 1 清单只保存生成器版本、命令名称、数据目录 UUID、托管连接 ID、精确工具名、Schema 摘要及参数名称/类型/必填性；不保存端点、认证头、Token、OAuth grant、完整 Schema、描述、默认值、样例、枚举值或机器路径。清单不是授权凭据，仍须妥善管理命令和工具名称等元数据。检查命令拒绝未知字段、未知版本、损坏或超过 1 MiB 的清单。

复制到另一个数据目录或设备后，先创建当地的托管连接，再显式重新绑定：

```sh
mcplink cli rebind ./notes-cli/notes.mcplink.json --connection LOCAL_CONNECTION_ID
```

重新绑定核对所有选定工具的 Schema 摘要及参数契约，原子更新清单，并检测并发修改。会在清单旁保留空的 `.清单文件名.lock`，用于跨数据目录的并发互斥；移动入口时无需复制它，操作期间不要删除。原连接丢失时不会按显示名选择其他账号。上游输入 Schema 的任何变化均要求重新核对、重新生成；工具筛选、连接停用和 Gateway 暂停即时生效。托管凭据更新、OAuth 刷新和后台重启不要求重新生成。

终端和生成入口使用同样的任务授权边界，不继承原生 Agent 的确认步骤；工具结果不确定时先核对，入口不自动重发。

`gateway status` 使用轻量后台查询，不扫描原生配置。暂停和恢复在 SQLite 持久化成功后才返回，后台或电脑重启不会自动解除暂停；这些操作需要 management API 10。Windows 与 macOS 可在桌面“后台运行”或托盘/菜单栏设置登录自启，自启时隐藏主窗口；关闭主窗口不退出代理。

`gateway stop` 需要 management API 11，等待上游进程树清理和后台运行锁释放；后台原本未运行时直接成功，不启动后台或创建数据目录。它不改变持久化的暂停偏好，后续 Agent 调用仍可按需启动后台。桌面仍在运行时也会通过巡检恢复后台；需要停止并退出应用时使用桌面菜单。持续阻止共享调用请使用 `gateway pause`。退出进行中的调用会报告结果待核对，不自动重试。

## OAuth 客户端配置

`credentials oauth-config ID --stdin` 接受部分更新对象，可选 `clientId`、`clientSecret`、`scopes`（字符串数组）和 `callbackPort`（1–65535）。省略字段保留原值；`clientId`、`clientSecret`、`callbackPort` 可用 `null` 清除。机密的 `••••••••` 掩码表示保留已存值。`scopes: []` 使用服务建议的 scope，回调端口留空自动分配，回调路径为 `/callback`。

使用你有权使用的注册应用和权限。机密经 stdin 传入系统凭据库，管理输出只返回掩码；不将机密写入命令参数或文件。配置完成后运行 `credentials login ID`，由用户打开返回的 URL 同意授权，再运行 `connections probe ID`。支持 DCR 的上游可不提供预注册应用；不支持 DCR 的上游须先配置 Client ID 及其要求的客户端机密。机密客户端的 `client_secret_basic` / `client_secret_post` 按发现的元数据选择。

配置本身保留当前 Token/OAuth 模式。更改已有 OAuth 的端点、应用身份、机密、scope 或回调配置会使旧 grant 失效，须重新授权。授权发现、注册或回调监听准备失败时保留原认证；返回授权 URL 后进入新的授权会话。含托管 OAuth 参数的连接使用共享入口，不能直接转换为原生 Agent 配置。

GitHub 远程 MCP 不提供 DCR，授权码流程要求预注册应用及客户端认证。GitHub App 的业务权限来自应用注册，scope 按应用支持情况选择；不要把资源端建议的 scope 当作应用已获权限。真实 Token 与 OAuth 验证范围分别见[兼容说明](compatibility.md)。

## 其他 stdin 对象

以下演示创建无凭据 STDIO 连接；将命令路径替换为实际服务。Agent 使用参数数组和 `input=`，可以避免 Shell 转义问题：

```python
import json
import subprocess

result = subprocess.run(
    ["mcplink", "connections", "save", "--stdin"],
    input=json.dumps({
        "name": "local_notes",
        "title": "本地笔记",
        "definition": {
            "command": "/absolute/path/to/mcp-server",
            "args": [],
            "enabled": True,
        },
    }),
    text=True, capture_output=True, check=False,
)
response = json.loads(result.stdout)
if result.returncode:
    raise RuntimeError(response["error"]["message"])
connection_id = response["ok"]["id"]
```

其他输入对象：

- `connections save`：必需 `name`、`definition`，编辑时另带 `id`，可选 `title`、`accountLabel`。这是完整替换；编辑时从 `connections show` 的脱敏定义开始，保留需沿用的字段和掩码。定义字段见 `schema`，使用 `headers`、`includeTools` 等规范名称。
- `credentials update ID`：必需 `value` 字符串，可选 `header`，默认 `Authorization`；不自动添加 Bearer。值从可信本机来源进入进程内存，再用上述 `input=` 方式交给 CLI。
- `mcp request ID`：`{"method":"tools/list","params":{}}` 等协议请求。工具调用可能有业务副作用；CLI 不自动重发失败或超时的调用。
- `rpc --stdin`：`{"method":"snapshot","params":{}}`。一般不需要使用兼容入口。

## 接入和恢复

先盘点、导入或保存连接；共享接入前完成 `probe`。核对预览中的真实目标路径、动作和接入方式，再按用户授权应用。写入完成后对应 Agent 仍需重新加载 MCP 或开启新会话。

`changed:true` 表示原生条目有外部修改；核对后可采用直连来源。`needsUpdate:true` 表示托管凭据或定义已更新但直连配置未同步，需另行预览／应用。共享连接会复用更新后的托管凭据。

过期预览或并发修改不会被覆盖；部分失败时根据 `result.results` 核对已完成目标，为待处理目标重新预览。通信中断时结果可能已经写入，先查历史及当前状态，不自动重放。

OAuth 的最终同意可能要求用户交互，系统凭据库首次访问也可能要求系统安全确认。无 UI 管理并不替代这些授权；CLI 返回 URL 或明确故障，由用户在对应系统流程完成授权后继续。

## 验证

```sh
npm run check
python3 tests/e2e/agent_cli.py --record artifacts/e2e/agent-cli/local-run
python3 tests/e2e/run.py
python3 tests/e2e/claude_sources.py --record artifacts/e2e/claude/local-run
python3 tests/e2e/tool_calls.py --record artifacts/e2e/tool-calls/local-run
python3 tests/e2e/generated_cli.py --record artifacts/e2e/generated-cli/local-run
```

测试使用隔离配置、合成 MCP/OAuth 与 fixture Vault，不修改日常 Agent 配置。报告记录命令、退出码、断言和二进制／Skill 哈希，输出到忽略的 `artifacts/` 目录。默认测试不要求真实账号或模型登录；`run.py --native-agents` 可额外检查已安装的原生客户端命令。
