# MCP 转 CLI 与管理接口

MCP 转 CLI 是 mcplink 的核心能力之一：从工具 Schema 发现参数，通过 `call` 直接调用，或用 `cli generate` 为选定工具生成具名命令入口，供终端、脚本和 Agent 使用。复杂参数保留 JSON 类型，凭据由运行时维护。

单服务入口、聚合入口和工具 CLI 引用同一份托管连接及认证。CLI 可调用服务的原始工具名，也可调用聚合入口发现的命名空间工具名。产品定义见[产品定位](product.md)。本文中的命令均描述现有实现。

普通 HTTP API 可以通过 `connections save --stdin` 的 `definition.httpApi` 注册，再复用同样的 `call`、`gateway` 和 `cli generate` 命令；`schema.httpApi` 描述映射契约。工具发现不发出业务请求，配置的 probe 只由显式检查执行，详见 [HTTP API 指南](http-api.md)。

`mcplink` CLI 与桌面使用同一个管理核心、后台、系统凭据库和配置事务。命令不启动桌面或浏览器；后台在首次管理调用时按需启动。`schema` 和 `skill` 离线可用。

客户端与来源支持范围见[Agent 兼容矩阵](agent-discovery.md)。原生文件写入需要客户端已安装，当前 management API 为 22，新的 CLI 契约诊断要求 API 22；桌面、CLI 和后台应使用同一构建。管理 Skill 的复制安装目标另见下文。

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

工具错误退出码依据 MCP 的 RPC 错误或 `isError`。部分服务把业务 HTTP 错误包装在成功的工具正文中，此时 `call` 会原样返回并退出 `0`；自动化须按该服务的契约另行校验业务状态。

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
| `connections export-config --agent AGENT --connection ID [--mode direct\|shared]` | 为需要客户端确认的目标生成配置，默认共享，`--connection` 可重复；不写客户端或建立绑定。直连输出包含完整凭据。 |
| `connections probe ID` / `connections tools ID` | 连接上游、检查认证并返回工具 Schema；非 `ready` 退出 `1`。 |
| `connections delete ID [--keep-native]` | 默认只删除无绑定连接。使用 `--keep-native` 时，所有已登记的绑定都须为直连，且原生条目仍存在、内容与登记时一致。成功后移除托管连接及其凭据，保留原生文件。 |
| `credentials update ID --stdin` / `credentials disconnect ID` | 更新原样 HTTP 凭据，或断开托管认证。 |
| `credentials login ID` | 返回 OAuth URL 和过期时间，不打开浏览器。 |
| `credentials oauth-config ID --stdin` | 配置预注册 OAuth 应用、scope 与回调端口，不启动登录。 |
| `changes preview --connection ID --target SOURCE_ID [--mode direct\|shared] [--action ACTION]` | 生成 10 分钟有效的脱敏计划，不写原生配置。两个标志都可重复。 |
| `changes apply PLAN_ID` | 应用指定计划，检查修订与文件哈希，逐目标报告结果。 |
| `changes reconcile` | 核对已写入但尚未完成记账的记录，返回仍待核对的数量，不重写配置文件。 |
| `changes history [--before CURSOR]` / `changes restore OPERATION_ID` | 每页最多 25 条，用末项 cursor 读取更早记录；按操作 ID 恢复。 |
| `gateway status` / `gateway pause` / `gateway resume` / `gateway stop` | 查看或切换共享状态，或等待当前后台清理退出。 |
| `gateway list` / `gateway save --stdin` | 查看或保存聚合入口的成员服务引用；不复制账号授权。 |
| `mcp describe ID` / `mcp request ID --stdin` | 查看上游能力，或派发一条协议请求，不自动重试。 |
| `connections tools ID --tool TOOL` | 发现指定工具，返回 Schema、描述和 `schemaHash`。 |
| `call ID TOOL --stdin [--schema-hash HASH]` | 校验参数后调用精确工具，不自动重试。 |
| `cli generate ID --name NAME --output NEW_DIRECTORY --tool TOOL ...` | 为明确选定的工具生成轻量入口。 |
| `cli inspect MANIFEST` / `cli check MANIFEST` / `cli rebind MANIFEST --connection ID` | 离线检查清单、比对当前契约，或显式重新绑定当前数据目录中的连接。 |
| `skill show` / `skill install --agent AGENT` | 导出或安装 mcplink 自身管理 Skill。 |

`AGENT` 的可选值按具体命令由 `schema` 列出；来源添加、配置生成与 Skill 安装的目标范围分别定义。`ACTION` 为 `upsert`、`disable`、`enable`、`remove`。目标使用返回的来源 ID，连接使用导入或保存返回的托管 ID。

原生条目已经是本机 mcplink Bridge 时，`connections import` 会关联其指向的已有托管 ID，登记为共享绑定，不另建上游连接或改写原生文件。识别要求绝对路径、标准 Bridge 参数、相同构建和管理数据目录；其他运行环境或未知连接会被拒绝，保留原配置供核对。

Claude Code 支持可写 user，以及显式添加的 local/project 作用域；插件、托管和未明确作用域的其他 JSON 仍只读。OpenCode 仅支持当前 `mcp.servers` JSON/JSONC 来源，空 MCP 配置无需选择版本。参数、覆盖关系、保留格式与转换限制见 [Agent 配置适配](agent-adapters.md)。只读来源不会因以其他 Agent 或路径重复登记而变为可写。

此路径使用 Claude 的原生配置入口和插件目录结构；配置位置与插件格式参照 [Claude MCP 文档](https://code.claude.com/docs/en/mcp) 和 [插件清单文档](https://code.claude.com/docs/en/plugins-reference)。

## 标准输入

编辑已有连接时，取 `connections show ID` 返回的 `definition`，连同其中的 `editVersion` 提交。带路径或参数的地址使用保留标记，启动参数按原编号显示为 `••••••••[1]` 等引用。可以移动引用、删除参数或填写新值。后台按读取时的连接与版本还原，版本过期须重新读取。完整的新定义可以直接通过 stdin 传入。

标准输入必须是一条 UTF-8 JSON 对象，最多 8 MiB，传完后关闭管道。凭据及可能含凭据的完整定义经 stdin 传入，不使用 argv、Shell 字面量、临时凭据文件或日志。

## 统一入口与托管认证

先添加或导入上游连接，完成各自认证。以实际返回的托管连接 ID 创建入口：

```sh
mcplink gateway save --stdin <<'JSON'
{"title":"工作工具","connectionIds":["CONNECTION_A_ID","CONNECTION_B_ID"]}
JSON
mcplink gateway list
mcplink changes preview --connection ENTRY_ID --target hermes:user --mode shared
mcplink changes apply PLAN_ID
mcplink connections tools ENTRY_ID
```

返回的入口含 `id`、`members`、`editVersion`，原生接入名称固定为 `mcplink`。`connections show ENTRY_ID` 返回入口元数据及 `definition:null`，服务选择通过 `gateway save` 更新。为不同 Agent 保存不同入口时，成员可引用相同服务账号。入口不再单独登录：单服务 Bridge、聚合 Bridge 与 CLI 都使用成员连接的 Token 或 OAuth grant。认证复用范围是同一本机数据目录内的同一服务、账号和授权范围；凭据更新、断开及 OAuth 刷新共同生效。

修改入口时传入 `id`、返回的 `editVersion` 及完整 `connectionIds`，省略 `title` 时保留名称。服务选择立即生效；空数组提供空清单，不表示全部服务。旧编辑版本被拒绝，客户端需要刷新或重载清单。入口本身不保存上游认证，删除入口不删除成员账号；被入口引用的连接须先移出入口才能删除。

迁移已有单服务共享配置时，可在上面的预览命令中增加 `--replace-shared`，移除同一目标来源中属于入口成员的已有共享条目，再添加聚合条目。默认不移除；原生直连保留，外部修改与并发仍受保护，恢复按每个原生条目执行。手工导入目标不支持这一自动替换，需要在客户端核对原有条目。

聚合工具名以 `connections tools ENTRY_ID` 实际发现结果为准，不猜前缀。可对该入口执行 `call ENTRY_ID TOOL --stdin` 或 `cli generate ENTRY_ID ... --tool TOOL`，使用同样的参数校验与凭据。资源 URI 和提示名称同样有命名空间。故障上游在 MCP 清单的 `_meta.mcplink.upstreamErrors` 中报告，健康上游继续提供能力。

原生直连使用 `--mode direct`，由 Agent 自行连接并维护认证，不承诺共用托管 OAuth 登录态。聚合入口只允许 `--mode shared`，需要直连时选择具体上游连接。

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
mcplink cli check ./notes-cli/notes.mcplink.json
```

输出目录须尚不存在、父目录须已存在；生成器拒绝符号链接、覆盖和并发占用。目录包含 POSIX 入口 `notes`、Windows PowerShell 入口 `notes.ps1` 和 `notes.mcplink.json`，移动时一起保留。入口使用 PATH 中的 `mcplink`；可用环境变量 `MCPLINK_BIN` 指定可信的二进制完整路径，不导出该路径到清单。Windows 可用系统 PowerShell 运行 `powershell.exe -NoProfile -File ./notes-cli/notes.ps1 read_document --id example-document`，脚本执行遵循系统策略。入口的帮助离线可用，实际调用才连接后台。

参数名称保持原 Schema 属性名。`STRING` 原样传递，`BOOLEAN` 使用 `true`/`false`，`NUMBER`/`INTEGER` 使用 JSON 数值；数组、对象、联合类型以及未声明类型的参数使用一个 JSON 值，帮助标记为 `JSON`。不注入默认值、不转换类型。属性名与 `--stdin`、`--help` 等冲突或不能安全映射时，该工具只提供 `--stdin`。每次调用都由后台按当前 Schema 和工具筛选再次验证。

`--stdin` 与参数标志互斥，输入同样是最多 8 MiB 的 JSON 对象，输出和退出码与 `call` 相同。含 Shell 特殊字符、业务正文或敏感值时优先经 stdin；自动化也可直接调用原生命令 `mcplink cli run --manifest MANIFEST -- TOOL --stdin`，用参数数组及 `input=` 传递，避免 Shell 解析。

版本 1 清单只保存生成器版本、命令名称、数据目录 UUID、托管连接 ID、精确工具名、Schema 摘要及参数名称/类型/必填性；不保存端点、认证头、Token、OAuth grant、完整 Schema、描述、默认值、样例、枚举值或机器路径。清单不是授权凭据，仍须妥善管理命令和工具名称等元数据。检查命令拒绝未知字段、未知版本、损坏或超过 1 MiB 的清单。

`cli inspect` 离线检查文件结构；`cli check` 连接后台，比对当前连接与选定工具的契约，不执行工具、不修改文件。结果包含 `compatible`、`datasetMatches`、`discovery` 及各工具状态：

| 工具状态 | 含义与后续操作 |
| --- | --- |
| `current` | 已完成核验，契约一致。 |
| `changed` | 当前 Schema 或参数契约已变化；查看 `changes` 或无效 Schema 的 `reason`，核对后生成到新目录。 |
| `unavailable` | 该服务已完成发现，但清单不含此工具；检查工具是否移除或被服务/入口范围排除。 |
| `unchecked` | 无法完成核验；先按 `reason` 恢复授权、连接或后台，或明确执行 `cli rebind`，再重新检查。不能据此判断工具已删除。 |

`discovery.status` 为 `complete`、`partial`、`failed` 或 `not_checked`。`diagnostics` 仅返回固定诊断码和安全说明，聚合成员故障另含 `connectionId` 和 `namespace`。常见代码包括 `authentication_required`、`transport_failed`（通信中断或协议不兼容）、`discovery_timeout`、`gateway_paused`、`connection_inactive`、`connection_missing`、`connection_changed`、`tool_discovery_failed` 和 `dataset_mismatch`。不会返回原始上游错误正文、地址或认证值。

聚合发现部分失败时，健康成员仍可核验；故障成员的工具为 `unchecked`。`compatible` 只表示清单中所选工具均为 `current`，所以与所选工具无关的上游故障不使这些命令失效。调用时仍按当前契约和权限再次检查。`cli check` 以退出码 0 返回检查报告；脚本须读取 `compatible`、`discovery` 及逐工具状态，操作本身失败才退出 1。

`changes.parameters` 按参数名返回 `added`、`removed`、`modified` 及可用的 `before` / `after`（名称、类型、必填性）；`stdinOnly` 标明输入方式变化，`schemaChanged` 标明摘要变化。清单不保存完整 Schema，摘要变化但参数未变时仍须核对当前约束；仅 stdin 模式省略参数元数据，不虚构字段差异。契约变化后生成到新目录，旧命令保持原样并在调用时拒绝过期契约。

桌面“工具 CLI”使用同一生成与调用核心。可从托管服务或统一入口打开，查看工具的参数与 Schema、明确勾选工具、选取输出父目录生成，或检查已有清单。实际调用单独触发且一次只派发一次；结果仅保留在当前窗口，不自动重试。

桌面统一入口的工具范围与“工具 CLI”均提供“选择搜索结果”“取消搜索结果”和“清空全部选择”。搜索忽略大小写，前两项只影响当前匹配工具，保留隐藏选择；无匹配项时禁用。发现失败时保留入口已存范围，不根据故障清单自动删除选择。

界面调用拒绝超出 JavaScript 安全整数范围的整数及非有限数，防止解析后改变数值再派发。此类输入使用 `mcplink call --stdin` 或生成 CLI 的 stdin 通道，由核心解析原始 JSON 并校验；界面不会将它改成舍入值或 null 后发送。

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

不支持 DCR 的上游需要预注册应用及其要求的客户端认证。业务权限来自上游与应用注册，scope 按实际契约选择，不能把资源端建议的 scope 当作已获权限。协议和认证限制见[兼容说明](compatibility.md)。

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

`--keep-native` 要求后台管理 API 14，保留各客户端当前的配置内容。若需保留尚未同步的托管更改，可先预览并应用到相应客户端。遇到共享绑定、配置写入待核对或来源无法核实时，先处理对应绑定。

若托管记录已删除但系统凭据库暂不可用，后台会在下次启动时重试清理，也可在恢复访问后重试原删除命令。清理完成后，删除回执仅保留连接 ID 和已解除的绑定数，重试可取回相同结果。

`mcplink connections detach ID --source SOURCE_ID --name ENTRY_NAME` 解除托管绑定并保留原配置文件。修改服务地址的协议、主机或端口后，请重新设置认证，或核对地址后使用 `mcplink connections reconnect ID` 显式恢复访问。仅编辑名称或账号标签会保留连接状态。

先盘点、导入或保存连接；共享接入前完成 `probe`。核对预览中的真实目标路径、动作和接入方式，再按用户授权应用。写入完成后对应 Agent 仍需重新加载 MCP 或开启新会话。

`changed:true` 表示原生条目有外部修改；核对后可采用直连来源。`needsUpdate:true` 表示托管凭据或定义已更新但直连配置未同步，需另行预览／应用。共享连接会复用更新后的托管凭据。

`missing:true` 表示已登记的绑定没有扫描到对应原生条目，不计入当前接入。若来源可读取，可以明确选择 `--action upsert` 预览并重新创建原接入名称，或选择 `--action remove` 清理绑定记录。`unavailable:true` 表示来源无法读取，应先核对来源；仍存在但内容已变化的条目继续受覆盖保护。重新接入同样检查预览后的文件与绑定变化。

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

完整测试入口与可选原生参数见[端到端验证](../tests/e2e/README.md)。

## 入口工具范围与刷新

`gateway save --stdin` 可在 `connectionIds` 之外传 `toolSelections`：键是所选成员连接 ID，值是上游原始工具名数组。编辑要求当前 `editVersion`；省略映射或某个成员保留该成员的范围，null 沿用服务全局策略，空数组表示不开放工具。成员必须在本次服务选择内。入口工具范围与全局策略取交集，单服务入口和其他聚合入口不受影响；所有路径继续共用成员认证。

```json
{"title":"阅读入口","connectionIds":["SERVICE_CONNECTION_ID"],"toolSelections":{"SERVICE_CONNECTION_ID":["read_document"]}}
```

单服务与聚合 Bridge 接收上游清单变更并向相关客户端发送 `list_changed`；配置、认证和入口策略变化也会通知。可以运行 `mcplink gateway refresh ENTRY_ID` 或点击“通知 Agent 刷新”。传服务连接 ID 时，其引用入口一起更新。通知不会调用业务工具，客户端不支持实时刷新时仍需重载 MCP。
