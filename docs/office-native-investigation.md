# 千问办公、豆包工作原生管理调查

调查日期：2026-11-10。目标是核对个人桌面客户端的本地配置和管理入口，判断能否接入 mcplink 的预览、应用、冲突检查与恢复流程。

登录后实机确认：**千问办公当前共享 JSON 模式可以原生管理，已补齐适配；豆包工作保留客户端导入。** 千问的身份隔离或迁移存储使用导入回退。两款都已完成实际工具调用；下面区分实机结果、安装包代码证据及尚未验证的其他存储模式，不把缓存或未验证入口视为有效配置契约。

## 安装包与安装识别

安装包均来自[千问办公官方下载页](https://qwenwork.cn/download)和[豆包工作官网](https://www.doubao.com/work)。macOS 包完成 Developer ID 深度签名校验后安装、启动；Windows 包只检查分发内容或安装器元数据，没有在 macOS 执行 Windows 程序。

| 对象 | 观察结果 |
| --- | --- |
| 千问办公 macOS | 2.0.0，build 26110902；`QwenWorkCN.app`，主程序 `QwenWorkCN`，bundle ID `cn.qwenwork.desktop.mac`；签名 Taobao (China) Software CO.,LTD，Team ID `XN6U3EV979` |
| 豆包工作 macOS | 2.36.3；`DoubaoWork.app`，主程序 `DoubaoWork`，bundle ID `com.work.pc.doubao`；签名 Beijing Chuntian Zhiyun Technology Co., Ltd.，Team ID `96L78H6LMH` |
| 千问办公 Windows | 官方 `qwenworkcn-setup-x64.exe` 内的应用载荷包含 `QwenWorkCN.exe` |
| 豆包工作 Windows | 官网 Windows 下载器的产品配置为 `DoubaoWork` / `DoubaoWork.exe`，安装目录名 `DoubaoWork`；下载器版本 2.25.18，不代表最终下载的应用版本 |

据此修正安装探测：识别 `QwenWorkCN` 和 `DoubaoWork`，保留千问官方安装指南使用的 `QwenWork.app` 名称及中文名称。普通豆包的 `Doubao.app`、`豆包.app`、`Doubao.exe` 不再算作豆包工作。目录存在但缺少实际程序时仍显示未安装。

## 千问办公

### 数据路径和存储方式

[官方 macOS 安装指南](https://qwenwork.cn/docs/install/macos)列出 `~/.qwenworkcn` 数据目录。实际启动还创建 `~/Library/Application Support/QwenWorkCN`；两者不能合并理解成一个 MCP JSON 文件。

在此次 2.0.0 包中核对到：

- `out/main/main.js` 的 `CUSTOM_MCP_CONFIG_PATH` 指向 `~/.qwenworkcn/mcp.json`。`readCustomMcpConfig` 同时包含共享 JSON 兼容分支和身份隔离分支，由运行模式选择；出现这个路径不能证明当前账号继续使用 JSON。
- 身份隔离分支使用当前身份数据根目录下的 `data/agents.identity-v2.db`。迁移文件 `0074_identity_connector_sqlite_backflow.sql` 定义 `identity_connector_heads`、`identity_connector_records` 和 `identity_connector_preferences`。连接器定义与偏好分别存为 `encrypted_definition`、`encrypted_value`。
- `IdentityConnectorSqliteRepository` 通过 Electron `safeStorage` 加解密，校验加密内容中的身份和记录键，提交时校验 revision，并通知客户端运行时。身份加密文件的兼容位置为 `identities/<身份摘要>/connectors/config.enc`，也不是可直接编辑的导入 JSON。
- 首次未登录启动实际创建的是 `data/agents.db`，其中也有上述迁移表。**数据库里有表不等于身份隔离模式已启用**；也不能把未登录时的兼容行为推广到登录后的账号。
- 用户登录后，通过官方 JSON 导入保存唯一名称的合成连接器，实际生成 `~/.qwenworkcn/mcp.json`，身份连接器表记录数仍为零。客户端已发现合成工具并完成模型调用。
- 正常退出后，仅修改这条测试配置的启动参数，再启动客户端；其 MCP 运行时返回修改后的标记，证明外部 JSON 写入是当前账号的有效入口。

据此实现共享 JSON 原生适配：严格 JSON 的 `mcpServers`，支持 STDIO、Streamable HTTP、`enabled`，接入既有预览、并发检查、应用、启停及恢复流程。保留偏好和 `cliConnectors`，拒绝跨桶同名、SDK 路径和未验证的执行字段；不将 JSONC 写给使用 `JSON.parse` 的客户端。

每次盘点、读取和变更均检查身份格式标记、迁移目录及身份数据库；对共享 `agents.db` 只检查身份连接器表是否存在记录，不读取定义、凭据或身份值。发现身份数据或无法核对模式时，旧 JSON 来源只读保留、界面说明原因并回退到导入；预览后切换模式也不能继续应用或恢复。该保护不声称已适配加密身份存储。

### 已发现的管理入口

| 入口 | 能力和边界 |
| --- | --- |
| 桌面 tRPC / Electron IPC | `updateCustomMcpServer`、启停、移除等操作存在于客户端内部，带身份作用域。它不是已文档化的独立 HTTP 管理服务；不以调试端口注入 Electron IPC 作为产品适配方式 |
| 应用内 Connector 工具 | 随包 `guide-mcp.md` 描述 `qw_query` / `qw_action` 的自定义连接器 CRUD。对应实现要求原生确认；自动添加/更新 STDIO 的允许命令为 `npx`、`uv`、`uvx`，任意 mcplink Bridge 路径会要求手动粘贴。身份隔离运行时的能力凭据还绑定身份、聊天及子任务，不能复用为长期管理令牌 |
| 本地 MCP adaptor | 配置在 `~/.qwenworkcn/mcp-adaptor.config`。未登录时返回 33 项工具；登录后返回 35 项，增加 `qw_query` / `qw_action`。已用原生 `qw_query` 核对测试服务 connected 状态，并通过客户端 MCP 运行时实际调用测试工具。只在内存使用应用生成的本地 MCP 能力凭据，没有导出账号登录缓存 |
| 内置 Agent SDK CLI | 签名应用内的 `flash-cli-arm64` 实际支持 `mcp add/list/auth/logout/debug`。`debug paths` 显示默认配置根目录 `~/.config/qwenwork`；桌面运行时另行提供配置。CLI 命令存在不等于它管理桌面的身份连接器注册表，不把 SDK 或独立 Qwen Code 的配置路径用作桌面适配依据 |
| 企业 OpenAPI | [创建连接器 API](https://docs.qwenwork.cn/developer/ai-assets/connectors/manage/create-connectors.md)为企业市场资源提供 `POST /api/openapi/v1/connectors`，需要企业部署地址、API Key、`admin.v1.connectors.manage` 权限和幂等键。它不是个人桌面自定义连接器的本地配置接口；本次未调用企业写接口 |
| MCP 添加深链接 | 包内有携带名称和配置的添加入口，作用是打开客户端确认流程；本次未将此入口作为原生适配，也不把跳转视为写入成功 |

本次选择已经实机验证的 JSON 文件入口。当前账号的该模式已由 mcplink 实际应用共享 Bridge 和 HTTP 直连，重启后均调用成功。身份隔离模式仍需单独的管理契约和验收；不会通过包装命令或复用任务能力凭据绕过应用内工具的确认和 STDIO 限制。

## 豆包工作

### 数据路径和管理流程

实际 macOS 数据根为 `~/Library/Application Support/DoubaoWork`，包含 Chromium profile、缓存及 `sdk_storage/common.db`。该数据库未发现 MCP/connector 表。保存并重启后，合成名称可见于 `Default/IndexedDB/`、`Default/saman_shell_db_storage/` 和测试会话记录；这些位置包含缓存与运行状态，不能仅凭找到名称就当成可直接修改的配置文件。

2.36.3 包中，`local_webcontents/extensions/ai-views/static/js/async/64158.js` 定义个人连接器创建与更新请求；编辑界面调用它们保存表单：

- `POST /alice/office/skills/manage/connector/create_personal`
- `POST /alice/office/skills/manage/connector/update_personal`

负载包含名称、描述、图标、`mcp_config`，更新还需要 `connector_id`；同类客户端业务代码也出现在内置服务中。因此，个人连接器至少有云端保存流程，不能用写某个本地 JSON 来等价替代。不能仅凭 HTTP 路径就实现可靠适配：还缺少面向第三方的个人账号认证、查询/启停/删除、并发校验和恢复契约。

进一步核对 `31828.js` 的创建、更新和列表流程：先调用 `AGWManageCreatePersonalConnector` / `AGWManageUpdatePersonalConnector` / `AGWManageListUserConnectors`，再请求本地环境刷新并检查运行状态；本地激活失败时，错误会标记 `cloudMutationApplied`，说明云端提交与本地启动是两步操作。它们不能直接套用 mcplink 的单文件事务。

同一管理模块通过 `window.neotix.taskMode.connector` 调用的本地桥接操作是查询、重试、授权及取消授权（包括本地 HTTP 授权），未提供个人配置的独立创建/更新/删除接口。该桥接属于应用页面与原生宿主之间的内部调用，尚未找到可供外部程序独立连接的个人管理端点。**这是当前无法按原生文件方式接管的依据，不是“没有安装”或“没有查到路径”。** 若另做云端账号集成，还需要正式认证、版本、并发和回滚契约；不据此断言厂商以后不能提供自动管理能力。

没有复制登录 Cookie、账号令牌或原生请求签名来调用这些写接口；也没有把浏览器调试端口或逐按钮自动操作作为原生适配。

### 独立豆包 CLI

[官方 CLI 文档](https://www.doubao.com/work/docs/zh-cn/doubao-cli)说明其账号使用条件；[命令参考](https://www.doubao.com/work/docs/zh-cn/articles/376493082444-cli-commands)提供账号与会话操作。另从官方 npm 包 `@bytedance/doubao-cli` 2.31.3 获取 macOS 原生载荷，按包内 manifest 校验 SHA-256，实际执行 `--help` 成功：支持登录、会话、模型和权限选项，没有 MCP 连接器 CRUD 命令。CLI 有独立的运行目录，不能据此当作桌面的配置文件入口。

当前保留[官方自定义连接器流程](https://www.doubao.com/work/docs/zh-cn/articles/705018132598-plugins)。本次已在真实客户端保存 HTTP 和 mcplink STDIO Bridge，完成模型调用，并在原生菜单退出、重新启动后再次分别调用成功。支持导入不等于拥有独立原生管理接口。

## 验证范围

- 已执行：官方包下载、macOS 签名验证与安装启动；用户自行登录；Windows 分发元数据检查；本地目录、相关 SQLite 表结构及模式检查；本地 MCP 工具发现；两款 CLI 的实际帮助命令。
- 已执行：千问的官方 JSON 保存、外部文件修改、正常重启、原生管理状态查询和真实模型工具调用；mcplink 原生应用的共享 Bridge、HTTP 直连在重启后均通过客户端运行时调用。
- 已执行：豆包的 HTTP 和共享 STDIO Bridge 保存、实际模型调用、原生菜单退出、重启后两项工具再次调用。
- 已执行：用 mcplink 恢复事务撤销千问的两次原生写入，再移除本次界面导入的合成条目；重启后在连接器页核对。豆包通过原客户端移除两项合成连接器，保留原有推荐项；本次专用后台、连接和测试服务均已清理。
- 隔离验收另覆盖安装探测、离线导出、原生入口、身份存储保护、旧后台拒绝、冲突及恢复；构建后的界面覆盖原生与导入切换。这些使用合成服务，不能代替其他版本或真实业务账号的验收。
- 未执行：Windows 厂商桌面登录与加载、其他客户端版本、千问身份隔离模式的原生写入。跨平台构建和隔离验收以对应提交的 Actions 为准。

安装包、原厂代码、校验记录、截图和本机验收报告保存在忽略的 `artifacts/e2e/` 中，不进入公开源码或分发包。公开文档只保留版本、路径、接口边界和可复现检查方法，不保留账号内容。
