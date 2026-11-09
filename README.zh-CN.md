# mcplink：本地 MCP 管理器与共享网关

[![CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sw897/mcplink/actions/workflows/ci.yml)
[![MIT 许可](https://img.shields.io/badge/license-MIT-green)](LICENSE)

[English](README.md) · 简体中文

mcplink 是面向个人的桌面 MCP（Model Context Protocol）接入管理器。你可以在一个应用里查看本机 MCP 服务、维护连接和账号授权，再把服务接给需要的 AI Agent。根据客户端和连接的支持情况，选择原生直连，或通过本地共享网关复用托管连接。

桌面版免费、MIT 开源，本机管理无需注册 mcplink 账号。上游 MCP 服务仍按各自的规则认证和授权。

[快速上手](#快速上手) · [安装与运行](#安装与运行) · [从源码运行](#从源码运行) · [CLI 指南](docs/agent-cli.md)

## 功能

| 功能           | 可以做什么                                                                                         |
| -------------- | -------------------------------------------------------------------------------------------------- |
| 本机服务盘点   | 查看已有 MCP 服务、配置来源、接入的 Agent 和连接状态；按需添加项目或 profile 配置。                |
| 连接与账号维护 | 导入或新建连接，维护账号标签、Token 和 OAuth 授权，检查服务提供的工具。                            |
| 跨 Agent 接入  | 为不同客户端选择服务和接入方式，将同一托管连接接给多个 Agent。                                     |
| 配置预览与恢复 | 写入前查看目标文件和变更，检测并发修改，备份配置，并按操作记录逐条恢复。                           |
| 工具 CLI       | 通过 JSON CLI 管理连接、按 Schema 校验并调用工具，也可为选定工具生成轻量命令入口。                 |
| 桌面后台运行   | 在 macOS 菜单栏或 Windows 托盘查看共享状态，设置登录启动、暂停共享，以及选择完整退出或仅退出界面。 |

桌面、CLI 和内置管理 Skill 使用同一份托管数据与配置操作流程。共享模式下，在 mcplink 更新凭据后，已接入的 Agent 继续引用同一连接。

## 直连与共享

| 方式     | 连接如何运行                                                         | 适用情况                                                |
| -------- | -------------------------------------------------------------------- | ------------------------------------------------------- |
| 原生直连 | mcplink 按客户端格式写入配置，由 Agent 直接启动或连接上游 MCP 服务。 | 希望由客户端独立运行服务、维护原生认证。                |
| 共享网关 | Agent 通过本地 Bridge 使用 mcplink 托管的连接、凭据和工具筛选。      | 多个 Agent 需要共用同一服务账号，或希望在一处更新授权。 |

直连会把填写的令牌、启动参数和完整地址写入所选原生配置文件。选择项目配置时，请核对它是否会随 Git 共享。

共享接入的原生配置只保存本地程序入口和连接引用，上游凭据保存在系统凭据库中。每项服务保留自己的接入名称；共享入口保留上游工具名称和 Schema，具体工具在 Agent 中的显示方式由客户端决定。

关闭窗口后，应用可留在菜单栏或托盘，共享暂停状态会在重启后保留。选择完整退出会等待共享服务清理；选择仅退出界面可保留代理运行。后续 Agent 调用也能按需启动后台。详细行为见 [macOS 后台运行](docs/macos-background.md)和 [Windows 后台运行](docs/windows-background.md)。

## 支持的 Agent

mcplink 支持 Codex、Hermes、Claude Code、OpenCode、ZCode、DeepSeek Harness、OpenClaw、Kimi Code、WorkBuddy、千问办公和豆包工作。

mcplink 会识别客户端安装状态并盘点支持的用户配置。未安装的客户端仍显示在列表中，安装并刷新后即可写入原生文件。项目或其他 profile 的配置需显式添加；插件和系统托管等只读来源保留只读权限。

新增 WorkBuddy 原生文件管理，支持用户及显式项目 `.workbuddy/mcp.json` 的预览、写入、移除和恢复。千问办公可生成连接器导入 JSON，豆包工作可生成 HTTP/STDIO 连接器的填写字段，均支持直连或共享模式。生成配置不会建立绑定或确认客户端已接入，需要在对应客户端完成导入并验证工具调用；本机未安装也可准备导入配置。

各客户端的配置格式、导入步骤、可写来源和原生启停方式见[Agent 接入管理](docs/agent-discovery.md)及[配置适配](docs/agent-adapters.md)。

## 快速上手

1. 按[源码构建](#从源码运行)运行或打包 mcplink，并将应用或独立 CLI 放在长期保留的位置。共享配置会引用它的程序路径。
2. 打开应用，查看本机服务和已安装的 Agent；按需添加项目或 profile 的真实配置文件。
3. 导入已有连接，或填写 STDIO 启动命令、远程 MCP 地址来新建连接。设置所需凭据或完成 OAuth 授权，再检查工具清单。
4. 选择目标 Agent 和直连或共享方式，核对配置预览后应用。
5. 按提示重新加载 Agent 的 MCP 配置，并做一次实际工具调用确认接入结果。

后续可在账号与服务页面更新托管授权，在“变更与恢复”中检查和恢复操作。移动程序后，需要从新位置重新预览并应用共享配置。

## 安装与运行

通过[源码构建](#从源码运行)使用桌面版或独立 CLI：

| 平台                | 安装方式                                                                                               |
| ------------------- | ------------------------------------------------------------------------------------------------------ |
| macOS Apple Silicon | 运行 `npm run release:macos` 生成 DMG 和 App ZIP，将 `mcplink.app` 安装到 `/Applications`。            |
| Windows x64         | 运行 `npm run bundle` 生成 NSIS 安装包，再执行安装。                                                   |
| 独立 CLI            | 在源码目录运行 `cargo install --path crates/mcplink-core --locked`，并将 Cargo 的可执行目录加入 PATH。 |

macOS 发布候选位于 `artifacts/releases/<版本>/<时间>/`，Windows 安装包位于 `target/release/bundle/nsis/`。每次交付应核对随附的校验值和源码清单。

默认 macOS 预览构建使用 ad-hoc 签名，Windows 安装包未签名；正式签名与公证配置见[发布指南](docs/release.md)。Intel Mac 和 Linux 暂无已验收的安装包。

## CLI 与管理 Skill

CLI 提供 JSON 输出和 stdin 输入，可用于脚本或 Agent 自动化。将 `mcplink` 放入 PATH 后，先查看当前构建的命令契约和连接状态：

```sh
mcplink schema
mcplink status
```

例如，将 Codex 中名为 `notes` 的已有服务接给 Hermes：

```sh
mcplink connections import --source codex:user --name notes
mcplink connections probe CONNECTION_ID
mcplink changes preview --connection CONNECTION_ID --target hermes:user --mode shared
mcplink changes apply PLAN_ID
```

用 `status` 确认配置来源 ID，将 `notes` 换成实际服务名；`CONNECTION_ID` 和 `PLAN_ID` 使用前一步返回的值。凭据通过 stdin 传入。

内置管理 Skill 可安装到 Codex、Hermes、Claude Code 或 OpenCode，让 Agent 使用同一套 CLI 管理流程：

```sh
mcplink skill install --agent codex
```

工具调用、轻量 CLI 生成、账号授权和操作恢复的完整示例见 [CLI 指南](docs/agent-cli.md)。

## 从源码运行

技术栈为 Tauri 2、Rust、React 和 TypeScript。开发与 CI 使用 [.node-version](.node-version) 固定的 Node.js 22 LTS，最低支持 22.12.0；Rust 版本固定在 [rust-toolchain.toml](rust-toolchain.toml)。另需 Python 3.11+ 和 [Tauri 平台构建工具](https://v2.tauri.app/start/prerequisites/)。

macOS 需要 Xcode Command Line Tools；Windows 需要 MSVC 构建工具和 WebView2。

```sh
git clone https://github.com/sw897/mcplink.git
cd mcplink
npm ci
npm run desktop
```

构建与静态检查：

```sh
npm run check
npm run bundle
```

生成包含安装包、归档、校验值和验收报告的 macOS 发布候选：

```sh
npm run release:macos
```

产物位于 `artifacts/releases/<版本>/<时间>/`，签名与发布流程见[发布指南](docs/release.md)。安装独立 CLI 可执行 `cargo install --path crates/mcplink-core --locked`。

## 数据与认证

连接元数据、配置历史和共享网关保存在本机，托管凭据使用系统凭据库。连接会访问各自配置的上游 MCP 服务。配置备份可能保留原生文件中已有的明文凭据，需要按敏感资料保管。

支持 STDIO 和 Streamable HTTP，以及普通 tools、resources、prompts 请求。Token 和 OAuth 按上游契约配置；具体服务的授权、刷新和业务调用范围分别验收。协议限制见[支持范围与验证](docs/compatibility.md)，已有接入记录见[真实上游验收](docs/real-upstreams.md)。

问题报告请使用合成配置和样例，不要附上真实凭据、数据库或业务正文。安全问题通过 [SECURITY.md](SECURITY.md) 中的渠道反馈。

## 文档与参与

- [架构](docs/architecture.md) · [支持范围与验证](docs/compatibility.md)
- [Agent 接入](docs/agent-discovery.md) · [配置适配](docs/agent-adapters.md)
- [CLI 指南](docs/agent-cli.md) · [真实上游验收](docs/real-upstreams.md)
- [发布指南](docs/release.md) · [当前构建说明](docs/release-notes.md)
- [路线图](ROADMAP.md) · [参与开发](CONTRIBUTING.md) · [Issues](https://github.com/sw897/mcplink/issues)

## 许可

[MIT](LICENSE)。再分发时保留 [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt)，依赖及许可说明见 [docs/DEPENDENCIES.md](docs/DEPENDENCIES.md)。
