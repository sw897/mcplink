# mcplink

[![CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sw897/mcplink/actions/workflows/ci.yml)
[![MIT 许可](https://img.shields.io/badge/license-MIT-green)](LICENSE)

[English](README.md) · 简体中文

mcplink 将 MCP Server 和普通 HTTP API 接到每个 Agent 的一个 MCP 入口中，并让这些工具可从终端、脚本和自动化任务调用。服务、认证和工具访问范围可通过桌面或 CLI 统一维护。

统一入口、单服务入口和工具 CLI 复用同一份托管连接。免费、MIT 开源，使用本机管理功能无需注册 mcplink 账号。上游服务仍保留各自的认证与访问要求。

[下载](#安装与构建) · [快速上手](#快速上手) · [HTTP API Adapter](#http-api-转-mcp) · [CLI 指南](docs/agent-cli.md) · [项目文档](docs/README.md)

## 核心能力

| 能力 | 使用方式 |
| --- | --- |
| MCP 统一接入 | 每个 Agent 配置一个 `mcplink` 入口，使用所选服务的工具、资源和提示词；支持命名空间、每入口工具范围、分页和清单刷新通知。 |
| MCP 转 CLI | 直接调用工具或生成具名命令，支持 JSON stdin、Schema 校验、结构化结果和契约检查，复用托管认证。 |
| HTTP API 转 MCP | 用声明式工具参数及 HTTP 请求/结果映射注册 Webservice，通过同一个 MCP 入口和 CLI 使用，无需另写 MCP Server。 |

```mermaid
flowchart LR
    Agent["AI Agent"] -->|一个 MCP 入口| Gateway["mcplink"]
    Terminal["终端 / 脚本"] -->|工具 CLI| Gateway
    Gateway --> MCP["MCP Server"]
    Gateway --> Adapter["HTTP API Adapter"]
    Adapter --> API["HTTP API"]
```

## 安装与构建

下载 **[0.3.0 预览版](https://github.com/sw897/mcplink/releases/tag/v0.3.0)**，支持 macOS Apple Silicon 和 Windows x64。Release 提供安装包、独立 CLI、SHA-256 校验值和公开构建清单。

| 平台 | 安装方式 |
| --- | --- |
| macOS Apple Silicon | 打开 DMG 或 App ZIP，将 `mcplink.app` 复制到 `/Applications`。 |
| Windows x64 | 运行 NSIS 安装包，也可使用独立 CLI ZIP。 |
| 独立 CLI | 使用对应平台的 CLI 归档；或在源码目录运行 `cargo install --path crates/mcplink-core --locked`，将 Cargo 的可执行目录加入 PATH。 |

将应用与 CLI 保留在固定路径。升级时同步更新桌面和独立 CLI，并重启后台。0.3.0 使用数据语义版本 5、management API 21；旧程序拒绝打开新数据目录。

macOS 预览包采用 ad-hoc 签名、未公证，Windows 包未签名。Intel Mac 和 Linux 暂无已验收的安装包。兼容性、签名和校验值见[发布说明](docs/release-notes.md)与[发布指南](docs/release.md)；成功的 [CI 构建](https://github.com/sw897/mcplink/actions/workflows/ci.yml)也提供 `packages-*` 工件。

开发与打包命令见[从源码构建](#从源码构建)。

## 快速上手

1. 安装并打开 mcplink，或使用管理 CLI。
2. 导入已有服务，添加本地 STDIO 命令、远程 MCP 地址，或注册 HTTP API 定义。
3. 提供所需认证并查看工具。HTTP API 使用托管请求头，通过实际调用或显式 probe 验证访问。
4. 创建统一入口，选择服务和工具范围，再接给目标 Agent。核对并应用配置预览。
5. 重新加载 Agent 的 MCP 配置，实际调用一次工具确认可用。

后续在 mcplink 中调整服务选择，保留同一个 Agent 入口。支持实时刷新的客户端会收到清单变更通知，其他客户端需要重载 MCP。也可点击“通知 Agent 刷新”或运行 `mcplink gateway refresh ENTRY_ID`。

## 支持的 Agent

Codex、Hermes、Claude Code、OpenCode、ZCode、DeepSeek Harness、OpenClaw、Kimi Code、WorkBuddy、千问办公（QwenWork）、Pi Agent、豆包工作（Doubao Work）。

各客户端的配置作用域、支持操作和接入要求统一列在 [Agent 兼容矩阵](docs/agent-discovery.md)。项目或 profile 来源需显式添加；插件和系统托管等来源保留原有权限。

## Agent 接入与认证

| 方式 | 连接与认证 |
| --- | --- |
| 统一入口 | 一个 `mcplink` 条目提供所选服务，引用各成员原有的托管连接；工具、提示词和资源 URI 使用稳定命名空间。 |
| 单服务入口 | 一个 Bridge 提供一项托管服务，保留上游名称。 |
| 原生直连（可选） | Agent 自行连接上游并维护认证。普通 HTTP API 需要 mcplink Adapter，通过共享方式接入。 |

托管认证复用限于同一本机 mcplink 数据目录中的同一服务连接、账号和授权范围。上游允许时刷新 Token 或 OAuth grant，撤销或无法续期时需要重新认证。工具 CLI 引用这些连接，不另建账号。

共享配置保存本地程序入口和连接引用，上游凭据留在系统凭据库。直连配置包含完整地址、凭据和启动参数，应用前需核对保存与共享范围。

配置变更支持预览、并发修改检查、备份和逐条恢复。迁移到统一入口时，只有明确选择替换，才移除成员服务已有的共享条目。移动程序后，需要从新路径重新生成共享入口。

## MCP 转 CLI

直接调用连接中的工具，或为所选工具生成命令。调用方需要本机 mcplink 运行时，无需打开桌面或配置原生 MCP 客户端。以下占位符使用已有托管连接 ID 和实际工具名：

```sh
mcplink connections tools CONNECTION_ID --tool TOOL_NAME
mcplink call CONNECTION_ID TOOL_NAME --stdin
mcplink cli generate CONNECTION_ID --name notes --output ./notes-cli --tool TOOL_NAME
mcplink cli check ./notes-cli/notes.mcplink.json
```

通过 stdin 传入工具参数 JSON。生成的命令清单不含凭据，调用时复用托管认证。在服务或统一入口中打开“工具 CLI”，可以查看参数、生成命令、检查契约，或明确执行一次工具调用。输入格式、退出码和重新绑定见 [CLI 指南](docs/agent-cli.md)。

## HTTP API 转 MCP

注册基础 URL 和声明式 `httpApi` 工具定义，将普通 HTTP 操作转换为 MCP 工具，映射路径、query 参数、JSON 请求体及结果字段。此能力从 0.3.0 开始提供。

项目包含两个无 Key 示例：

```sh
mcplink connections save --stdin < examples/http-api/quickchart.json
mcplink connections save --stdin < examples/http-api/open-meteo.json
mcplink connections tools CONNECTION_ID
mcplink connections probe CONNECTION_ID
```

连接 ID 使用保存命令的返回值。QuickChart 将图表 JSON 渲染为 PNG，Open-Meteo 返回天气 JSON。例如，将 `OPEN_METEO_CONNECTION_ID` 替换为天气连接 ID：

```sh
mcplink call OPEN_METEO_CONNECTION_ID get_weather --stdin <<'JSON'
{"latitude":31.23,"longitude":121.47,"timezone":"Asia/Shanghai"}
JSON
```

将两项服务加入同一个统一入口，即可由每个 Agent 的一个 MCP 条目调用，也可直接通过 CLI 使用。工具发现只检查本地定义；示例中显式配置的 probe 会执行一次真实 API 调用。

Adapter 支持 JSON、UTF-8 文本、PNG/JPEG/WebP 图片字节和映射后的 Base64 图片结果。图片作为 MCP 图片内容返回。凭据使用托管请求头；普通 HTTP API 不使用 MCP OAuth 发现，也不能导出为原生 MCP 直连地址。

当前采用显式操作定义，尚未提供 OpenAPI 导入、multipart 上传或异步任务。公共服务保留各自的限额与条款；Open-Meteo 免费入口限非商业用途。定义方式、映射规则及限制见 [HTTP Adapter 指南](docs/http-api.md)。

## 管理 CLI 与 Skill

Agent 和脚本可以无 UI 管理 mcplink。先检查安装版本的 JSON 命令契约和实际连接状态：

```sh
mcplink schema
mcplink status
```

桌面、管理 CLI 和内置 Skill 共用连接、凭据、配置预览与恢复操作。服务盘点、账号标签、工具策略及后台控制由同一个核心维护。macOS 菜单栏和 Windows 托盘支持登录启动、暂停状态保存和退出控制。安装与管理流程见 [CLI 指南](docs/agent-cli.md)。

## 从源码构建

桌面采用 Tauri 2、React、TypeScript 和 Rust。安装 [.node-version](.node-version) 固定的 Node.js 22 LTS（最低 22.12.0）、[rust-toolchain.toml](rust-toolchain.toml) 中的工具链、Python 3.11+ 及 [Tauri 平台构建工具](https://v2.tauri.app/start/prerequisites/)。macOS 需要 Xcode Command Line Tools，Windows 需要 MSVC 构建工具和 WebView2。

```sh
git clone https://github.com/sw897/mcplink.git
cd mcplink
npm ci
npm run desktop
```

| 命令 | 用途 |
| --- | --- |
| `npm run build` | 检查类型并构建前端。 |
| `npm run check` | 检查 Rust 格式并运行静态分析。 |
| `npm run release:macos` | 构建并校验 macOS DMG、App ZIP 和 CLI 归档。 |
| `npm run release:windows` | 构建并校验 Windows NSIS 安装包和 CLI ZIP。 |

候选产物位于 `artifacts/releases/<版本>/<时间>/`，bundle 产物位于 `target/release/bundle/`。干净构建、签名和公开分发流程见[发布指南](docs/release.md)。

## 数据与认证

连接元数据和配置历史保存在本机，托管凭据使用系统凭据库。配置备份可能包含原生文件中已有的明文凭据，应按原文件的敏感程度保管。

MCP 服务支持 STDIO、Streamable HTTP 及普通 tools、resources、prompts 请求；HTTP API 成员通过 Adapter 提供工具。认证与工具行为遵循上游契约，详见[支持范围](docs/compatibility.md)。问题报告请使用合成数据，安全问题按 [SECURITY.md](SECURITY.md) 报告。

## 文档与参与

- [产品定位](docs/product.md) · [文档索引](docs/README.md) · [Agent 接入](docs/agent-discovery.md) · [CLI 指南](docs/agent-cli.md)
- [架构](docs/architecture.md) · [配置适配](docs/agent-adapters.md) · [兼容范围](docs/compatibility.md)
- [发布指南](docs/release.md) · [路线图](ROADMAP.md) · [参与开发](CONTRIBUTING.md) · [Issues](https://github.com/sw897/mcplink/issues)

## 许可

[MIT](LICENSE)。再分发时保留 [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt)，依赖及许可说明见 [docs/DEPENDENCIES.md](docs/DEPENDENCIES.md)。
