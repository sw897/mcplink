# 端到端验证

从实际 CLI、后台和 Bridge 入口验证配置及调用行为。默认测试使用合成用户目录、MCP/OAuth 服务及 fixture 凭据库，不要求真实账号或模型登录。

## 环境与运行

先安装项目依赖，构建前端和 CLI：

```sh
npm ci
npm run build
npm run check
python3 tests/e2e/run.py --record artifacts/e2e/core-local
```

`MCPLINK_E2E_BINARY` 可指定待测构建或安装目录中的程序；未指定时使用 debug CLI。每次运行使用新的记录目录，报告与临时内容保存在忽略的 `artifacts/e2e/`。各脚本参数以 `--help` 为准。

## 场景

| 入口 | 覆盖范围 |
| --- | --- |
| `run.py`、`agent_cli.py` | 服务盘点、连接与授权、命令契约、共享及恢复 |
| `config_integrity.py`、`bridge_adoption.py` | 格式保留、并发与失败恢复、已有 Bridge 接管和保留原生配置退出托管 |
| `agent_discovery.py`、`office_agents.py`、`pi_agent.py` | 安装门禁、来源与存储保护、原生配置及客户端确认、共享调用 |
| `claude_sources.py`、`claude_writable.py`、`opencode_sources.py` | 客户端作用域、只读边界、格式及可写事务 |
| `native_clients.py` | Codex app-server 与 Hermes 官方 MCP 发现/工具注册表：一个原生入口调用两个独立上游、范围变化、授权故障隔离、恢复及条目还原；不启动模型推理 |
| `claude_native.py` | Claude 原生 CLI 加载生成的原生配置，并调用两个独立上游；模型 Messages 响应来自 loopback 合成服务，不使用真实模型账号 |
| `tool_calls.py`、`generated_cli.py`、`cli_workflow.py` | 工具发现、Schema 校验、错误与退出码、命令生成及重新绑定；契约参数差异、单服务/聚合发现故障归因、旧后台拒绝及零调用核对 |
| `http_api.py` | HTTP API 的路径/query/JSON 映射、图片与 Base64 转换、发现与核验区分、聚合路由、CLI 与凭据生命周期 |
| `aggregate_gateway.py` | 单入口多上游、重名路由、分页、资源与提示、服务选择、配置迁移和三路径共用 OAuth；可用 `--pi-package` 验证官方 Pi MCP 运行时 |
| `entry_policy.py` | 同一账号的独立入口白名单、空与继承语义、全局上限和拒绝伪造调用 |
| `catalog_refresh.py` | 工具/资源/提示变更通知、入口隔离、手动刷新、断开清理及官方 Pi 实时工具刷新（`--pi-package`） |
| `catalog_pagination.py` | 动态上游的聚合分页快照、跨入口游标隔离、认证与成员变更后的游标失效 |
| `remote_transports.py`、`oauth_clients.py` | 远程传输、协议会话、OAuth 注册与刷新、并发和失效处理 |
| `desktop_ui.py` | 构建后界面流程与可访问性，包括搜索结果批量选择/取消、契约变更及未能核验提示，HTTP API 只读发现、探测确认、修订冲突、重复提交和异步关闭；需要 Chrome 和 agent-browser |
| `windows_desktop.py` | NSIS 隔离安装后的真实 Tauri/WebView2 窗口：Windows UI Automation 导航、弹窗取消、HTTP 探测、暂停/恢复、关闭到托盘及单实例重开；交互桌面不可用时失败 |
| `windows_background.py`、`daemon_lifecycle.py` | 暂停持久化、退出、重连与进程树清理 |
| `windows_signing.py`、`windows_installer.py` | 签名拒绝边界，NSIS 隔离安装、旧版升级、重装及卸载保留数据；有旧包时明确记录基线 |
| `verify_package.py`、`installed_app.py` | macOS DMG、打包内容、签名和隔离安装场景 |

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
