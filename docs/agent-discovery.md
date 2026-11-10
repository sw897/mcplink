# Agent 接入管理

从同一服务列表选择目标 Agent 和直连或共享方式。mcplink 根据客户端支持的配置能力提供相应操作；配置作用域、自动写入和客户端确认的边界如下。

## 兼容矩阵

| 客户端（CLI 标识） | 用户配置或接入位置 | 项目或 profile | 管理边界 |
| --- | --- | --- | --- |
| Codex `codex` | `$CODEX_HOME/config.toml`，默认 `~/.codex/config.toml` | 显式添加 TOML 文件 | `mcp_servers`、`enabled`；配置事务与共享代理 |
| Hermes `hermes` | `$HERMES_HOME/config.yaml`，默认 `~/.hermes/config.yaml` | 显式添加 YAML 文件 | `mcp_servers`、`enabled`；工具及资源限制按可等价范围处理 |
| Claude Code `claude` | 用户 `.claude.json` | 显式 local/project 来源 | `mcpServers`；用户和项目配置可写，插件及托管策略只读；审批与项目启停在客户端维护 |
| OpenCode `opencode` | 用户 `opencode.json` / `opencode.jsonc` | 显式项目来源 | 当前 `mcp.servers`、`disabled` 格式；保留 JSONC 注释 |
| ZCode `zcode` | `~/.zcode/cli/config.json` | `<项目>/.zcode/config.json` | `mcp.servers`、`enable`；非空 `.agents/mcp.json` 回退来源只读 |
| DeepSeek Harness `deepseek` | `$DSH_HOME/profiles/<profile>/cordis.patch.yml`，默认 `~/.dsh` | 显式 profile | Cordis MCP client 插件或 manager 映射；基础组合只读 |
| OpenClaw `openclaw` | `$OPENCLAW_CONFIG_PATH`，默认 `$OPENCLAW_STATE_DIR/openclaw.json` / `~/.openclaw/openclaw.json` | 显式配置文件 | JSON5 `mcp.servers`、`enabled` |
| Kimi Code `kimicode` | `$KIMI_CODE_HOME/mcp.json`，默认 `~/.kimi-code/mcp.json` | `<项目>/.kimi-code/mcp.json` | `mcpServers`、`enabled`；不读取旧 kimi-cli 的 `~/.kimi` |
| WorkBuddy `workbuddy` | `~/.workbuddy/mcp.json` | `<项目>/.workbuddy/mcp.json` | 严格 JSON 的 `mcpServers`；启停、信任与工具策略在客户端维护 |
| 千问办公 `qwenwork` | `~/.qwenworkcn/mcp.json` | 当前用户来源 | 共享 JSON 存储支持配置事务和 `enabled`；身份隔离或迁移存储需在客户端确认配置 |
| Pi Agent `pi` | `$PI_CODING_AGENT_DIR/mcp.json`，默认 `~/.pi/agent/mcp.json` | `<项目>/.pi/mcp.json` | 内置 MCP 严格 JSON；支持 `enabled`，保留描述与工具暴露设置；项目信任在 Pi 中维护 |
| 豆包工作 `doubaowork` | 客户端的自定义连接器设置 | 由客户端管理 | 生成 HTTP/STDIO 配置后在客户端保存；当前不托管其本地配置文件或建立绑定 |

## 操作流程

1. **盘点**：查看服务、客户端和来源。已有配置目录只表示发现来源，不代表已安装客户端或已登录。
2. **准备连接**：导入已有条目或新建连接，完成授权并检查工具。共享接入前需检查成功。
3. **接入**：选择目标与模式，核对配置内容。可写来源通过预览和应用事务处理；需要客户端确认的目标按界面提示完成设置。
4. **加载**：重新加载客户端或开启新会话，实际调用工具。写入、生成或复制成功均不代表客户端已获信任或已加载。
5. **维护**：更新托管授权、同步直连定义，或恢复原生配置操作。仅生成配置不会产生绑定或配置操作历史。

共享入口只包含 mcplink 程序、管理数据目录和连接引用，凭据与工具过滤在后台维护。直连配置包含上游地址、启动参数及可能的凭据，需要核对保存与共享范围。

## 来源与权限

用户来源按表中的路径发现；项目和 profile 必须显式登记。客户端未安装时不可写入原生文件；已安装但未被识别时，可选择实际 CLI 或应用程序，后台重新核验安装入口。

插件、托管策略及共享回退文件按具体来源保留只读权限，不能通过另一客户端类型或路径绕过。符号链接可盘点，写入须选择真实文件。盘点不会读取私有登录缓存、执行凭据命令或替客户端授予信任。

预览不创建文件；应用前复核安装状态、连接修订和文件哈希。外部编辑、账号更新或存储模式切换都会使旧计划失效。恢复按条目执行，保留无关改动。

## 客户端确认

需要在客户端保存的配置，通过“接给 Agent”生成。CLI 使用 `connections export-config`，可离线准备内容；目标和输出结构以 `schema` 为准。共享内容使用本机 STDIO Bridge，直连须显式选择并包含完整上游配置。

千问办公的 JSON 可在连接器添加界面粘贴保存；豆包工作的 HTTP URL/Headers 或 STDIO command、逐项 args/env 在自定义连接器中填写。参数顺序、空参数、空格和引号需要保持原样。客户端更新后以其当前设置页面为准。

千问办公的共享 JSON 保留偏好和 `cliConnectors`，拒绝跨表同名及未验证执行字段。检测到身份隔离、迁移或无法核对存储模式时，停止旧 JSON 的写入与恢复，使用客户端确认流程。豆包工作目前没有可供此适配器使用的独立本地配置管理契约。

## 格式与验证

字段转换、作用域和不等价选项见[配置适配](agent-adapters.md)，平台与协议限制见[兼容说明](compatibility.md)。桌面、CLI 和后台应使用同一构建；先运行 `mcplink schema` 与 `mcplink status` 获取实际命令和来源 ID。

自动化测试入口见[端到端验证](../tests/e2e/README.md)。隔离文件事务、实际客户端加载和模型账号调用分别验证；客户端本身的认证、审批和工具策略继续由原客户端维护。
