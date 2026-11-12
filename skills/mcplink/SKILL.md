---
name: mcplink
description: 通过 mcplink CLI 管理单服务与聚合入口、托管认证和配置事务，生成工具 CLI，并按授权校验和调用工具。
---

# mcplink 管理

使用本机 `mcplink` CLI 管理 MCP。CLI 与桌面共用后台、系统凭据库和配置事务；不需要打开桌面。后台按需启动。

## 定位与发现

先定位 `mcplink`（或用户配置的 `MCPLINK_BIN`）。不在 PATH 时可使用已安装 macOS 应用的 `/Applications/mcplink.app/Contents/MacOS/mcplink`。源码开发环境可使用构建好的 `target/release/mcplink`。找不到程序时说明缺少 CLI，按项目 README 的安装入口处理，不改用手工覆写原生配置。

先运行 `mcplink schema` 获取当前命令、参数和 stdin 契约；用 `mcplink status` 获取实际连接、绑定、配置来源和认证状态。`schema` 离线可用。以运行结果中的 ID 为准，不猜连接 ID 或项目来源。

支持的客户端、安装状态和管理能力以当前 `schema`、`status` 为准，桌面、CLI 与后台使用同一构建。原生文件客户端未安装时不可写入，配置目录残留不算已安装。需要在客户端确认的配置只生成内容，不代表已安装或已接入。后台在登记原生来源、预览、应用及恢复时复核安装状态与来源权限，不能用其他 Agent 类型或路径绕过只读。

- 默认 stdout 是单个 JSON：成功为 `{"ok":...}`，失败为 `{"error":{"code":"...","message":"..."}}`，部分结果位于 `result`。
- 退出码：`0` 成功，`1` 操作失败或未就绪，`2` 参数或 stdin 无效。`--help`、`--version` 输出普通文本；`bridge` 的 stdout 是 MCP 协议。
- 若当前 `schema` 包含 `call`，可用 `connections tools ID --tool TOOL` 获取指定工具的输入 Schema 与摘要，再以 `call ID TOOL --stdin` 传入参数对象。对象类型原样保留，校验失败退出 `2`。上游错误退出 `1`，工具结果在 `.result.output`；成功时在 `.ok.output`。
- 工具调用使用返回的托管连接 ID 和精确工具名，不按显示名选账号；按本次业务授权执行，不因管理连接的授权而扩大到发送消息或修改业务数据。终端入口不继承原生 Agent 的工具确认流程，`readOnlyHint` 不是授权。
- 需要固定参数契约时传 `--schema-hash HASH`；Schema 或连接变化时停止并重新发现。工具调用超时或通信中断后先核对结果，不自动重发。新 CLI 遇到旧后台时按错误提示使用同一构建重启，不回退绕过校验。
- 若当前 `schema` 包含 `cli generate`，可按明确的连接、工具选择和新目录生成轻量入口；先 `cli inspect MANIFEST` 和离线 `--help` 核对；`cli check MANIFEST` 对照当前契约，不执行工具、不修改文件。复杂或敏感参数用 `--stdin`；自动化可直接以参数数组调用 `cli run --manifest MANIFEST -- TOOL --stdin`。生成物不含凭据，仍依赖本机 mcplink 和当前托管权限；换数据目录须明确选择当地连接后 `cli rebind`，契约变化时重新生成，不按名称自动换账号。
- `--data-dir` / `MCPLINK_DATA_DIR` 选择管理数据；`CODEX_HOME` / `HERMES_HOME` 选择原生配置范围。沿用用户指定环境，未经要求不切换到另一套数据。
- `connections list` 包含 `detected:true` 的未托管条目。此类条目先按 `sourceId` 和 `name` 导入，再使用返回的托管 ID。
- Claude Code 自动盘点用户 `.claude.json` 与已启用用户插件默认 `.mcp.json`。用户来源可通过配置事务写入；插件来源带 `readOnly:true`，可以导入、检查和复用连接，原文件由插件维护。以每个来源返回的权限为准，不直接改写插件缓存。

## HTTP API 工具与核验

若定义含 `httpApi`，它是普通 HTTP API Adapter，不是 HTTP MCP 上游。用 `connections save --stdin` 保存基础 URL、工具参数 Schema、请求/结果映射及托管 headers；不要把凭据放入工具模板或默认值，也不对普通 API 运行 MCP OAuth 发现。

`connections tools ID` 只读取本地工具定义，不执行 HTTP API。`verification.upstream: not_checked` 表示配置有效但上游尚未核验，不是网络错误；`previous_call` 表示最近一次实际调用通过。`verification.probe` 只描述已保存探测的工具、方法和路径，不含托管凭据或参数值。

`connections probe ID` 可能按已保存的 probe 参数执行一次真实业务请求，产生费用或变更。先核对探测操作及本次任务授权，再明确执行；未配置 probe 时不能自行选择工具代替探测。管理连接、保存配置或更新凭据的授权不自动涵盖业务调用。需要显式工具调用时按 `call ID TOOL --stdin` 的同一授权边界处理。桌面“查看工具”和取消探测不执行请求，“运行探测”需要单独确认。超时或结果不明时先核对，不自动重试。

## 盘点和接入

先盘点连接与来源，按具体目标支持的操作完成接入。原生来源通过配置事务维护；需要客户端确认的目标使用当前 Schema 中的 `connections export-config` 生成内容。共享前先检查连接成功；明确需要直连时传 `--mode direct`，此时内容可能含完整凭据，只交给目标客户端，不打印到聊天或报告。配置写入、复制及生成成功都不代表客户端已加载或调用通过。

```sh
mcplink sources list
mcplink connections list
mcplink connections import --source codex:user --name local_notes
mcplink connections show CONNECTION_ID
mcplink connections tools CONNECTION_ID
```

自定义项目/profile 来源用 `sources add --agent AGENT --path /absolute/config/file` 添加真实文件，Agent 选项以当前 `schema` 为准。来源 ID 从返回值取得，`readOnly:true` 的来源只读。`CLAUDE_CONFIG_DIR` 可选择 Claude 用户目录。

若 schema 提供 `--scope`，Claude Code 可写 user 来源；`sources add --agent claude --scope local|project --project /absolute/project` 显式添加项目作用域。Local 按 Git 项目根目录定位 `.claude.json` 的项目键，Project 使用所选目录 `.mcp.json`。只写目标服务，不修改信任、审批或项目启停偏好；插件、托管及其他未明确作用域的 JSON 保持只读。原生启停在 Claude Code `/mcp` 维护，不能向服务条目添加 `enabled`。

若 schema 支持 `opencode`，可用 `sources add --agent opencode --path /absolute/opencode.jsonc`；项目来源另加 `--scope project --project /absolute/project`。只支持当前 `mcp.servers.<name>` / `disabled` 格式；空文件或没有 MCP 设置的文件直接按空服务表读取，无需版本参数。旧版服务表和 `enabled` 字段会被拒绝。JSONC 注释及未涉及设置保持原样。原生专用 OAuth、文件变量、超时或协议选项不能等价导入时保留原客户端配置，另建明确的托管连接；不读取原生登录缓存。文件写入、原生加载和工具调用分别核验。

新增或编辑连接用 `connections save --stdin`。输入为完整的规范化连接对象；编辑时以 `connections show` 的 `definition` 为起点，保留仍需沿用的字段和掩码，连同 `id`、`name` 等元数据提交。原生 `http_headers`、`enabled_tools` 等字段须转换为 `schema` 中的规范字段。保存连接本身不写 Agent 配置。

单服务入口通过 mcplink Bridge 引用一个托管服务；聚合入口引用多个托管服务，Agent 中只配置一个名为 `mcplink` 的条目。两者与 CLI 共用成员连接 ID 对应的认证。原生直连作为可选方式保留，由 Agent 自行连接和维护认证。单服务共享前先 `probe` 成功；聚合入口中的故障服务通过清单 `_meta.mcplink.upstreamErrors` 报告，健康服务仍可使用。给目标 Agent 接入、停用、启用或移除时统一使用配置事务：

```sh
mcplink changes preview --connection CONNECTION_ID --target hermes:user --mode shared
mcplink changes apply PLAN_ID
mcplink changes history
mcplink changes restore OPERATION_ID
```

`--connection` 和 `--target` 可重复以批量操作；`--action` 支持 `upsert`、`disable`、`enable`、`remove`。先检查预览里的目标路径、名称、方式和动作，再按用户已有授权应用，不因采用 CLI 额外要求一次确认。预览 10 分钟有效，原生文件或连接修订变化后需重新预览。部分失败时核对 `result.results`，仅处理未完成目标，不盲目重放整批。写入后报告对应 Agent 需要重新加载 MCP 或开启新会话。

若 `schema` 包含 `gateway save`，使用 stdin 对象 `{"title":"入口名称","connectionIds":["实际服务连接 ID"]}` 创建聚合入口，返回入口 `id`、`members`、`editVersion`。`gateway list` 查看入口。为不同 Agent 提供不同服务时分别创建入口；成员可以引用同一个账号。入口不单独认证，不能嵌套入口。编辑时传 `id`、原 `editVersion` 和完整 `connectionIds`，省略 title 保留名称；保存后选择立即生效，空数组表示不提供任何服务，不表示全部服务。`toolSelections` 可按成员连接 ID 保存原始工具名称数组：null 沿用服务策略，空数组不开放工具，省略保留已存范围。入口只能收窄全局策略，不另建账号。

把返回的入口 ID 作为 `changes preview --connection ENTRY_ID --target SOURCE_ID --mode shared` 的目标。聚合入口不支持 direct。明确需要迁移已有单服务共享条目时加 `--replace-shared`，只替换同一来源中属于入口成员的托管共享条目，保留直连，逐项核对预览与恢复记录。默认不移除已有条目，客户端确认导入的目标需手工核对原有配置。

聚合工具名从 `connections tools ENTRY_ID` 实际发现，不猜服务前缀；可用于 `call` 和 `cli generate`。单服务入口保留原始名称。服务选择更新不重写原生配置，支持的清单会发送变更通知；不能实时刷新的客户端需重载。`gateway refresh ENTRY_ID` 可显式通知指定入口刷新，传服务连接 ID 时通知其引用入口，不调用业务工具。删除入口不删除上游账号；删除被入口引用的服务前先从入口移除。

`changed:true` 表示原生条目有外部修改，核对后可用 `connections adopt --source SOURCE_ID --name NAME` 采用直连来源。`needsUpdate:true` 表示托管连接已更新而直连条目尚未同步，重新预览并应用。共享条目不作为上游来源采用。

## 认证与排查

`credentials update CONNECTION_ID --stdin` 接受 `{"value":"上游要求的原样凭据","header":"Authorization"}`，默认 header 为 Authorization，不自动加 Bearer。凭据由可信本机来源经 stdin 传入；不要放在命令参数、Shell 历史、聊天回复、临时文件或报告里。`connections save` 中的 headers/env 同样走 stdin。管理输出保留掩码，不尝试读回秘密。

同一本机数据目录内，同一服务、账号和授权范围的托管连接，认证一次供单服务入口、多个聚合入口与 CLI 复用。更新一次凭据共同生效；直连绑定需另行通过预览／应用同步，不承诺共用托管 OAuth 登录态。更新后先查看工具与核验状态；普通 MCP 可重新 `probe`，HTTP API 按下述调用边界核验。`credentials disconnect CONNECTION_ID` 停止该连接的托管调用；它不会清除原生客户端已保存的直连凭据。不同服务、账号、数据目录或设备分别维护授权，撤销或无法续期时重新认证。

OAuth 用 `credentials login CONNECTION_ID` 返回 URL，CLI 不打开浏览器；若上游要求用户交互，交给用户完成，随后检查连接。系统凭据库首次访问也可能需要用户在系统安全窗口授权，不能承诺绕过。

若 `schema` 包含 `credentials oauth-config`，可经 stdin 部分更新 `clientId`、`clientSecret`、`scopes` 数组和 `callbackPort`。只使用用户有权使用的注册应用；省略字段保留原值，`null` 清除可选字段，机密掩码表示保留，空 scope 使用服务建议。机密不经过 argv、文件或聊天；配置后再登录。更改已有 OAuth 参数会使旧授权失效，含托管参数的连接使用共享入口。无 DCR 或缺应用机密时按错误补齐，不借用其他客户端身份；手工 Token 成功不代表 OAuth/刷新已通过。

`needs_auth` 先处理认证；`disconnected` 先核对是否应恢复；协议或网络故障核对端点和服务。`gateway status|pause|resume` 只影响共享路径，暂停会影响所有共享连接。

需要核对协议能力时使用 `mcp describe CONNECTION_ID`。`mcp request CONNECTION_ID --stdin` 接受 `{"method":"tools/list","params":{}}` 等 MCP 请求；工具调用可能产生业务副作用，应在用户授权范围内执行。失败或通信中断后先核对执行结果，不自动重发。

默认只删除无绑定的托管连接。若当前 schema 支持 `connections delete CONNECTION_ID --keep-native`，可按授权解除未变化的直连绑定并退出托管，原生条目保留；共享绑定会被拒绝，避免留下无效 Bridge。需要移除原生条目时仍通过配置事务处理。更详细的操作使用 `mcplink COMMAND --help`；底层 `rpc --stdin` 保留兼容，使用它时还须检查嵌套 `results/status`。
