# Agent 配置适配

本文面向配置适配的使用与维护。完整客户端及作用域见[兼容矩阵](agent-discovery.md)。原生文件适配基线为 management API 17，聚合入口与当前桌面快照要求 API 18；桌面、CLI 和后台应使用同一构建，实际命令以 `mcplink schema` 为准。

OpenCode 使用官方现行的 2.x `mcp.servers` / `disabled` 配置结构，前后台须使用同一构建。

## Claude Code

| 作用域 | 位置 | 写入与边界 |
| --- | --- | --- |
| user | `.claude.json` 根 `mcpServers` | 默认可写；`CLAUDE_CONFIG_DIR` 可指定用户目录 |
| local | 用户文件 `projects[项目键].mcpServers` | 只纳入明确选择的项目；Git 仓库内按项目根目录定位 |
| project | 所选目录 `.mcp.json` 的 `mcpServers` | 可写服务；不修改审批、信任或权限设置 |
| 插件/托管 | 插件登记对应的默认 `.mcp.json`、显式托管 JSON | 只读；插件更新与策略由原客户端维护 |
| 其他 JSON | 显式选择的文件 | 默认只读；明确 `--scope user` 后才按根 `mcpServers` 写入，托管/插件文件仍只读 |

```sh
mcplink sources add --agent claude --scope local --project /absolute/project
mcplink sources add --agent claude --scope project --project /absolute/project
mcplink changes preview --connection CONNECTION_ID --target claude:user --mode shared
```

使用返回的来源 ID 作为项目目标。同名服务按 local → project → user 整项覆盖，不跨作用域合并字段。盘点仅标注已登记项目来源的覆盖关系；企业策略、插件端点去重、项目审批和当前会话实际加载结果由客户端核定。相同文件中不同作用域的条目可批量应用，并分别恢复。

直连输出原生 `type: stdio/http`、`command/args/env` 或 `url/headers`。Claude Code 服务条目不支持通用 `enabled`，启停在客户端 `/mcp` 维护；mcplink 可预览移除配置。原生 `headersHelper`、专用 OAuth 参数、工作目录和托管工具筛选等不能等价转换时明确拒绝，不静默丢字段。项目命令导入后保留原项目工作目录；跨上下文转换不能等价时使用共享入口。

STDIO 中的 `${NAME}` / `${NAME:-default}` 转成托管 `${env:NAME}` 并在直连时还原，环境变量名不套用托管项目占位符含义。托管调用在变量缺值时明确失败；Claude Code 自身可能保留未展开的文本。远程 URL/headers 变量还有客户端专属的凭据过滤，现阶段拒绝此类原生导入及托管变量直连；需要时显式建立托管连接并使用共享入口。原生毫秒 `timeout` 与未知服务字段同样拒绝导入。Windows local 项目键使用原生客户端的正斜线格式，物理路径仍按系统路径处理。

管理输出只显示纯环境变量引用；含默认值的环境项按凭据遮蔽，避免默认值进入界面或 `connections show` 输出。

## OpenCode

| 配置 | 服务表 | 启停 | 本地进程 |
| --- | --- | --- | --- |
| 当前 OpenCode 2.x | `mcp.servers.<name>` | `disabled`，与启用状态相反 | `type: local`、`command: [程序, 参数…]`、`environment` |

远程服务使用 `type: remote`、`url` 和 `headers`。默认发现已存在的全局 `opencode.json` / `opencode.jsonc`；目录由 `OPENCODE_CONFIG_DIR`、`XDG_CONFIG_HOME` 或用户目录确定。项目来源需显式添加，不递归推断项目树。

```sh
mcplink sources add --agent opencode --scope user
mcplink sources add --agent opencode --scope project --project /absolute/project
mcplink sources add --agent opencode --path /absolute/opencode.jsonc
mcplink skill install --agent opencode
```

无需选择版本。空文件、没有 `mcp`、空 `mcp` 或只有全局超时的配置都读取为空服务表。仅接受 `mcp.servers`；旧版直接挂在 `mcp` 下的服务、`enabled`、重复 JSON 属性及 JSON5 扩展会被拒绝，不自动迁移文件。CLI 和桌面不再提供 `layout` 选择，RPC 同样拒绝该历史参数。

JSONC 支持注释与尾逗号，编辑保留未涉及的字段、注释、空白及变量引用。`{env:NAME}` 导入后转为托管环境引用，使用后台实际继承的环境，缺值明确失败。`{file:…}`、原生专用 OAuth 参数、原生毫秒超时和非 legacy 协议目前不能完整托管，导入时给出错误；原文件保留。可另建规范化连接，明确配置托管凭据和超时，再用共享入口供客户端使用。不会读取原生 OAuth 登录缓存。

OpenCode 字面 `${…}` 与托管层的变量语法冲突，目前拒绝导入，避免静默展开原本的文本。未知服务字段、启用的 `codemode` 等没有等价执行语义时也明确拒绝；文件中无关设置仍保留。

共享条目使用当前格式编码 Bridge 的命令数组及 `environment`，只保存托管 ID 和本地配置引用。凭据更新或后台重启后已有条目继续使用同一连接。直连不能保留工具筛选等约束时要求共享，不扩大允许工具集合。新托管定义的秒级超时转换为对应毫秒字段；原生超时导入的限制仍适用。

## 其他原生格式

| 客户端 | 字段与转换边界 |
| --- | --- |
| Codex | TOML `mcp_servers`；保留原表与未知设置，inline table 只盘点不直接编辑。 |
| Hermes | YAML `mcp_servers`；原生工具 glob、白名单及资源/提示词开关不能等价直连时使用共享入口。 |
| ZCode | JSON `mcp.servers`、`enable`；非空共享回退来源保持只读，避免新表遮蔽原服务。 |
| DeepSeek Harness | 按 profile 修改 Cordis patch 中的 MCP client 插件或 manager 服务；保留无关 YAML 和动态表达式，不执行 `!!js`。 |
| OpenClaw | JSON5 `mcp.servers`、`enabled`；不使用旧 mcporter 格式，拒绝保留名称。 |
| Kimi Code | `KIMI_CODE_HOME` 下的 `mcp.json`、`mcpServers` 和 `enabled`；项目来源显式登记。 |
| WorkBuddy | 严格 JSON `mcpServers`；`timeout` 小于 1000 的正整数按秒，其余按毫秒。导出要求启动与请求超时一致，并写为毫秒；原生启停、信任及工具策略不写进服务条目。带工具限制的连接使用共享入口。 |
| 千问办公 | 当前用户共享 JSON 的 `mcpServers`，保留根偏好与 `cliConnectors`，拒绝跨表同名。身份隔离或迁移存储不写旧 JSON，不能把 SDK 路径当作桌面配置。 |
| Pi Agent | 内置 MCP 严格 JSON，允许 UTF-8 BOM；保留 `description`、`exposure` 和 `toolExposure`，不修改项目信任。 |

Pi 的用户目录支持绝对 `PI_CODING_AGENT_DIR` 或 `~/` 路径，默认 `~/.pi/agent`。适配契约核对于官方 `@earendil-works/pi-coding-agent` 1.5.1；旧包及替代内置 MCP 的第三方扩展不据此视为兼容。服务名仅允许英文字母、数字、连字符及下划线，并拒绝连字符/下划线的命名空间冲突。

Pi 项目中仅含启用或暴露偏好的条目不是完整上游，不能导入为连接。动态凭据命令、变量替换、相对工作目录、原生认证和进度超时不能等价托管时明确拒绝；`hidden` 或逐工具暴露限制不能作为无限制上游导入。托管工具限制和认证参数需要共享入口。配置写入后由用户在 Pi 中信任项目并 `/reload`。

需要客户端确认的目标通过同一规范连接生成配置，不能由序列化结果推断已加载，也不能建立虚假的原生绑定。配置文件、加密存储或运行时接口只有在具备明确管理契约时才能纳入文件事务。

## 验证

隔离 CLI 场景覆盖预览、应用、单条恢复、同文件不同作用域、并发编辑、损坏/重复 JSON、JSONC、当前格式的启停、空配置、旧格式拒绝、环境引用、共享凭据更新及后台重启。执行方式见[端到端验证](../tests/e2e/README.md)。

可选原生验收使用明确指定的客户端与隔离配置，等待实际 MCP 连接并调用工具。原生连接检查、模型账号调用和文件事务分别核对，不能互相替代。自动化固定版本及平台入口由 CI 工作流维护。

格式依据：[Claude Code MCP](https://code.claude.com/docs/en/mcp)、[OpenCode 配置](https://opencode.ai/docs/config/)、[ZCode MCP](https://zcode.z.ai/en/docs/mcp-services)、[OpenClaw MCP](https://docs.openclaw.ai/tools/mcp)、[Kimi Code MCP](https://www.kimi.com/code/docs/en/kimi-code-cli/customization/mcp.html)、[WorkBuddy MCP](https://www.workbuddy.ai/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/MCP-Guide)、[千问办公连接器](https://docs.qwenwork.cn/features/connectors)、[Pi MCP](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/mcp.md)。客户端升级后需重新核对契约和运行时行为。
