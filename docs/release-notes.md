# mcplink 0.3.3 · Preview / 预览版

mcplink 将多个 MCP Server 与普通 HTTP API 接到每个 Agent 的一个入口，并把工具转成终端、脚本和自动化命令。本版本加入应用内更新，并完善凭据保护和配置恢复。

mcplink connects MCP servers and ordinary HTTP APIs through one entry per agent and turns tools into terminal, script, and automation commands. This version adds in-app updates and improves credential protection and configuration recovery.

## 改进 / Improvements

- **软件更新**支持检查 GitHub 上的版本、核验与版本绑定的签名并确认安装。首个支持此功能的版本需要手动安装。<br>**Software updates** check GitHub for new versions and verify version-bound signatures before asking for installation confirmation. Install this first updater-enabled version manually.
- **凭据保护**包括 Windows 长凭据的分片存储、macOS 固定钥匙串助手，以及按来源核对的凭据迁移。本地管理连接使用加密会话。<br>**Credential protection** includes chunked storage for large Windows credentials, a fixed macOS Keychain helper, and origin checks during migration. The local management interface uses encrypted sessions.
- **HTTP API 接入**支持导入 OpenAPI 和 Swagger，配置响应成功条件，并过滤工具输出中的托管凭据。<br>**HTTP API integration** imports OpenAPI and Swagger definitions, supports configurable success conditions, and filters managed credentials from tool results.
- **配置和后台恢复**补强配置写入与恢复、OAuth 回调取消和后台退出。Windows 验收覆盖 NTFS ACL、凭据迁移、NSIS 覆盖安装和目录已有写句柄时的 CLI 同步。<br>**Configuration and backend recovery** improve configuration writes and recovery, OAuth callback cancellation, and graceful shutdown. Windows acceptance covers NTFS ACLs, credential migration, NSIS upgrades, and CLI synchronization while a directory is open for writing.

## 升级与下载 / Upgrade and downloads

0.3.3 沿用 **数据语义版本 5**，使用 **management API 24**。先退出旧后台，再一起升级桌面、独立 CLI 和 Bridge。保留已有连接、托管凭据及生成的 CLI 清单。工具契约变化时，核对后再生成到新目录。

Version 0.3.3 retains **data semantics 5** and uses **management API 24**. Stop the old backend before upgrading the desktop, standalone CLI, and Bridge together. Existing connections, managed credentials, and generated CLI manifests are retained. Review changed tool contracts before regenerating into a new directory.

预览构建支持 macOS Apple Silicon 和 Windows x64。macOS 预览包使用临时签名，未经公证。Windows 预览包未签名。平台清单记录源码提交和签名状态。下载后按平台 SHA-256 校验值核对。

Preview builds support macOS Apple Silicon and Windows x64. The macOS preview is ad-hoc signed and has not been notarized. Windows preview packages are unsigned. Platform manifests record the source revision and signing status. Verify downloads against the platform SHA-256 checksums.

[README](https://github.com/sw897/mcplink/blob/v0.3.3/README.md) · [CLI](https://github.com/sw897/mcplink/blob/v0.3.3/docs/agent-cli.md) · [HTTP API](https://github.com/sw897/mcplink/blob/v0.3.3/docs/http-api.md) · [兼容矩阵 / Compatibility](https://github.com/sw897/mcplink/blob/v0.3.3/docs/agent-discovery.md) · [发布流程 / Release process](https://github.com/sw897/mcplink/blob/v0.3.3/docs/release.md)
