# 端到端验证

从实际 CLI、后台和 Bridge 入口验证配置及调用行为。默认测试使用合成用户目录、MCP/OAuth 服务及 fixture 凭据库，不要求真实账号或模型登录。

## 环境与运行

先安装项目依赖，构建前端和 CLI：

```sh
npm ci
npm run build
npm run check
node scripts/prepare-sidecar.mjs --debug --test-fixtures
python3 tests/e2e/run.py --record artifacts/e2e/core-local
```

使用合成凭据库的脚本需要显式构建 `test-fixtures`。`MCPLINK_E2E_BINARY` 可指定待测构建或安装目录中的程序，默认使用 debug CLI。`npm run bundle -- --test-fixtures` 将合成验收安装包构建到 `target/fixture-bundle`。发布包由默认构建生成，单独验证系统凭据库边界。每次运行使用新的记录目录，报告与临时内容保存在忽略的 `artifacts/e2e/`。各脚本参数以 `--help` 为准。

## 场景

| 入口 | 覆盖范围 |
| --- | --- |
| `frontend.py` | 真实前端生产构建、模块/样式/字体与公开资源、仅服务生成目录、开发刷新、端口占用及退出清理 |
| `runtime_session.py` | 桌面与 Bridge 共用的内存 Session，并发认证、拒绝后明确重试、实例更换重新认证和中断后无业务重放。先运行 `cargo build --locked -p mcplink-core --features test-fixtures --example runtime_session`。`openapi.py` 的迁移确认场景也使用此测试程序。 |
| `macos_keychain.py` | 独立临时钥匙串与合成身份；两个不同构建的调用者复用同一固定凭据助手，拒绝错误调用者和散列，普通操作不显示系统授权 UI；不接触日常凭据 |
| `run.py`、`agent_cli.py` | 服务盘点、连接与授权、命令契约、共享及恢复 |
| `config_integrity.py`、`bridge_adoption.py` | 目录链接、历史来源 ID、跨数据目录写锁、预览和备份期限、写入中断及补记、已有 Bridge 接管。用 `--previous-binary` 验证旧版预览的迁移。 |
| `agent_discovery.py`、`office_agents.py`、`pi_agent.py` | 安装门禁、来源与存储保护、原生配置及客户端确认、共享调用 |
| `claude_sources.py`、`claude_writable.py`、`opencode_sources.py` | 客户端作用域、只读边界、格式及可写事务 |
| `native_clients.py` | Codex app-server 与 Hermes 官方 MCP 发现/工具注册表：一个原生入口调用两个独立上游、范围变化、授权故障隔离、恢复及条目还原；不启动模型推理 |
| `claude_native.py` | Claude 原生 CLI 加载生成的原生配置，并调用两个独立上游；模型 Messages 响应来自 loopback 合成服务，不使用真实模型账号 |
| `tool_calls.py`、`generated_cli.py`、`cli_workflow.py` | 工具发现、Schema 校验、错误与退出码、命令生成及重新绑定；契约参数差异、单服务/聚合发现故障归因、旧后台拒绝及零调用核对 |
| `http_api.py` | HTTP API 的路径/query/JSON 映射、图片与 Base64、聚合调用、连接复用、凭据回显过滤及超时后单次派发 |
| `openapi.py` | OpenAPI／Swagger 离线导入、本机和内网候选显式选择、文档级展开容量、并发预览、响应条件及受保护字段迁移 |
| `aggregate_gateway.py` | 单入口多上游、重名路由、分页、资源与提示、服务选择、配置迁移和三路径共用 OAuth；可用 `--pi-package` 验证官方 Pi MCP 运行时 |
| `entry_policy.py` | 同一账号的独立入口白名单、空与继承语义、全局上限和拒绝伪造调用 |
| `catalog_refresh.py` | 工具/资源/提示变更通知、入口隔离、手动刷新、停止后台后的巡检及官方 Pi 实时工具刷新（`--pi-package`） |
| `catalog_pagination.py` | 动态上游的聚合分页快照、跨入口游标隔离、成员变更、手动刷新和对应种类通知后的游标失效 |
| `remote_transports.py`、`oauth_clients.py` | 远程传输、出站地址与代理边界、会话换代、OAuth 注册与刷新、取消和固定端口复用 |
| `desktop_ui.py` | 构建后界面流程与可访问性，包括缺失入口成员、晚到的表单回复、非法端口、异步关闭及重复导入。需要 Chrome 和 agent-browser；可用 `--forms-only` 仅跑恢复表单和 OpenAPI。 |
| `windows_desktop.py` | NSIS 隔离安装后的真实 Tauri/WebView2 窗口：Windows UI Automation 导航、弹窗取消、HTTP 探测、暂停/恢复、关闭到托盘及单实例重开；交互桌面不可用时失败 |
| `windows_background.py`、`daemon_lifecycle.py` | 暂停持久化、退出、残留端口冲突、并发重连与进程树清理。Linux 源码 CLI 可通过后者检查 SIGTERM 和进程组。 |
| `windows_signing.py`、`windows_installer.py` | 签名拒绝边界，NSIS 隔离安装、旧版升级、重装及卸载保留数据。`--production` 检查从发布包安装得到的 CLI，签名候选还检查安装后的签名。 |
| `updater.py` | 真实 Rust 更新实现访问回环 GitHub 夹具：版本筛选、取消、哈希/签名/签名版本拒绝、受限 URL、CLI 同步与中断恢复；`--live` 另检查公开 GitHub。此脚本不安装桌面，Windows 下载契约不等于 Windows 应用内升级通过。 |
| `verify_package.py`、`installed_app.py` | macOS DMG、打包内容和签名。隔离升级场景核对已有配置、认证、Bridge 和生成的 CLI。 |
| `release_vault.py` | 用实际发布 CLI 验证显式的 fixture 请求被拒绝，且后台与测试凭据库均未创建 |

Windows 覆盖验收可在一次性 x64 测试账户中传入两个不同的默认模式 NSIS，并启用 `windows_installer.py --native-migration`。将 `MCPLINK_E2E_BINARY` 指向当前默认模式 CLI。验收在 Credential Manager 中存放合成授权，通过真实安装器检查连接、授权、已有生成脚本和旧条目历史。原生配置的 DACL 和命名数据流须在应用及恢复后保持，数据库和备份须为私有。测试清理本次创建的合成凭据，原始报告保存在验收目录。

`macos_keychain.py` 不修改证书信任。本地默认模式保留既有默认钥匙串；GitHub 托管 macOS 运行器显式传 `--github-hosted-keychain-default`，仅在测试期间将临时钥匙串选为默认，结束时恢复原值并移除临时钥匙串；其他环境拒绝该选项。正式本机签名、固定助手准备和部署不调用这条 CI 流程。

## 原生客户端与平台

`pi_agent.py --pi-package /absolute/package` 可使用官方 Pi 包运行 CLI 和 MCP 运行时检查。其他可选原生客户端参数见各脚本；固定版本与完整跨平台调用方式见 [CI 工作流](../../.github/workflows/ci.yml)。

`native_clients.py --codex /absolute/codex --hermes-source /absolute/vendor-checkout --hermes-python /absolute/venv/python --record artifacts/e2e/native-local` 使用明确指定的真实运行时，不执行来源盘点用的占位启动器。不要解析虚拟环境 Python 的符号链接到系统 Python。Hermes 使用固定官方源码和可选 MCP 依赖闭包；不启动其模型或网关。工具注册表中由客户端增加的资源/提示辅助工具不算作聚合上游工具。

CI 固定 Codex 0.167.0（Apache-2.0）、Claude Code 2.1.319（Anthropic 商业许可）、Hermes 官方提交 `3b421578588243f920886e6fe79a6faa87ea026e`（MIT）与 Pi 1.5.1（MIT）。这些运行时只用于测试，不进入 mcplink 安装包。Hermes 的测试依赖位于 [hermes-mcp-requirements.txt](../fixtures/hermes-mcp-requirements.txt)。

Hermes 和 Pi 验收实时清单刷新。Codex 的现有 thread 会缓存工具清单；测试使用官方 `config/mcpServer/reload` 后新建临时 thread，不将其算作现有会话实时刷新。Claude 测试通过 `--bare --strict-mcp-config --mcp-config` 明确加载 mcplink 写出的原生文件，并只批准两个合成读取工具；其 loopback provider 按 [Messages 工具协议](https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls)驱动真实客户端调用，不能证明模型推理或业务账号权限。

Chrome renderer 与原生桌面是两层独立检查。agent-browser 0.41.2 需要 Node.js 24+；CI 仅为浏览器驱动选择 Node 24，随后恢复项目固定的 Node 22。`--browser-driver` 可指定对应平台的原生驱动文件；Windows 使用实际 `.exe`，避免 npm Shell 启动器差异。等待目标输入控件加载并检查选中状态，不重试可能产生业务副作用的调用。

Windows UI Automation 通过已验证归属的 Tauri 主窗口枚举 WebView2 原生内容 HWND，再访问其实际可访问性 provider；不使用浏览器 renderer 代替安装应用。wry 的父 HWND 与 WebView2 UIA 树衔接存在[上游已报告的限制](https://github.com/tauri-apps/tauri/issues/15624)，因此这项验收不证明所有屏幕阅读器都能从父窗口自动遍历整个应用。测试不关闭 UIA 或改变系统辅助功能设置。

原生 MCP 连接成功不代表模型业务回合通过。测试真实客户端、系统凭据库或业务账号时，使用独立配置与合成数据，明确实际范围并清理测试资源。不要把账号凭据、业务正文、原生数据库或原始登录日志加入仓库。

CI 为 macOS 和 Windows 分别构建并运行适用场景，只上传 `packages-*` 中明确选择的安装包、CLI 与公开构建清单。原始报告、截图、浏览器资料、凭据库、配置和日志保存在忽略的本地工件目录；失败日志仅输出固定场景和阶段摘要。平台专属测试只证明对应平台；本地检查不能替代另一平台的结果。

同一分支有更新时取消被替代的验收作业，避免重复占用运行资源。源码安全审计使用独立的 macOS Intel 运行池，应用仍在 macOS Apple Silicon 和 Windows x64 上构建验收；审计运行池不代表新增 Intel Mac 安装包支持。

tag 推送不重复触发分支 CI；发布门禁仍是该 tag 对应提交的完整双平台成功记录。界面测试以持久状态确认结果；会自动消失的 toast 不作为服务错误仍被保留的唯一证据，业务请求仍按计数核对且不自动重试。
