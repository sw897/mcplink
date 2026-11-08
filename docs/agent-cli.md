# Agent 无 UI 管理

`mcplink` CLI 与桌面使用同一个管理核心、后台、系统凭据库和配置事务。命令不启动桌面或浏览器；后台在首次管理调用时按需启动。`schema` 和 `skill` 离线可用。

## 安装与发现

从源码安装独立 CLI（需要项目 README 中的 Rust 构建环境）：

```sh
cargo install --path crates/mcplink-core --locked
mcplink schema
mcplink skill install --agent codex
mcplink skill install --agent hermes
```

Cargo 默认将程序放入 `~/.cargo/bin`，该目录需在 PATH 中。也可直接使用构建产物 `target/release/mcplink`，或 macOS 应用内的 `/Applications/mcplink.app/Contents/MacOS/mcplink`。共享配置会固定使用发起预览的程序绝对路径，请使用长期保留的位置。

管理 Skill 的源文件为 [`skills/mcplink/SKILL.md`](../skills/mcplink/SKILL.md)，随二进制嵌入，安装时不需要源码目录或网络。安装目标是 `CODEX_HOME/skills/mcplink` 或 `HERMES_HOME/skills/mcplink`，未指定时使用用户默认目录。安装独立副本；相同内容重复执行返回 `unchanged`，不同内容或目标符号链接会被拒绝，不覆盖已有包。后续 Skill 版本可通过用户选择的版本管理流程维护。`mcplink skill show` 返回当前二进制内的 Skill 内容。

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

`mcplink schema` 提供版本化 JSON，包括命令树、参数、允许值、stdin 对象字段、输出和退出码。每条命令也支持 `--help`。

可用全局 `--data-dir /absolute/path` 或 `MCPLINK_DATA_DIR` 指定管理数据。`CODEX_HOME`、`HERMES_HOME` 决定默认原生配置来源；自定义项目/profile 用 `sources add` 显式添加。`MCPLINK_HOME`、`MCPLINK_VAULT=fixture` 是隔离验收环境约定，不应在真实账号环境启用 fixture Vault。

## 命令

| 命令 | 用途 |
| --- | --- |
| `status` | 连接、绑定、来源、最近历史和 Gateway 的完整快照。 |
| `sources list` / `sources add --agent AGENT --path PATH [--label LABEL]` | 查看来源，添加真实配置文件。 |
| `connections list` / `connections show ID` | 查看连接摘要或脱敏定义。未导入条目带 `detected:true`。 |
| `connections import --source SOURCE_ID --name NAME` | 将已有原生条目纳入托管，返回托管 ID，不改写原生文件。 |
| `connections adopt --source SOURCE_ID --name NAME` | 明确采用已绑定直连条目的外部修改。 |
| `connections save --stdin` | 新建或完整替换托管定义，不写原生配置。 |
| `connections probe ID` / `connections tools ID` | 连接上游、检查认证并返回工具 Schema；非 `ready` 退出 `1`。 |
| `connections delete ID` | 删除无绑定的托管连接；有绑定时拒绝。 |
| `credentials update ID --stdin` / `credentials disconnect ID` | 更新原样 HTTP 凭据，或断开托管认证。 |
| `credentials login ID` | 返回 OAuth URL 和过期时间，不打开浏览器。 |
| `changes preview --connection ID --target SOURCE_ID [--mode direct\|shared] [--action ACTION]` | 生成 10 分钟有效的脱敏计划，不写原生配置。两个标志都可重复。 |
| `changes apply PLAN_ID` | 应用指定计划，检查修订与文件哈希，逐目标报告结果。 |
| `changes history` / `changes restore OPERATION_ID` | 查看最近 25 条历史，恢复指定条目并保留无关后续修改。 |
| `gateway status` / `gateway pause` / `gateway resume` | 查看或切换所有共享连接的后台状态。 |
| `mcp describe ID` / `mcp request ID --stdin` | 查看上游能力，或派发一条协议请求，不自动重试。 |
| `connections tools ID --tool TOOL` | 源码开发能力：发现指定工具，返回 Schema、描述和 `schemaHash`。 |
| `call ID TOOL --stdin [--schema-hash HASH]` | 源码开发能力：校验参数后调用精确工具，不自动重试。 |
| `skill show` / `skill install --agent AGENT` | 导出或安装 mcplink 自身管理 Skill。 |

`AGENT` 为 `codex`、`hermes`、`claude`。`ACTION` 为 `upsert`、`disable`、`enable`、`remove`。目标使用来源 ID（如 `hermes:user`），连接使用导入或保存返回的托管 ID。

Claude Code 来源标记为 `readOnly:true`：支持用户 `.claude.json` 中的 `mcpServers`、已启用用户插件默认 `.mcp.json` 的包裹或裸映射形式。`CLAUDE_CONFIG_DIR` 指定用户目录；设置与插件安装登记用于发现当前用户插件。可按来源 ID 导入、检查和复用连接，原文件保持不变，写入目标选择器会禁用只读来源。`sources add --agent claude` 可补充只读 JSON 文件。项目或企业配置优先级、其他插件入口和云连接器不自动推断。

此路径使用 Claude 的原生配置入口和插件目录结构；配置位置与插件格式参照 [Claude MCP 文档](https://code.claude.com/docs/en/mcp) 和 [插件清单文档](https://code.claude.com/docs/en/plugins-reference)。

## 标准输入

标准输入必须是一条 UTF-8 JSON 对象，最多 8 MiB，传完后关闭管道。凭据及可能含凭据的完整定义经 stdin 传入，不使用 argv、Shell 字面量、临时凭据文件或日志。

## 通用工具调用（源码开发能力）

以下入口未包含在 v0.1.0 发布包中。先运行当前程序的 `schema` 确认命令；后台也须使用同一构建。新 CLI 连接旧后台时会明确失败，不会重放成其他命令或自动停止旧后台。

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

参数保留嵌套对象、数组、布尔值、数值和 null。支持 JSON Schema Draft 4/6/7/2019-09/2020-12，未声明时使用 2020-12；支持文档内引用，拒绝获取外部 HTTP/文件引用。`format` 作为注解，不强制断言；不注入默认值、不转换参数类型。Schema 最大 1 MiB，错误最多给出 16 个 Schema 位置，不回显参数值。

`--schema-hash` 可固定发现时的输入 Schema 摘要；不匹配时在工具派发前失败。发现期间连接修订变化同样失败。固定摘要采用保守策略：输入 Schema 的任何变化均需重新核对，即使变化可能兼容。

终端调用不继承原生 Agent 的工具确认流程。按当前任务授权核对工具与参数，不能把工具的 `readOnlyHint` 当作授权依据。超时或连接中断后先核对执行结果，不自动重发。

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
```

测试使用隔离配置、合成 MCP/OAuth 与 fixture Vault，不修改日常 Agent 配置。报告记录命令、退出码、断言和二进制／Skill 哈希，输出到忽略的 `artifacts/` 目录。默认测试不要求真实账号或模型登录；`run.py --native-agents` 可额外检查已安装的原生客户端命令。
