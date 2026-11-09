# Agent 接入管理与共享代理

当前支持 **Codex、Hermes、Claude Code、OpenCode、ZCode、DeepSeek Harness、OpenClaw、Kimi Code、WorkBuddy、千问办公、豆包工作**。前九类维护原生文件，千问办公和豆包工作通过客户端导入。Trae、Trae CN、VS Code 仍不在支持名单；不会卸载这些软件或删除其原生配置。

客户端始终显示为支持对象。未发现本机程序或命令入口时显示“未安装”，原生文件写入不可用；安装后刷新。已有配置目录仅表示残留来源，不算已安装。探测只读取预设程序位置、常见 CLI 目录、PATH 和已知配置位置，不启动客户端，也不验证版本或登录状态。`detected` 包含配置残留，`installed` 表示找到程序入口。原生文件客户端的 `configurable` 随安装状态切换；`management:manual` 的千问办公和豆包工作可离线准备导入内容，因此 `configurable:true`，没有可写 `profilePath` 或原生来源，也不会产生已接入状态。

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
| WorkBuddy | 用户 `~/.workbuddy/mcp.json`；显式项目 `<项目>/.workbuddy/mcp.json` | `mcpServers`，`type:stdio/streamableHttp`；启停与信任在客户端维护 |

DeepSeek 将 desktop 和 web 作为独立可写目标，同时列出已初始化的 headless/acp profile。新接入通过 `insert` 添加原生 MCP client 插件；已有 manager 服务按原来的 manager 映射逐项修改。原生包基础 `cordis.yml` 只读，变更写入对应 patch，不替换整个运行组合。

WorkBuddy 的 `timeout` 是毫秒连接超时，映射到托管 `startupTimeout` 秒；非整秒无法等价转换时拒绝导入。`disabledTools` 对应精确工具黑名单。请求超时、工具白名单、原生运行时声明或动态凭据模板不能等价写入时明确失败；可显式建立共享连接。当前 MCP 上游仅支持 STDIO / Streamable HTTP，保留但不托管旧 SSE 条目。不会写入未经定义的 `enabled` 字段。

## 客户端导入

在服务页点击「接给 Agent」，或在 Agent 页选择客户端并点击服务旁的「生成配置」。选择直连或共享，再生成导入内容。客户端未安装时也可操作。两个入口均不创建绑定、操作历史或客户端配置文件；导入后需在原客户端检查加载并完成真实工具调用。

| 客户端 | 生成内容 | 客户端操作 |
| --- | --- | --- |
| 千问办公 `qwenwork` | `mcpServers` JSON；HTTP 使用 `type:streamable-http` | 扩展 → 连接器 → +添加 → 粘贴 JSON，保存并启用，再新建任务 |
| 豆包工作 `doubaowork` | 服务器名称、HTTP URL/Headers 或 STDIO command/逐项 args/env | 插件·技能·伙伴 → 插件 → +添加 → 新建自定义连接器，按字段填写并保存 |

千问办公支持批量 JSON；豆包工作按服务逐项填写，参数须保留顺序、空格、空参数及引号。两者的本地存储不是公开配置契约，不猜测数据库或修改私有存储。没有可等价表达的工作目录、超时、工具/资源限制、OAuth 参数或变量引用时拒绝直连导出，提示使用共享模式。

共享模式生成本机 mcplink STDIO Bridge，只包含本机路径、数据目录和托管连接引用；先检查连接并完成账号认证。暂停期间仍可准备配置，但 UI 会提示恢复代理才能调用。移动程序后需重新生成入口。直连内容含完整令牌、地址和启动参数，UI 明确提示，CLI 必须显式选择 `--mode direct`；不把该内容当作脱敏状态输出。

## 管理与代理

已安装客户端的可写目标复用同一套预览、文件哈希冲突检查、备份、应用和逐条恢复事务。来源登记、预览、应用和恢复都会在后台检查安装状态，预览后卸载也会阻止写入。预览不创建文件；应用后才创建缺失的用户配置。JSON/JSONC/JSON5 注释和未涉及设置保留，DeepSeek 的无关 YAML 及动态表达式保持原样，不执行 `!!js`。

- **直连**：按客户端格式写入上游 command / args / env 或 HTTP URL / headers。
- **共享代理**：写入原生 stdio 入口，调用当前 mcplink CLI 的 `bridge --connection ID`，通过同一托管连接复用凭据、工具过滤和上游会话。原生文件不保存共享上游凭据。
- **启停和移除**：使用各客户端原生状态字段；Claude Code 的项目启停继续在其 `/mcp` 中维护，WorkBuddy 启停与信任也由原客户端维护，mcplink 可以接入、替换和移除其配置。
- **导入**：只转换能够等价表示的定义。未知执行选项、动态凭据模板、远程执行环境、不等价的毫秒超时会明确拒绝；可另建托管连接并使用共享代理。新适配器的原生 OAuth 导出也使用共享代理，认证由 mcplink 维护。

只读按**具体来源**决定：插件和系统托管来源、DeepSeek 基础组合、ZCode `.agents/mcp.json` 共享回退文件不会被写入。ZCode 存在非空共享回退时，写入原生空服务表会遮蔽回退服务，因此要求先在原客户端合并，避免丢失有效配置。这些限制不将整类客户端降为只读。

## 版本与验收范围

当前客户端名单、WorkBuddy 操作与客户端导入需要 **management API 15** 及同一构建的桌面和后台。`sources add --agent` 接受九类原生文件客户端标识，仅允许已安装客户端，来源 ID 从 `status` 读取；ZCode/Kimi Code/WorkBuddy 的项目来源须显式指定项目目录。DeepSeek 可通过完整 profile 路径单独登记。千问办公/豆包工作使用 `connections export-config`，不登记原生来源。

隔离回归 `tests/e2e/agent_discovery.py` 使用只读安装入口 fixture 模拟安装和卸载，使用独立 TOML、JSON5、YAML 解析器读取生成的原生配置，然后实际启动其中的 command/args/env，通过 MCP SDK 调用合成上游工具。覆盖未安装禁用、安装刷新、卸载后预览/应用/恢复保护、九类直连与共享入口、两个 DeepSeek profile、HTTP 导入、启停、移除恢复、注释和动态 YAML 保留、并发冲突及退休来源。`tests/e2e/office_agents.py` 验证办公客户端的具体字段、导入无副作用、参数边界和真实 Bridge 调用；`tests/e2e/desktop_ui.py` 验证界面生成、复制及待导入提示。它们不代表已安装所有第三方客户端，亦不代表原生模型回合验收。

办公客户端格式依据（2026-11-09）：[WorkBuddy 配置路径](https://www.codebuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/MCP-Guide)、[WorkBuddy 字段规范](https://open.workbuddy.cn/docs/connector)、[千问办公连接器](https://docs.qwenwork.cn/features/connectors)、[豆包工作插件](https://www.doubao.com/work/docs/zh-cn/articles/705018132598-plugins)。本机未安装这三款客户端，原生加载与模型回合验收待具备环境后执行。

格式依据：[ZCode MCP](https://zcode.z.ai/en/docs/mcp-services)、[OpenClaw MCP](https://docs.openclaw.ai/tools/mcp)、[Kimi Code MCP](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/mcp.html)、[DeepSeek MCP client](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/mcp/mcp-client)、[DeepSeek MCP manager](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/mcp/mcp-manager)。配置适配和 profile 写入设计参考 [CC Switch](https://github.com/farion1231/cc-switch/tree/cac921f5d7f662085800cd99684f367bd0d90e74/src-tauri/src)，保留 mcplink 自身的配置事务和 MCP 共享网关。

桌面会检查 Unix 用户的 nvm 版本目录。支持 CLI 的客户端安装位置未被识别时，可在 Agent 页点击“选择已安装的 CLI”；WorkBuddy 使用“选择已安装的程序”，macOS 选择 `WorkBuddy.app`，Windows 选择 `WorkBuddy.exe`。后台核验文件名、类型、应用包中的实际程序或 Unix 执行权限；预览和写入时均重新核验。办公客户端按预设桌面安装位置探测；千问办公/豆包工作生成配置不依赖安装探测。
