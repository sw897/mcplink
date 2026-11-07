---
name: mcplink
description: 通过 mcplink CLI 无 UI 管理 Codex、Hermes 的 MCP，盘点和导入 Claude Code 用户及插件 MCP；管理连接、托管凭据和共享接入，预览、应用及恢复可写来源的配置变更。
---

# mcplink 管理

使用本机 `mcplink` CLI 管理 MCP。CLI 与桌面共用后台、系统凭据库和配置事务；不需要打开桌面。后台按需启动。

## 定位与发现

先定位 `mcplink`（或用户配置的 `MCPLINK_BIN`）。不在 PATH 时可使用已安装 macOS 应用的 `/Applications/mcplink.app/Contents/MacOS/mcplink`。源码开发环境可使用构建好的 `target/release/mcplink`。找不到程序时说明缺少 CLI，按项目 README 的安装入口处理，不改用手工覆写原生配置。

先运行 `mcplink schema` 获取当前命令、参数和 stdin 契约；用 `mcplink status` 获取实际连接、绑定、配置来源和认证状态。`schema` 离线可用。以运行结果中的 ID 为准，不猜连接 ID 或项目来源。

- 默认 stdout 是单个 JSON：成功为 `{"ok":...}`，失败为 `{"error":{"code":"...","message":"..."}}`，部分结果位于 `result`。
- 退出码：`0` 成功，`1` 操作失败或未就绪，`2` 参数或 stdin 无效。`--help`、`--version` 输出普通文本；`bridge` 的 stdout 是 MCP 协议。
- `--data-dir` / `MCPLINK_DATA_DIR` 选择管理数据；`CODEX_HOME` / `HERMES_HOME` 选择原生配置范围。沿用用户指定环境，未经要求不切换到另一套数据。
- `connections list` 包含 `detected:true` 的未托管条目。此类条目先按 `sourceId` 和 `name` 导入，再使用返回的托管 ID。
- Claude Code 自动盘点用户 `.claude.json` 与已启用用户插件默认 `.mcp.json`。这些来源带 `readOnly:true`，可以导入、检查和复用连接；配置由原客户端或插件维护。不要把它们选作写入目标，或直接改写插件缓存。

## 盘点和接入

```sh
mcplink sources list
mcplink connections list
mcplink connections import --source codex:user --name local_notes
mcplink connections show CONNECTION_ID
mcplink connections probe CONNECTION_ID
mcplink connections tools CONNECTION_ID
```

自定义项目/profile 来源用 `sources add --agent codex|hermes|claude --path /absolute/config/file` 添加真实文件。Claude JSON 使用 `mcpServers`，以只读方式纳入。来源 ID 从返回值取得。`CLAUDE_CONFIG_DIR` 可选择 Claude 用户目录。

新增或编辑连接用 `connections save --stdin`。输入为完整的规范化连接对象；编辑时以 `connections show` 的 `definition` 为起点，保留仍需沿用的字段和掩码，连同 `id`、`name` 等元数据提交。原生 `http_headers`、`enabled_tools` 等字段须转换为 `schema` 中的规范字段。保存连接本身不写 Agent 配置。

直连使用 Agent 原生连接能力；共享通过 mcplink Bridge 复用托管凭据。依用户要求选择；共享前必须 `probe` 成功。给目标 Agent 接入、停用、启用或移除时统一使用配置事务：

```sh
mcplink changes preview --connection CONNECTION_ID --target hermes:user --mode shared
mcplink changes apply PLAN_ID
mcplink changes history
mcplink changes restore OPERATION_ID
```

`--connection` 和 `--target` 可重复以批量操作；`--action` 支持 `upsert`、`disable`、`enable`、`remove`。先检查预览里的目标路径、名称、方式和动作，再按用户已有授权应用，不因采用 CLI 额外要求一次确认。预览 10 分钟有效，原生文件或连接修订变化后需重新预览。部分失败时核对 `result.results`，仅处理未完成目标，不盲目重放整批。写入后报告对应 Agent 需要重新加载 MCP 或开启新会话。

`changed:true` 表示原生条目有外部修改，核对后可用 `connections adopt --source SOURCE_ID --name NAME` 采用直连来源。`needsUpdate:true` 表示托管连接已更新而直连条目尚未同步，重新预览并应用。共享条目不作为上游来源采用。

## 认证与排查

`credentials update CONNECTION_ID --stdin` 接受 `{"value":"上游要求的原样凭据","header":"Authorization"}`，默认 header 为 Authorization，不自动加 Bearer。凭据由可信本机来源经 stdin 传入；不要放在命令参数、Shell 历史、聊天回复、临时文件或报告里。`connections save` 中的 headers/env 同样走 stdin。管理输出保留掩码，不尝试读回秘密。

共享连接更新一次凭据即可供其绑定复用；直连绑定需另行通过预览／应用同步。更新后重新 `probe`。`credentials disconnect CONNECTION_ID` 断开托管认证；它不会清除原生客户端已保存的直连凭据。

OAuth 用 `credentials login CONNECTION_ID` 返回 URL，CLI 不打开浏览器；若上游要求用户交互，交给用户完成，随后检查连接。系统凭据库首次访问也可能需要用户在系统安全窗口授权，不能承诺绕过。

`needs_auth` 先处理认证；`disconnected` 先核对是否应恢复；协议或网络故障核对端点和服务。`gateway status|pause|resume` 只影响共享路径，暂停会影响所有共享连接。

需要核对协议能力时使用 `mcp describe CONNECTION_ID`。`mcp request CONNECTION_ID --stdin` 接受 `{"method":"tools/list","params":{}}` 等 MCP 请求；工具调用可能产生业务副作用，应在用户授权范围内执行。失败或通信中断后先核对执行结果，不自动重发。

只删除无绑定的托管连接：先按授权移除目标 Agent 条目，再 `connections delete CONNECTION_ID`。更详细的操作使用 `mcplink COMMAND --help`；底层 `rpc --stdin` 保留兼容，使用它时还须检查嵌套 `results/status`。
