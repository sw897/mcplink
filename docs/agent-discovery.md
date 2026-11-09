# 八类 Agent 接入管理与共享代理

当前源码的支持名单固定为 **Codex、Hermes、Claude Code、OpenCode、ZCode、DeepSeek Harness、OpenClaw、Kimi Code**。Trae、Trae CN、WorkBuddy、VS Code 已从支持名单移除；不会卸载这些软件或删除其原生配置。

八类客户端始终显示为支持对象。未发现本机程序或命令入口时显示“未安装”，配置、接入及目标选择不可用；安装后刷新即可启用。已有配置目录仅表示残留来源，不算已安装。探测只读取预设程序位置、常见 CLI 目录、PATH 和已知配置位置，不启动客户端，也不验证版本或登录状态。`detected` 包含配置残留，`installed` 表示找到程序入口，`configurable` 随安装状态切换。

## 原生格式

| 客户端 | 默认配置位置 | 服务表与启用状态 |
| --- | --- | --- |
| Codex | `CODEX_HOME/config.toml`，默认 `~/.codex/config.toml` | TOML `mcp_servers`，`enabled` |
| Hermes | `HERMES_HOME/config.yaml`，默认 `~/.hermes/config.yaml` | YAML `mcp_servers`，`enabled` |
| Claude Code | 用户 `.claude.json`；显式 local/project 来源 | JSON `mcpServers`；原生项目启停偏好由客户端维护，不能向条目伪造 `enabled` |
| OpenCode | 用户 `opencode.json` / `opencode.jsonc`；显式项目来源 | 只维护当前 `mcp.servers`、`disabled` 格式 |
| ZCode | `~/.zcode/cli/config.json`；项目 `.zcode/config.json` | `mcp.servers`，`enable` |
| DeepSeek Harness | `$DSH_HOME/profiles/<profile>/cordis.patch.yml`，默认 `~/.dsh` | Cordis MCP client 插件行，或 MCP manager 的 `config.servers`；插件 `disabled` / 服务 `enabled` |
| OpenClaw | `$OPENCLAW_CONFIG_PATH`，默认 `$OPENCLAW_STATE_DIR/openclaw.json` / `~/.openclaw/openclaw.json` | JSON5 `mcp.servers`，`enabled` |
| Kimi Code | `$KIMI_CODE_HOME/mcp.json`，默认 `~/.kimi-code/mcp.json`；项目 `.kimi-code/mcp.json` | JSON `mcpServers`，`enabled`；不读取旧 kimi-cli 的 `~/.kimi` |

DeepSeek 将 desktop 和 web 作为独立可写目标，同时列出已初始化的 headless/acp profile。新接入通过 `insert` 添加原生 MCP client 插件；已有 manager 服务按原来的 manager 映射逐项修改。原生包基础 `cordis.yml` 只读，变更写入对应 patch，不替换整个运行组合。

## 管理与代理

已安装客户端的可写目标复用同一套预览、文件哈希冲突检查、备份、应用和逐条恢复事务。来源登记、预览、应用和恢复都会在后台检查安装状态，预览后卸载也会阻止写入。预览不创建文件；应用后才创建缺失的用户配置。JSON/JSONC/JSON5 注释和未涉及设置保留，DeepSeek 的无关 YAML 及动态表达式保持原样，不执行 `!!js`。

- **直连**：按客户端格式写入上游 command / args / env 或 HTTP URL / headers。
- **共享代理**：写入原生 stdio 入口，调用当前 mcplink CLI 的 `bridge --connection ID`，通过同一托管连接复用凭据、工具过滤和上游会话。原生文件不保存共享上游凭据。
- **启停和移除**：使用各客户端原生状态字段；Claude Code 的项目启停继续在其 `/mcp` 中维护，mcplink 可以接入、替换和移除配置。
- **导入**：只转换能够等价表示的定义。未知执行选项、动态凭据模板、远程执行环境、不等价的毫秒超时会明确拒绝；可另建托管连接并使用共享代理。新适配器的原生 OAuth 导出也使用共享代理，认证由 mcplink 维护。

只读按**具体来源**决定：插件和系统托管来源、DeepSeek 基础组合、ZCode `.agents/mcp.json` 共享回退文件不会被写入。ZCode 存在非空共享回退时，写入原生空服务表会遮蔽回退服务，因此要求先在原客户端合并，避免丢失有效配置。这些限制不将整类客户端降为只读。

## 版本与验收范围

八类配置操作和安装状态校验需要 **management API 9** 及同一构建的桌面和后台。`sources add --agent` 接受八类标识，仅允许已安装客户端，来源 ID 从 `status` 读取；ZCode/Kimi Code 的项目来源须显式指定项目目录。DeepSeek 可通过完整 profile 路径单独登记。

隔离回归 `tests/e2e/agent_discovery.py` 使用只读安装入口 fixture 模拟安装和卸载，使用独立 TOML、JSON5、YAML 解析器读取生成的原生配置，然后实际启动其中的 command/args/env，通过 MCP SDK 调用合成上游工具。覆盖未安装禁用、安装刷新、卸载后预览/应用/恢复保护、八类直连与共享入口、两个 DeepSeek profile、HTTP 导入、启停、移除恢复、注释和动态 YAML 保留、并发冲突及退休来源。它不代表已安装所有第三方客户端，亦不代表原生模型回合验收。

格式依据：[ZCode MCP](https://zcode.z.ai/en/docs/mcp-services)、[OpenClaw MCP](https://docs.openclaw.ai/tools/mcp)、[Kimi Code MCP](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/mcp.html)、[DeepSeek MCP client](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/mcp/mcp-client)、[DeepSeek MCP manager](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/mcp/mcp-manager)。配置适配和 profile 写入设计参考 [CC Switch](https://github.com/farion1231/cc-switch/tree/cac921f5d7f662085800cd99684f367bd0d90e74/src-tauri/src)，保留 mcplink 自身的配置事务和 MCP 共享网关。

桌面会检查 Unix 用户的 nvm 版本目录。若安装位置未被识别，在 Agent 页点击“选择已安装的 CLI”，选择该客户端的启动文件。后台仅核验文件名、类型以及 Unix 执行权限；预览和写入时均重新核验。
