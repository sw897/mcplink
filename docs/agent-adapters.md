# Agent 配置适配（源码开发能力）

以下能力以当前源码构建为准，未全部包含在公开 v0.1.0 发布包中。先检查当前二进制的 `schema`，前后台须使用同一构建。八类 Agent 的配置管理、MCP 共享代理及安装状态校验需要 management API 9，格式及权限见[八类 Agent 接入管理](agent-discovery.md)。未安装时不可配置；原生配置应用、客户端加载、业务工具调用分别验收。

OpenCode 仅维护官方现行的 2.x 配置结构，不再兼容旧版服务表或版本选择参数。旧后台会在相关配置操作发送前被拒绝；更新时需使用同一构建的前后台。

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

## 验证范围

隔离 CLI 场景覆盖预览、应用、单条恢复、同文件不同作用域、并发编辑、损坏/重复 JSON、JSONC、当前格式的启停、空配置、旧格式拒绝、环境引用、共享凭据更新及后台重启。失败方式见[验收清单](../tests/e2e/agent-adapters-failure-modes.md)。

OpenCode 当前原生验收入口使用 `MCPLINK_E2E_OPENCODE` 指定的客户端，本机为 2.2.6。脚本隔离配置与数据目录，持续运行原生服务并等待 MCP 实际连接，避免首次异步查询空列表造成误判。Claude Code 原生验收使用 2.1.319。原生连接检查不代表模型侧业务工具调用；历史平台记录和真实账号范围见[兼容说明](compatibility.md)。

官方契约核对于 2026-11-08：[Claude Code MCP](https://code.claude.com/docs/en/mcp)、[OpenCode MCP](https://opencode.ai/docs/mcp-servers/)、[OpenCode 配置](https://opencode.ai/docs/config/)。实现和验收固定客户端版本，后续版本变化须重新验证。
