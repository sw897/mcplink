# 支持范围与验证

以下为当前实现与已检查的范围。依赖版本由 Cargo.lock 和 package-lock.json 固定，客户端更新后应重新验证配置格式与加载方式。

## 平台与 Agent

| 项目 | 范围 |
| --- | --- |
| macOS | Apple Silicon；已在 macOS 27 上验证本地构建与桌面流程。候选包使用 ad-hoc 签名，Developer ID 与公证尚待完成。 |
| Windows | x64/NSIS 构建、八类 Agent 配置和后台生命周期已通过 CI 隔离验收；托盘和桌面行为记录见 [Windows 后台说明](windows-background.md)。 |
| Linux / Intel Mac | 未实机验收。 |
| Codex | 用户 config.toml，以及手动添加的 TOML 来源；已检查 CLI 0.166.0、0.167.0 的相关入口。 |
| Hermes | 用户 config.yaml，以及手动添加的 YAML 来源；已检查 0.23.0 的相关入口。 |
| Claude Code | 支持 user/local/project 预览、写入及恢复；插件与托管来源只读。已验证 2.1.319 的隔离原生加载及 MCP 连接。 |
| OpenCode | 仅维护当前 2.x `mcp.servers` / `disabled` 格式，支持全局和显式项目 JSON/JSONC 的读写、启停、共享及恢复；不支持旧版服务表和版本选择参数。Windows 本机 2.2.6 已在隔离配置中加载共享入口并连接；不含模型业务调用。 |
| ZCode | 用户及显式项目 `mcp.servers`，支持配置事务与共享代理；非空 `.agents/mcp.json` 回退来源保持只读，避免被新表遮蔽。 |
| DeepSeek Harness | profile patch 的静态 MCP client 插件和 manager `config.servers` 支持管理与共享代理；基础组合只读。 |
| OpenClaw | 当前 JSON5 `mcp.servers`，支持配置事务与共享代理；不使用旧 mcporter 格式。 |
| Kimi Code | 当前 `KIMI_CODE_HOME` / `~/.kimi-code/mcp.json` 及显式项目来源，支持配置事务与共享代理；不读取旧 `~/.kimi`。 |
| WorkBuddy | 用户及显式项目 `.workbuddy/mcp.json` 严格 JSON，支持文件事务、直连与共享；启停/信任和工具策略在原客户端维护。macOS 5.9.3 已验三种接入的真实模型调用；另通过系统钥匙串保存合成凭据、认证上游共享调用和重启复核。[范围与字段依据](workbuddy-verification.md)。 |
| 千问办公 | 共享 JSON 支持原生预览、写入、启停和恢复；身份隔离或迁移时回退到导入。macOS 2.0.0 已验外部写入、重启及工具调用；导入仍可离线准备。 |
| Pi Agent | 内置 MCP 用户及显式项目的严格 JSON，支持直连、共享、启停和恢复；保留工具暴露设置，项目信任由用户维护。已用官方 1.5.1 CLI 发现配置，并用其实际 MCP 运行时调用合成服务；未做模型账号回合。详情见 [Pi Agent](pi-agent.md)。 |
| 豆包工作 | 生成自定义 HTTP/STDIO 连接器字段，支持直连/共享，可离线准备；不写客户端存储或建立绑定。macOS 2.36.3 已验 HTTP、共享 Bridge、重启和模型工具调用；缺少独立原生管理契约，依据见[办公客户端调查](office-native-investigation.md)。 |

支持十二类客户端，十一类支持原生文件，豆包工作使用客户端导入，千问另保留导入及身份存储回退。适配及限制见[Agent 接入](agent-discovery.md)和[配置适配](agent-adapters.md)；不修改客户端审批或信任，不读取原生登录缓存。文件事务、原生加载和模型侧工具调用分别记录，后者不因配置写入或连接检查通过而自动视为通过。

此前运行核心和桌面已通过[macOS、Windows CI 基线验收](https://github.com/sw897/mcplink/actions/runs/42572006882)，覆盖原有八类 Agent 配置和共享代理、工具 CLI、合成 OAuth 上游及后台进程清理；macOS 另验收 DMG 和安装后场景。当前[双平台 CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml)配置了 WorkBuddy、千问办公/豆包工作专项及 Pi 官方 MCP 运行时验收，逐平台保存报告；新增步骤的远端结果以对应提交的 Actions 为准。原生客户端加载、真实账号调用和模型侧业务执行分别记录，不能相互替代。

默认扫描已支持的用户入口。项目/profile 通过手动添加真实配置文件纳入管理，不推断尚未选择的项目或运行中会话的覆盖关系。

## 配置与运行环境

- TOML 表、YAML 映射和 JSON/JSONC/JSON5 编辑保留未涉及条目及格式；JSONC 支持注释与尾逗号，拒绝重复键及 JSON5 扩展，OpenClaw 单独使用 JSON5 解析。TOML inline table 可盘点，暂不直接编辑。
- 符号链接来源可读，写入需选择真实文件。预览后的外部修改会使计划失效。
- STDIO 命令必须能在桌面的运行环境中找到。推荐使用绝对路径，应用不会自动加载用户 Shell。
- 不读取原生 Agent 的私有登录存储或 `.env`。扫描结果不能证明客户端已登录。
- 账号标签由用户维护，不代表上游核实的真实身份或组织。

## MCP 与认证

支持 STDIO、Streamable HTTP，以及普通的 tools、resources、prompts 请求。当前不支持旧 HTTP+SSE endpoint、tasks、sampling、elicitation、订阅或完整的取消传播。失败中的工具调用不会自动重发。

共享入口原样转发上游工具名称和 Schema。客户端可能自行正规化名称；会产生冲突的接入名会在写入前拒绝。服务、工具参数和账号上下文的兼容性仍需按实际任务核对。

标准 OAuth、PKCE 和刷新令牌轮转使用合成上游进行端到端验证。腾讯文档的真实 Token 共享、更新与双端文档读取曾完成验证；其路径是手工 Token 更新，不声明自动刷新能力。其他服务需要独立确认认证和协议支持。

源码开发能力已修复 GitHub 远程 MCP 的初始化版本兼容：使用已实现的 legacy `initialize` 流程时明确请求 `2025-11-25`，避免 SDK 默认 `2026-07-28` 与上游会话版本不一致。已在官方只读远程入口验证手工 Token 的发现、只读仓库文件读取、两个 Bridge、通用及生成 CLI，以及同一连接的凭据更新。以上不代表全部 2026 协议能力或原生 Agent 加载已验收。

预注册应用的 PKCE、显式 scope、固定回调、`client_secret_basic` / `client_secret_post`、并发刷新和后台重启已完成合成端到端验证。GitHub 真实 OAuth 尚缺可用的注册应用与账号同意，不能以手工 Token 或合成结果代替。腾讯会议官方 MCP 已验证真实专用 Token、只读会议列表、双 Bridge、通用/生成 CLI、托管凭据更新、无效值后恢复和后台重启；该长期 Token 不计为 OAuth 刷新验收。该服务会把部分业务 HTTP 错误包装在 `isError: false` 的结果中，须另行校验业务状态。详细范围见[真实上游验收](real-upstreams.md)。飞书与腾讯 IMA 仍待相应官方接入、应用权限及测试对象；实施步骤见[路线图](../ROADMAP.md#后续方向)。

## 加载与恢复

磁盘写入成功后，Agent 仍需重新加载 MCP 或开启新会话。界面显示“待 Agent 加载”，不会把写入成功当作运行时已生效。

共享入口使用发起预览的应用路径；应用移动后需从新位置重新预览并应用。后台按需启动，关闭窗口不关闭后台。异常重启后下一次调用可重新连接，执行结果不确定的写入需要先核对。

操作历史按条目恢复；同一条目有后续修改时拒绝覆盖，无关条目的修改会保留。托管定义更新后，已有直连配置需单独预览同步。

macOS 首次访问钥匙串或受系统保护的目录时可能需要系统授权，等待期间应处理系统提示后刷新。隔离 fixture 验证不能代替真实钥匙串、全新系统账号或下载后的 Gatekeeper 验收。
