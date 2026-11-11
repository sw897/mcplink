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
| `tool_calls.py`、`generated_cli.py`、`cli_workflow.py` | 工具发现、Schema 校验、错误与退出码、命令生成及重新绑定 |
| `http_api.py` | HTTP API 的路径/query/JSON 映射、图片与 Base64 转换、发现与核验区分、聚合路由、CLI 与凭据生命周期 |
| `aggregate_gateway.py` | 单入口多上游、重名路由、分页、资源与提示、服务选择、配置迁移和三路径共用 OAuth；可用 `--pi-package` 验证官方 Pi MCP 运行时 |
| `entry_policy.py` | 同一账号的独立入口白名单、空与继承语义、全局上限和拒绝伪造调用 |
| `catalog_refresh.py` | 工具/资源/提示变更通知、入口隔离、手动刷新、断开清理及官方 Pi 实时工具刷新（`--pi-package`） |
| `catalog_pagination.py` | 动态上游的聚合分页快照、跨入口游标隔离、认证与成员变更后的游标失效 |
| `remote_transports.py`、`oauth_clients.py` | 远程传输、协议会话、OAuth 注册与刷新、并发和失效处理 |
| `desktop_ui.py` | 构建后界面流程与可访问性；需要 Chrome 和 agent-browser |
| `windows_background.py`、`daemon_lifecycle.py` | 暂停持久化、退出、重连与进程树清理 |
| `windows_signing.py`、`windows_installer.py` | 签名拒绝边界，NSIS 隔离安装、旧版升级、重装及卸载保留数据；有旧包时明确记录基线 |
| `verify_package.py`、`installed_app.py` | macOS DMG、打包内容、签名和隔离安装场景 |

## 原生客户端与平台

`pi_agent.py --pi-package /absolute/package` 可使用官方 Pi 包运行 CLI 和 MCP 运行时检查。其他可选原生客户端参数见各脚本；固定版本与完整跨平台调用方式见 [CI 工作流](../../.github/workflows/ci.yml)。

原生 MCP 连接成功不代表模型业务回合通过。测试真实客户端、系统凭据库或业务账号时，使用独立配置与合成数据，明确实际范围并清理测试资源。不要把账号凭据、业务正文、原生数据库或原始登录日志加入仓库。

CI 为 macOS 和 Windows 分别构建并运行适用场景，上传验收摘要和安装包。平台专属测试只证明对应平台；本地检查不能替代另一平台的结果。
