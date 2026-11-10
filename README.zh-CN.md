# mcplink：桌面 MCP 管理器与本地共享网关

[![CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sw897/mcplink/actions/workflows/ci.yml)
[![MIT 许可](https://img.shields.io/badge/license-MIT-green)](LICENSE)

[English](README.md) · 简体中文

mcplink 在一个桌面应用中管理 MCP 服务、账号授权和 AI Agent 接入。盘点已有配置，维护常用服务，再通过直连或本地共享网关，将服务接给需要的 Agent。

免费、MIT 开源，本机管理无需注册 mcplink 账号。上游服务仍按各自的规则认证和授权。

[快速上手](#快速上手) · [安装与构建](#安装与构建) · [CLI 指南](docs/agent-cli.md) · [项目文档](docs/README.md)

## 功能

- **服务盘点**：查看本机 MCP 服务、配置来源、连接状态和可用工具。
- **连接与账号**：新建或导入连接，维护账号标签，管理 Token 或 OAuth 授权。
- **Agent 接入**：为各客户端选择服务和接入方式，跨 Agent 复用托管连接。
- **预览与恢复**：检查配置变更，检测并发修改，备份文件并逐条恢复操作。
- **桌面与 CLI**：桌面、JSON CLI 和内置管理 Skill 共用管理核心，可直接调用工具或生成轻量命令入口。
- **后台控制**：通过 macOS 菜单栏或 Windows 托盘管理共享网关、登录启动、暂停状态和退出方式。

## 支持的 Agent

Codex、Hermes、Claude Code、OpenCode、ZCode、DeepSeek Harness、OpenClaw、Kimi Code、WorkBuddy、千问办公、Pi Agent、豆包工作。

各客户端的配置作用域、支持操作和接入要求统一列在 [Agent 兼容矩阵](docs/agent-discovery.md)。项目或 profile 来源需显式添加；插件和系统托管等来源保留原有权限。

## 直连与共享

| 方式 | 运行方式 | 适用情况 |
| --- | --- | --- |
| 直连 | Agent 使用自身配置，直接启动或连接上游 MCP 服务。 | 希望由客户端独立运行服务、维护授权。 |
| 共享网关 | 本地 Bridge 使用 mcplink 托管的连接、凭据和工具筛选。 | 多个 Agent 共用服务账号，或在一处更新托管授权。 |

直连配置包含完整地址、凭据和启动参数，应用前需核对保存及共享范围。共享配置只保存本地程序入口和连接引用，上游凭据留在系统凭据库中。

每项服务保留自己的接入名称。共享网关保留上游工具名称和 Schema，具体显示方式由客户端决定。

## 快速上手

1. 构建并打开 mcplink。设置共享接入前，将应用或独立 CLI 放在长期保留的位置。
2. 查看发现的服务与 Agent，按需添加项目或 profile 配置来源。
3. 导入已有连接，或填写 STDIO 命令、远程 MCP 地址新建连接。完成所需认证，再检查工具清单。
4. 选择目标 Agent 和接入方式，核对配置内容，按界面提示完成接入。
5. 重新加载客户端的 MCP 配置，并做一次实际工具调用确认结果。

后续可在服务与账号页面更新托管授权，在变更历史中检查和恢复操作。移动程序后，需要从新位置重新生成共享入口。

## 安装与构建

技术栈为 Tauri 2、React、TypeScript 和 Rust。安装 [.node-version](.node-version) 固定的 Node.js 22 LTS（最低 22.12.0）、[rust-toolchain.toml](rust-toolchain.toml) 中的工具链、Python 3.11+ 及 [Tauri 平台构建工具](https://v2.tauri.app/start/prerequisites/)。

macOS 需要 Xcode Command Line Tools；Windows 需要 MSVC 构建工具和 WebView2。

```sh
git clone https://github.com/sw897/mcplink.git
cd mcplink
npm ci
npm run desktop
```

| 目标 | 构建与安装 |
| --- | --- |
| macOS Apple Silicon | `npm run release:macos` 生成 DMG 和 App ZIP，将 `mcplink.app` 安装到 `/Applications`。 |
| Windows x64 | `npm run bundle` 在 `target/release/bundle/nsis/` 生成 NSIS 安装包。 |
| 独立 CLI | `cargo install --path crates/mcplink-core --locked`，将 Cargo 的可执行目录加入 PATH。 |

`npm run build` 构建前端，`npm run check` 检查 Rust 格式与静态问题。macOS 发布产物位于 `artifacts/releases/<版本>/<时间>/`。

默认 macOS 预览包使用 ad-hoc 签名，Windows 安装包未签名。签名、公证、构建清单和校验值见[发布指南](docs/release.md)。Intel Mac 和 Linux 暂无已验收的安装包。

## CLI 与管理 Skill

CLI 提供 JSON 输出和 stdin 输入，供脚本和 Agent 使用。先检查当前安装版本的命令与连接状态：

```sh
mcplink schema
mcplink status
```

内置管理 Skill 使用同一套 CLI 和配置事务。安装目标、连接设置、授权、工具调用、命令生成和恢复操作见 [CLI 指南](docs/agent-cli.md)。

## 数据与认证

连接元数据和配置历史保存在本机，托管凭据使用系统凭据库。配置备份可能包含原生文件中已有的明文凭据，需要按原文件的敏感程度保管。

支持 STDIO、Streamable HTTP 及普通 tools、resources、prompts 请求。认证与工具行为遵循上游服务契约。平台和协议限制见[支持范围](docs/compatibility.md)；问题报告请使用合成样例，安全问题按 [SECURITY.md](SECURITY.md) 报告。

## 文档与参与

- [文档索引](docs/README.md) · [Agent 接入](docs/agent-discovery.md) · [CLI 指南](docs/agent-cli.md)
- [架构](docs/architecture.md) · [配置适配](docs/agent-adapters.md) · [兼容范围](docs/compatibility.md)
- [发布指南](docs/release.md) · [路线图](ROADMAP.md) · [参与开发](CONTRIBUTING.md) · [Issues](https://github.com/sw897/mcplink/issues)

## 许可

[MIT](LICENSE)。再分发时保留 [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt)，依赖及许可说明见 [docs/DEPENDENCIES.md](docs/DEPENDENCIES.md)。
