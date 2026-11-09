# mcplink：本地 MCP 接入管理器

[![CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sw897/mcplink/actions/workflows/ci.yml)
[![版本](https://img.shields.io/badge/version-0.1.0%20preview-blue)](https://github.com/sw897/mcplink/releases/tag/v0.1.0)
[![MIT 许可](https://img.shields.io/badge/license-MIT-green)](LICENSE)

**[English](README.md) · 简体中文**

mcplink 是本地桌面 **MCP（Model Context Protocol）管理器与 Gateway**。当前源码支持 **Codex、Hermes、Claude Code、OpenCode、ZCode、DeepSeek Harness、OpenClaw、Kimi Code** 八类客户端的接入管理与 MCP 共享代理；未安装的客户端显示“未安装”且不可配置，安装后刷新即可接入。在同一处查看 MCP 服务、维护托管凭据，并通过桌面或 JSON CLI 管理连接，具体格式及来源限制见[八类 Agent 接入](docs/agent-discovery.md)。

**v0.1.0 为预览版，提供 macOS Apple Silicon 与 Windows x64 下载。** macOS 包使用 ad-hoc 签名，尚无 Developer ID 签名和公证；Windows 安装包未签名。安装前请核对下方平台范围。

## 能做什么

- **盘点已有服务**：读取原生 MCP 配置，按需将连接纳入托管。
- **直连或共享**：按服务选择原生直连或共享本地 Gateway；关闭桌面窗口后，共享连接仍可调用。
- **后台运行**：Windows 托盘与 macOS 菜单栏支持关窗留驻、登录自启和窗口唤起；共享暂停跨重启保留，完整退出等待服务清理。详见 [Windows](docs/windows-background.md) 与 [macOS](docs/macos-background.md) 的行为和验收范围。
- **先预览再修改**：核对目标文件和变更，检查并发修改，备份并逐条恢复操作。
- **复用托管凭据**：用系统凭据库保存 Secret，在一处更新共享连接的授权。
- **桌面与 CLI 共用核心**：两种入口使用相同的 Rust 管理核心、后台和配置事务。
- **安装管理 Skill**：让 Agent 通过内置 mcplink Skill 使用 CLI 管理连接。

支持 STDIO、Streamable HTTP MCP 连接。协议与认证边界见[兼容范围](docs/compatibility.md)和[架构](docs/architecture.md)。业务 Skill 的编写和版本分发由用户自行管理。

## 下载与安装

[v0.1.0 发布页](https://github.com/sw897/mcplink/releases/tag/v0.1.0)提供安装包、SHA-256 校验清单及源码与产物清单。

| 平台 | 下载 |
| --- | --- |
| macOS · Apple Silicon | [DMG 安装包](https://github.com/sw897/mcplink/releases/download/v0.1.0/mcplink_0.1.0_aarch64.dmg) · [App ZIP](https://github.com/sw897/mcplink/releases/download/v0.1.0/mcplink_0.1.0_aarch64.app.zip) |
| Windows · x64 | [NSIS 安装包](https://github.com/sw897/mcplink/releases/download/v0.1.0/mcplink_0.1.0_x64-setup.exe) |
| macOS · Apple Silicon · 独立 CLI | [CLI tar.gz](https://github.com/sw897/mcplink/releases/download/v0.1.0/mcplink_cli_0.1.0_aarch64-apple-darwin.tar.gz) |
| SHA-256 校验清单 | [SHA256SUMS](https://github.com/sw897/mcplink/releases/download/v0.1.0/SHA256SUMS) |

macOS：将 DMG 内的 `mcplink.app` 复制到 `/Applications`，再配置共享入口。系统可能拦截未经公证的预览版；请先核对来源与校验值，再参照 [Apple 的应用打开说明](https://support.apple.com/en-us/102445)处理系统提示。Windows：运行 x64 安装程序，系统可能提示发布者未经验证。

计算下载文件的 SHA-256，与 `SHA256SUMS` 中对应文件名的记录比较：

```sh
# macOS
shasum -a 256 mcplink_0.1.0_aarch64.dmg
```

```powershell
# Windows PowerShell
Get-FileHash .\mcplink_0.1.0_x64-setup.exe -Algorithm SHA256
```

应用和独立 CLI 应放在长期保留的位置。共享入口使用程序绝对路径；移动后需从新位置重新预览并应用。

## 开始使用

1. 打开应用，核对扫描到的服务及配置来源。其他项目或 profile 可通过“配置来源”添加真实配置文件。
2. 选中连接并检查工具及认证状态。
3. 选择目标 Agent、直连或共享模式，核对预览后应用。
4. 按提示重新加载 MCP 或开启新的 Agent 会话。

配置扫描、磁盘写入成功均不代表运行中的 Agent 已加载连接。

### Agent 范围

| Agent | 盘点与导入 | 配置写入 |
| --- | --- | --- |
| Codex | 用户配置，以及手动添加的 TOML 文件 | 支持 |
| Hermes | 用户配置，以及手动添加的 YAML 文件 | 支持 |
| Claude Code | 用户配置、已启用用户插件，以及手动添加的 JSON 文件 | 只读 |

上表描述 v0.1.0。源码开发能力新增 Claude Code user/local/project 可写作用域，以及当前 OpenCode 2.x JSON/JSONC 适配，尚未包含在下载包中，详见[适配范围与验证](docs/agent-adapters.md)。OpenCode 不再支持旧版配置结构。客户端审批、策略和登录缓存继续由原客户端维护。

### CLI 与管理 Skill

解压独立 CLI 并保留许可文件，或使用 macOS app 内的 `/Applications/mcplink.app/Contents/MacOS/mcplink`。将 `mcplink` 放入 PATH 后：

```sh
mcplink schema
mcplink status
mcplink skill install --agent codex
mcplink skill install --agent hermes
```

例如，导入已有服务，再预览接给另一个 Agent：

```sh
mcplink connections import --source codex:user --name notes
mcplink connections probe CONNECTION_ID
mcplink changes preview --connection CONNECTION_ID --target hermes:user --mode shared
mcplink changes apply PLAN_ID
```

将 `notes` 替换为已有服务名称；`CONNECTION_ID`、`PLAN_ID` 使用前一步返回的值。凭据通过 stdin 传入。命令、JSON 输入、退出码、MCP 请求和恢复流程见 [CLI 指南](docs/agent-cli.md)，`mcplink schema` 提供命令契约。

## 平台验证状态

| 平台 | v0.1.0 已验证 | 签名 |
| --- | --- | --- |
| macOS Apple Silicon | 本地桌面流程；CI 构建与 44 项核心/CLI/来源隔离场景；DMG 和 5 项安装后场景 | ad-hoc；无 Developer ID 和公证 |
| Windows x64 | CI 构建与 44 项核心/CLI/来源隔离场景；安装包内容、许可和编译路径检查 | 未签名 |
| Intel Mac / Linux | 无发布包，未实机验收 | — |

Windows 桌面交互、全新 macOS 账号首次钥匙串授权，以及网络下载后的 Gatekeeper 验收尚未完成。隔离测试使用合成 MCP/OAuth 服务和 fixture 凭据，具体上游服务仍需单独核验。首版不提供自动更新。

## 从源码运行

技术栈为 **React + TypeScript、Tauri 2、Rust**。开发与 CI 使用 **Node.js 22 LTS**，具体版本固定在 [.node-version](.node-version)，最低支持 **22.12.0**。另需 [rust-toolchain.toml](rust-toolchain.toml) 固定的 Rust 版本、Python 3.11+，以及 [Tauri 平台构建工具](https://v2.tauri.app/start/prerequisites/)。macOS 需要 Xcode Command Line Tools；Windows 需要 MSVC 构建工具和 WebView2。

```sh
git clone https://github.com/sw897/mcplink.git
cd mcplink
npm ci
npm run desktop
```

从检出目录安装独立 CLI：

```sh
cargo install --path crates/mcplink-core --locked
```

构建与验证：

```sh
npm run build
npm run check
npm run bundle
python3 tests/e2e/run.py
python3 tests/e2e/agent_cli.py --record artifacts/e2e/agent-cli/local
python3 tests/e2e/claude_sources.py --record artifacts/e2e/claude/local
```

`npm run check` 会构建默认测试使用的 debug sidecar；`MCPLINK_E2E_BINARY` 可指定其他待测程序。测试使用隔离 home 与合成服务，不操作日常 Agent 配置，记录进入忽略的 `artifacts/`。`npm run release:macos` 生成经过验收、附校验清单的 macOS 候选包；签名和发布流程见[发布指南](docs/release.md)。

## 数据与认证

配置、管理元数据和后台保存在本机。SQLite 保存管理元数据，托管 Secret 保存在系统凭据库；连接仍会访问用户配置的上游服务。配置备份可能包含原客户端已有的敏感字段，应按本地凭据管理。

标准 OAuth、手工 Token 和原生客户端登录有各自的支持边界，详见[架构与认证](docs/architecture.md)。问题报告请使用合成样例，不附真实凭据、配置或数据库；漏洞通过 [SECURITY.md](SECURITY.md) 中的渠道私密报告。

## 项目文档

- [兼容范围](docs/compatibility.md) · [架构](docs/architecture.md) · [CLI 指南](docs/agent-cli.md)
- [Agent 配置适配](docs/agent-adapters.md) · [真实上游验收](docs/real-upstreams.md)
- [构建、签名与发布](docs/release.md) · [v0.1.0 发布说明](docs/releases/v0.1.0.md)
- [开发与贡献](CONTRIBUTING.md) · [路线图](ROADMAP.md) · [反馈问题](https://github.com/sw897/mcplink/issues)

## 许可

mcplink 使用 [MIT License](LICENSE)。分发时请保留[第三方许可声明](THIRD_PARTY_NOTICES.txt)；依赖信息见[依赖与许可](docs/DEPENDENCIES.md)。
