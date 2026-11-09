# mcplink · Preview / 预览构建说明

mcplink is a local desktop MCP manager and shared gateway for Codex, Hermes, Claude Code, OpenCode, ZCode, DeepSeek Harness, OpenClaw, Kimi Code, and WorkBuddy, with connector import generation for QwenWork and Doubao Work.

## Features

- Discover local MCP configurations and connect services to installed agents through native direct access or the shared gateway.
- Maintain managed connections and account authorization, with credentials in the system credential store.
- Preview configuration edits, detect concurrent changes, back up files, and restore individual operations.
- Manage WorkBuddy user/project MCP files, or generate QwenWork JSON and Doubao Work HTTP/STDIO connector fields for import in the client. Generated imports do not confirm client loading or create native bindings.
- Use the desktop, JSON CLI, or bundled management Skill. Invoke tools or generate command-line entry points for selected tools.
- Use the macOS menu bar or Windows tray for background controls, login startup, persistent pause, and exit cleanup.

## Install and verify

Keep the application or standalone CLI in a permanent location before configuring shared entries. On macOS, copy the app to `/Applications`; on Windows, run the NSIS installer. Reload the agent's MCP configuration after applying changes and make an actual tool call.

The files supplied with each build record its source revision, SHA-256 checksums, signing status, and acceptance results. Check `SHA256SUMS`, `release-manifest.json`, and `verification.json` alongside the artifacts. Isolated MCP/OAuth checks use synthetic services; each real upstream has its own authorization and business-call acceptance scope.

See the [project guide](https://github.com/sw897/mcplink/blob/main/README.md), [supported scope](https://github.com/sw897/mcplink/blob/main/docs/compatibility.md), and [release guide](https://github.com/sw897/mcplink/blob/main/docs/release.md).

## 中文说明

mcplink 在一个桌面应用里管理本机 MCP 服务、账号授权和十一类 Agent 的接入。九类支持原生文件管理，包括 WorkBuddy 用户及项目配置；千问办公生成导入 JSON，豆包工作生成自定义连接器字段。生成后需在客户端完成导入并验证工具调用。每项连接可按客户端支持情况选择原生直连或共享网关，共享凭据在系统凭据库中维护。

配置先预览再应用，写入前检查文件变化，支持备份和逐条恢复。桌面与 JSON CLI 共用管理核心，可通过内置管理 Skill 操作连接，也可直接调用工具或生成轻量命令入口。

macOS 菜单栏和 Windows 托盘支持后台运行、登录启动及暂停共享。完整退出等待共享服务清理，仅退出界面可保留代理；暂停状态在重启后保留。

安装和共享接入前，将程序放在固定位置。安装包的源码、校验值、签名状态及验收结果以随附清单为准。使用说明见[中文 README](https://github.com/sw897/mcplink/blob/main/README.zh-CN.md)。
