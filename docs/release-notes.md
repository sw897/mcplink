# mcplink 0.3.0 · Preview / 预览版

mcplink connects multiple MCP servers through one entry per agent and turns tools into commands for terminals, scripts, and automation. This release adds a declarative HTTP API-to-MCP Adapter: ordinary web services can use the same managed connections, aggregate entries, authentication, and tool CLIs.

## HTTP API Adapter

- Register tools with a base URL, JSON Schema inputs, and explicit HTTP operation definitions.
- Map encoded path parameters, query values and JSON request bodies; return JSON, UTF-8 text, PNG/JPEG/WebP images, or mapped Base64 image fields.
- Reuse managed headers, per-service and per-entry tool policies, connection revisions, background controls, and CLI contracts. Raw HTTP APIs use shared entries; they are not exported as native MCP endpoints.
- Discover tools locally without claiming an upstream connection. Actual calls or an explicitly configured probe verify the service. Requests have bounded responses and timeouts, with no redirects or automatic retries.
- Start with the keyless QuickChart and Open-Meteo examples in `examples/http-api/`. Public services retain their own rate limits and terms.

See the [HTTP Adapter guide](https://github.com/sw897/mcplink/blob/v0.3.0/docs/http-api.md) for registration, mappings, calls and limits. OpenAPI import, multipart uploads and asynchronous jobs are not included in this version.

## Upgrade and installation

Upgrade the desktop, standalone CLI and background together. Version 0.3.0 uses **data semantics 5** and **management API 21**; older binaries refuse the newer data directory. Keep the app or CLI at a permanent path, reload the client's MCP configuration, and make a tool call after upgrading. Existing service connections and credentials remain in the same managed store.

Downloads support macOS Apple Silicon and Windows x64. The macOS preview uses **ad-hoc signing and is not notarized**; Windows packages are **unsigned**. This release does not claim Developer ID, Authenticode or SmartScreen trust. Platform manifests record the actual signing flags and source revision; compare downloads with the platform SHA256SUMS files.

The desktop and CLI retain unified MCP access, resources and prompts, per-entry tool ranges, stable pagination, refresh notifications, configuration preview/recovery, and generated commands. Client-specific operations remain documented in the [compatibility matrix](https://github.com/sw897/mcplink/blob/v0.3.0/docs/agent-discovery.md).

Only installers, standalone CLI archives, checksums and public build provenance are distributed. Local configurations, credentials, account-specific acceptance records, screenshots and raw verification reports are excluded. See the [release guide](https://github.com/sw897/mcplink/blob/v0.3.0/docs/release.md) for build and signing procedures.

## 中文说明

0.3.0 增加通用 HTTP API 转 MCP Adapter。用基础 URL、工具参数 Schema 和请求/结果映射注册普通 Webservice，即可通过单服务共享入口、统一入口和工具 CLI 调用；不需要另写 MCP Server。支持路径、query、JSON 请求以及 JSON、文本和图片结果，附 QuickChart、Open-Meteo 无 Key 示例。凭据复用托管请求头，工具发现只读取本地定义；真实核验需要实际调用或显式 probe。

桌面、独立 CLI 和后台应一起升级，当前数据语义为 5、management API 为 21；旧程序拒绝打开新数据目录。升级后重载客户端 MCP 并实际调用。macOS Apple Silicon 预览包采用 ad-hoc 签名、未公证，Windows x64 预览包未签名；实际状态、源码修订与校验值以公开构建清单为准。

公开附件只包含安装包、CLI、校验值和构建来源信息，不包含私有配置、凭据或原始验收材料。使用说明见[中英文 README](https://github.com/sw897/mcplink/blob/main/README.md)。
