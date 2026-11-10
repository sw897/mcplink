# Agent 接入管理与共享代理

当前支持 **Codex、Hermes、Claude Code、OpenCode、ZCode、DeepSeek Harness、OpenClaw、Kimi Code、WorkBuddy、千问办公、Pi Agent、豆包工作**。前十一类维护原生文件；千问办公的身份隔离或迁移存储回退到客户端导入，豆包工作通过客户端导入。Trae、Trae CN、VS Code 仍不在支持名单；不会卸载这些软件或删除其原生配置。

客户端始终显示为支持对象。未发现本机程序或命令入口时显示“未安装”，原生文件写入不可用；安装后刷新。已有配置目录仅表示残留来源，不算已安装。探测只读取预设程序位置、常见 CLI 目录、PATH 和已知配置位置，不启动客户端，也不验证版本或登录状态。`detected` 包含配置残留，`installed` 表示找到程序入口。原生文件客户端的 `configurable` 随安装状态切换；`supportsImport:true` 的千问办公和豆包工作可以离线准备导入。`management:manual` 不提供可写 `profilePath`；千问已有的旧 JSON 来源只读保留并说明原因。导出不产生绑定或已接入状态。

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
| 千问办公 | 用户 `~/.qwenworkcn/mcp.json`，仅共享 JSON 模式 | 严格 JSON 的 `mcpServers`，`type:stdio/streamable-http`，`enabled`；身份隔离或迁移存储回退到导入 |
| Pi Agent | `$PI_CODING_AGENT_DIR/mcp.json`，默认 `~/.pi/agent/mcp.json`；显式项目 `.pi/mcp.json` | 内置 MCP 严格 JSON，`mcpServers`，`type:stdio/http`，`enabled`；不写项目信任登记 |

DeepSeek 将 desktop 和 web 作为独立可写目标，同时列出已初始化的 headless/acp profile。新接入通过 `insert` 添加原生 MCP client 插件；已有 manager 服务按原来的 manager 映射逐项修改。原生包基础 `cordis.yml` 只读，变更写入对应 patch，不替换整个运行组合。

WorkBuddy 5.9.3 的用户文件使用严格 JSON。`timeout` 按客户端规则解释：小于 1000 的正整数按秒，其余按毫秒；能等价表示时同时映射到规范启动与请求超时。导出要求两种超时一致，并写无歧义的毫秒值。工具启停由独立原生策略维护，用户文件中的 `disabledTools` 不作为已生效限制；工具过滤使用共享代理。原生运行时声明或动态凭据模板不能等价写入时明确失败。当前 MCP 上游仅支持 STDIO / Streamable HTTP，保留但不托管旧 SSE 条目。不会写入未经定义的 `enabled` 字段。[实机验收与字段依据](workbuddy-verification.md)。

[Pi Agent](pi-agent.md)仅针对当前内置 MCP。保留根偏好、描述和工具暴露设置，不能等价托管的动态凭据、身份认证、进度超时及工具可见性限制明确拒绝。偏好覆盖不作为完整上游导入；项目来源需用户在 Pi 中信任，应用后 `/reload`。第三方替代扩展和旧版 CLI 不据此视为已接入。

## 客户端导入

在服务页点击「接给 Agent」，或在 Agent 页选择客户端并点击服务旁的「生成配置」。选择直连或共享，再生成导入内容。客户端未安装时也可操作。两个入口均不创建绑定、操作历史或客户端配置文件；导入后需在原客户端检查加载并完成真实工具调用。

| 客户端 | 生成内容 | 客户端操作 |
| --- | --- | --- |
| 千问办公 `qwenwork` | `mcpServers` JSON；HTTP 使用 `type:streamable-http` | 技能·扩展 → 连接器 → +添加 → 粘贴 JSON，保存并启用，再新建任务 |
| 豆包工作 `doubaowork` | 服务器名称、HTTP URL/Headers 或 STDIO command/逐项 args/env | 插件·技能·工作台 → 插件 → +添加 → 新建自定义连接器，按字段填写并保存 |

千问办公原生来源为当前用户 `~/.qwenworkcn/mcp.json`，使用严格 JSON 的 `mcpServers`，支持 STDIO、Streamable HTTP 和 `enabled`；保留其他偏好及 `cliConnectors`，拒绝跨桶同名和未验证的执行字段。检测到身份格式标记、迁移目录、身份数据库或非空身份连接器表时，禁止写旧 JSON，并切换到导入。写入后正常重启千问办公。

千问导入支持批量 JSON；豆包工作按服务逐项填写，参数须保留顺序、空格、空参数及引号。[实机调查](office-native-investigation.md)记录了存储、接口和原生加载证据。没有可等价表达的工作目录、超时、工具/资源限制、OAuth 参数或变量引用时拒绝直连导出，提示使用共享模式。

共享模式生成本机 mcplink STDIO Bridge，只包含本机路径、数据目录和托管连接引用；先检查连接并完成账号认证。暂停期间仍可准备配置，但 UI 会提示恢复代理才能调用。移动程序后需重新生成入口。直连内容含完整令牌、地址和启动参数，UI 明确提示，CLI 必须显式选择 `--mode direct`；不把该内容当作脱敏状态输出。

## 管理与代理

已安装客户端的可写目标复用同一套预览、文件哈希冲突检查、备份、应用和逐条恢复事务。来源登记、预览、应用和恢复都会在后台检查安装状态，预览后卸载也会阻止写入。预览不创建文件；应用后才创建缺失的用户配置。JSON/JSONC/JSON5 注释和未涉及设置保留，DeepSeek 的无关 YAML 及动态表达式保持原样，不执行 `!!js`。

- **直连**：按客户端格式写入上游 command / args / env 或 HTTP URL / headers。
- **共享代理**：写入原生 stdio 入口，调用当前 mcplink CLI 的 `bridge --connection ID`，通过同一托管连接复用凭据、工具过滤和上游会话。原生文件不保存共享上游凭据。
- **启停和移除**：使用各客户端原生状态字段；Claude Code 的项目启停继续在其 `/mcp` 中维护，WorkBuddy 启停与信任也由原客户端维护，mcplink 可以接入、替换和移除其配置。
- **导入**：只转换能够等价表示的定义。未知执行选项、动态凭据模板、远程执行环境、不等价的毫秒超时会明确拒绝；可另建托管连接并使用共享代理。新适配器的原生 OAuth 导出也使用共享代理，认证由 mcplink 维护。

只读按**具体来源**决定：插件和系统托管来源、DeepSeek 基础组合、ZCode `.agents/mcp.json` 共享回退文件不会被写入。ZCode 存在非空共享回退时，写入原生空服务表会遮蔽回退服务，因此要求先在原客户端合并，避免丢失有效配置。这些限制不将整类客户端降为只读。

## 版本与验收范围

当前需要 **management API 17** 及同一构建的桌面和后台。`sources add --agent` 接受十一类原生文件客户端标识，仅允许已安装客户端，来源 ID 从 `status` 读取；ZCode/Kimi Code/WorkBuddy/Pi 的项目来源须显式指定项目目录。DeepSeek 可通过完整 profile 路径单独登记。千问仅适配当前用户的标准 JSON 路径；千问/豆包的 `connections export-config` 继续可用，导出不登记原生来源。

隔离回归 `tests/e2e/agent_discovery.py` 使用安装入口 fixture 模拟安装和卸载，使用独立格式解析器读取生成的原生配置，再启动 command/args/env，通过 MCP SDK 调用合成上游工具。覆盖十一类直连与共享入口、两个 DeepSeek profile、安装门禁、启停、恢复、并发冲突和退休来源。`tests/e2e/office_agents.py` 另验严格 JSON、CLI 冲突、身份存储切换、导出无副作用及 Bridge；`tests/e2e/pi_agent.py` 用固定官方 Pi 包核对实际 CLI 和 MCP 运行时；`tests/e2e/desktop_ui.py` 验原生写入、导入、复制和模式回退。隔离验收与厂商客户端实机验收分别记录。

办公客户端格式依据：[WorkBuddy 配置路径](https://www.codebuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/MCP-Guide)、[WorkBuddy 字段规范](https://open.workbuddy.cn/docs/connector)、[千问办公连接器](https://docs.qwenwork.cn/features/connectors)、[豆包工作插件](https://www.doubao.com/work/docs/zh-cn/articles/705018132598-plugins)。2026-11-10 实机已验证千问办公 2.0.0 的 JSON 保存、外部写入、正常重启和工具调用，以及豆包工作 2.36.3 的 HTTP、自定义 STDIO Bridge 和模型工具调用，详情见[调查记录](office-native-investigation.md)。WorkBuddy 5.9.3 的三种接入、系统钥匙串共享调用与重启复核范围见[实机验证](workbuddy-verification.md)。这不代表其他版本、Windows 厂商客户端或真实业务账号已验收。

格式依据：[ZCode MCP](https://zcode.z.ai/en/docs/mcp-services)、[OpenClaw MCP](https://docs.openclaw.ai/tools/mcp)、[Kimi Code MCP](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/mcp.html)、[DeepSeek MCP client](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/mcp/mcp-client)、[DeepSeek MCP manager](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/mcp/mcp-manager)。配置适配和 profile 写入设计参考 [CC Switch](https://github.com/farion1231/cc-switch/tree/cac921f5d7f662085800cd99684f367bd0d90e74/src-tauri/src)，保留 mcplink 自身的配置事务和 MCP 共享网关。

桌面会检查 Unix 用户的 nvm 版本目录。支持 CLI 的客户端安装位置未被识别时，可在 Agent 页点击“选择已安装的 CLI”；WorkBuddy 使用“选择已安装的程序”，macOS 选择 `WorkBuddy.app`，Windows 选择 `WorkBuddy.exe`。后台核验文件名、类型、应用包中的实际程序或 Unix 执行权限；预览和写入时均重新核验。办公客户端按预设桌面安装位置探测：千问使用 `QwenWorkCN.app` / `QwenWorkCN.exe`，豆包工作使用 `DoubaoWork.app` / `DoubaoWork.exe`；普通豆包不算豆包工作。千问办公/豆包工作生成配置不依赖安装探测。
