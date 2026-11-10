# mcplink · Preview / 预览构建说明

mcplink provides unified access to multiple MCP servers and MCP-to-CLI. Single-service entries, aggregate entries, and tool CLIs reuse the same managed service-account authentication. Native direct connections remain optional.

## Features

- Discover MCP configuration sources, services, and installed agents.
- Create or import connections, inspect tools, and maintain managed authorization.
- Expose selected servers through one aggregate entry, with namespaced tools, resources, prompts, pagination, and upstream failure diagnostics.
- Maintain single-service or optional direct configurations, and explicitly migrate existing shared entries through configuration preview.
- Preview configuration edits, detect concurrent changes, back up files, and restore individual operations.
- Invoke MCP tools or generate command-line entry points using the same managed connections.
- Control background sharing, login startup, pause state, and exit behavior from the menu bar or tray.

Supported clients and operation limits are listed in the [Agent compatibility matrix](https://github.com/sw897/mcplink/blob/main/docs/agent-discovery.md).

## Install and verify

Keep the application or standalone CLI in a permanent location before configuring shared entries. On macOS, copy the app to `/Applications`; on Windows, run the NSIS installer. Complete the client setup as prompted, reload MCP, and make an actual tool call.

Check the source revision, SHA-256 checksums, signing status, and acceptance results supplied with the build. Packaging and signing options are described in the [release guide](https://github.com/sw897/mcplink/blob/main/docs/release.md).

## 中文说明

mcplink 提供多 MCP Server 统一接入与 MCP 转 CLI。单服务入口、聚合入口和工具 CLI 共用同一服务账号的托管认证，原生直连作为可选方式保留。

可以盘点配置来源和服务，新建或导入连接，检查工具，选择直连或共享接入。配置变更支持预览、并发检查、备份与逐条恢复；共享网关通过系统凭据库维护授权和工具筛选。

“统一入口”页保存多个服务的选择，Agent 配置一个 `mcplink` 条目即可使用。聚合入口处理命名空间、分页及上游故障，服务选择更新无需重写原生配置；客户端仍需刷新清单或重载 MCP。迁移单服务条目时可明确选择替换并核对预览。当前数据目录使用数据库语义版本 2 和 management API 18，桌面、CLI 与后台应一起升级。

菜单栏和托盘支持登录启动、暂停共享及退出控制。具体客户端的接入能力见[兼容矩阵](https://github.com/sw897/mcplink/blob/main/docs/agent-discovery.md)。安装后将程序保留在固定位置，按提示完成客户端设置并实际调用工具验证。

安装包的源码版本、校验值、签名状态和验收结果以随附清单为准。使用说明见[项目指南](https://github.com/sw897/mcplink/blob/main/README.zh-CN.md)。
