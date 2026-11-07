# mcplink

一个本地桌面 MCP 接入管理器。在同一处查看服务、维护托管凭据，并把已有连接接给常用的 AI Agent。

**当前状态：0.1.0 预览版，macOS Apple Silicon 优先。** Windows 构建与兼容性验证正在推进；本地 macOS 包使用 ad-hoc 签名，尚未完成 Developer ID 签名和公证。

## 能做什么

- 盘点 Codex、Hermes 的原生 MCP 配置，以及 Claude Code 用户和已启用用户插件的只读来源。
- 将已有服务接给另一个 Agent，按服务选择直连或共享本地 Gateway。
- 在应用前预览配置变更，检查并发修改，备份并逐条恢复操作。
- 用系统凭据库保存托管凭据，通过共享连接复用授权。
- 从桌面或 JSON CLI 完成同一组管理操作。
- 安装内置管理 Skill，让 Agent 通过 CLI 管理连接。

Claude Code 来源当前只读；配置写入支持 Codex、Hermes。业务 Skill 的编写和版本分发由用户自行管理。完整边界见[兼容范围](docs/compatibility.md)。

## 从源码运行

开发需要 Node.js、Rust、Python 3.11+，以及目标平台的原生构建工具。Node/Rust 的构建版本由 [.node-version](.node-version) 和 [rust-toolchain.toml](rust-toolchain.toml) 固定。macOS 需要 Xcode Command Line Tools。

```sh
npm ci
npm run desktop
```

`desktop` 会构建 Rust 核心和 sidecar，再启动 Tauri。

## 使用流程

1. 打开应用，核对扫描到的服务及配置来源。其他项目或 profile 可通过“配置来源”添加真实配置文件。
2. 选中连接并检查工具。配置扫描本身不代表认证有效或运行中的 Agent 已加载。
3. 选择目标 Agent、直连或共享模式，核对预览后应用。
4. 按结果提示重新加载 MCP 或开启新的 Agent 会话。

直连适用于本地工具或自行管理授权的服务。共享模式通过本机 Gateway 复用 mcplink 托管的连接；关闭桌面窗口后，已接入的共享服务仍可调用。应用移动后，需要重新预览并更新共享入口路径。

## CLI 与管理 Skill

独立 CLI 可以从源码安装，也可直接使用 macOS app 内的 `Contents/MacOS/mcplink`：

```sh
cargo install --path crates/mcplink-core --locked
mcplink schema
mcplink status
mcplink skill install --agent codex
mcplink skill install --agent hermes
```

例如，将已有服务纳入管理后，预览接给另一个 Agent：

```sh
mcplink connections import --source codex:user --name notes
mcplink connections probe CONNECTION_ID
mcplink changes preview --connection CONNECTION_ID --target hermes:user --mode shared
mcplink changes apply PLAN_ID
```

`CONNECTION_ID` 和 `PLAN_ID` 使用前一步返回的值。凭据通过标准输入传入。命令、输入格式、退出码和恢复流程见 [CLI 指南](docs/agent-cli.md)。

## 构建与验证

```sh
npm run build
npm run check
npm run bundle
```

macOS app 和 DMG 位于 `target/release/bundle/`。生成带验收结果、发布说明与 SHA256 清单的候选包：

```sh
npm run release:macos
```

无账号的隔离端到端验证：

```sh
python3 tests/e2e/run.py
python3 tests/e2e/agent_cli.py --record artifacts/e2e/agent-cli/local
python3 tests/e2e/claude_sources.py --record artifacts/e2e/claude/local
```

先构建核心程序；这些测试默认使用 `target/debug/mcplink`，可用 `MCPLINK_E2E_BINARY` 指向其他构建。测试使用合成配置、MCP/OAuth 上游和 fixture Vault，不操作日常 Agent 配置。记录输出到忽略的 `artifacts/`。

## 数据与认证

配置、管理数据和后台保存在本机。原生配置反映客户端现状，SQLite 保存管理元数据，托管 Secret 保存在系统凭据库。配置备份可能包含原客户端已有的敏感字段，应按本地凭据管理。

标准 OAuth、手工 Token 和原生客户端登录由各自的维护方处理。支持一种认证流程并不代表任意上游服务均已兼容，详见[架构与认证](docs/architecture.md)。

## 项目文档

- [兼容范围与限制](docs/compatibility.md)
- [架构](docs/architecture.md)
- [CLI 与管理 Skill](docs/agent-cli.md)
- [构建、签名与发布](docs/release.md)
- [开发与贡献](CONTRIBUTING.md)
- [路线图](ROADMAP.md)
- [安全问题](SECURITY.md)

## 许可

mcplink 使用 [MIT License](LICENSE)。分发时请保留 [第三方许可声明](THIRD_PARTY_NOTICES.txt)；依赖信息见[依赖与许可](docs/DEPENDENCIES.md)。
