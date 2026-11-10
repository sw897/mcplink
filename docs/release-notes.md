# mcplink · Preview / 预览构建说明

mcplink is a desktop MCP manager and local gateway for macOS and Windows. Manage services, account authorization, and AI agent connections from the desktop, JSON CLI, or bundled management Skill.

## Features

- Discover MCP configuration sources, services, and installed agents.
- Create or import connections, inspect tools, and maintain managed authorization.
- Connect services through direct access or a shared gateway with system credential storage and tool filtering.
- Preview configuration edits, detect concurrent changes, back up files, and restore individual operations.
- Invoke MCP tools or generate command-line entry points using the same managed connections.
- Control background sharing, login startup, pause state, and exit behavior from the menu bar or tray.

Supported clients and operation limits are listed in the [Agent compatibility matrix](https://github.com/sw897/mcplink/blob/main/docs/agent-discovery.md).

## Install and verify

Keep the application or standalone CLI in a permanent location before configuring shared entries. On macOS, copy the app to `/Applications`; on Windows, run the NSIS installer. Complete the client setup as prompted, reload MCP, and make an actual tool call.

Check the source revision, SHA-256 checksums, signing status, and acceptance results supplied with the build. Packaging and signing options are described in the [release guide](https://github.com/sw897/mcplink/blob/main/docs/release.md).

## 中文说明

mcplink 是面向 macOS 和 Windows 的桌面 MCP 管理器与本地共享网关。桌面、JSON CLI 和内置管理 Skill 共用连接、账号授权与配置事务。

可以盘点配置来源和服务，新建或导入连接，检查工具，选择直连或共享接入。配置变更支持预览、并发检查、备份与逐条恢复；共享网关通过系统凭据库维护授权和工具筛选。

菜单栏和托盘支持登录启动、暂停共享及退出控制。具体客户端的接入能力见[兼容矩阵](https://github.com/sw897/mcplink/blob/main/docs/agent-discovery.md)。安装后将程序保留在固定位置，按提示完成客户端设置并实际调用工具验证。

安装包的源码版本、校验值、签名状态和验收结果以随附清单为准。使用说明见[项目指南](https://github.com/sw897/mcplink/blob/main/README.zh-CN.md)。
