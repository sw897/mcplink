# mcplink 0.3.1 · Preview / 预览版

mcplink 将多个 MCP Server 与普通 HTTP API 接到每个 Agent 的一个入口，并把工具转成终端、脚本和自动化命令。本补丁完善契约诊断、批量工具选择及 HTTP API 探测确认，包含自 0.3.0 后的客户端与跨平台验收修复。

mcplink connects MCP servers and ordinary HTTP APIs through one entry per agent and turns tools into terminal, script, and automation commands. This patch improves contract diagnostics, bulk tool selection, and HTTP API probe confirmation, and includes client and cross-platform acceptance fixes since 0.3.0.

## 改进 / Improvements

- **CLI 契约诊断**：区分工具变化、移除或范围排除，以及认证、通信或发现故障导致的未能核验。聚合入口按成员归因，健康工具继续核验。展示参数增删、类型/必填性及 stdin 模式变化；完整旧 Schema 不写入清单。<br>**CLI contract diagnostics:** distinguish changed, removed or filtered tools from contracts left unchecked by authentication, transport, or discovery failures. Attribute aggregate failures to members and keep healthy tools checkable. Summarize parameter and stdin-mode changes without storing full old schemas.
- **搜索结果批量选择**：统一入口工具范围和工具 CLI 可批量选择或取消当前搜索结果，保留隐藏选择；清空全部为独立操作。<br>**Bulk search selection:** select or deselect matching tools in entry ranges and Tool CLI while preserving hidden selections; clearing the entire selection is a separate action.
- **HTTP API 探测确认**：查看工具只读本地定义，真实探测单独确认工具及 HTTP 操作。连接修订变化时拒绝旧确认，每次确认只派发一次。<br>**HTTP API probe confirmation:** inspect local definitions without a request, then confirm the tool and HTTP operation before probing. Reject stale revisions and dispatch each confirmation once.
- **质量与许可**：更新第三方声明版本，CI 与发布预检比对应用版本及锁定依赖。扩充契约、搜索操作、关键客户端原生加载及 Windows 安装窗口验收。<br>**Quality and notices:** align third-party notices with the release and check locked dependency coverage in CI and release preflight. Extend contract, search, native-client, and installed Windows window acceptance.

## 升级与下载 / Upgrade and downloads

桌面、独立 CLI 和后台应一起升级并重启后台。0.3.1 沿用 **数据语义版本 5**，management API 升为 **22**；新契约诊断会拒绝旧后台。保留原有连接和托管凭据，生成的旧 CLI 清单仍可检查；契约确有变化时，核对后生成到新目录。

Upgrade the desktop, standalone CLI, and backend together, then restart the backend. Version 0.3.1 retains **data semantics 5** and raises management API to **22**; new diagnostics reject older backends. Existing connections, managed credentials, and generated manifests are retained. Review changed contracts and regenerate into a new directory.

支持 macOS Apple Silicon 和 Windows x64。macOS 预览包为 **ad-hoc 签名、未公证**，Windows 包 **未签名**。本版本不宣称正式发布者签名或系统信任；实际状态及源码提交见平台构建清单，下载后按平台 SHA256SUMS 核对。

Downloads support macOS Apple Silicon and Windows x64. The macOS preview is **ad-hoc signed and not notarized**; Windows packages are **unsigned**. No production publisher signature or system trust is claimed. Platform manifests record the source revision and signing flags; verify downloads against the platform SHA256SUMS.

公开附件仅包含安装包、独立 CLI、校验值和公开构建来源信息。客户端配置、凭据、截图、业务账号材料及原始验收报告不随包发布。

Public assets contain only installers, standalone CLIs, checksums, and public build provenance. Native configurations, credentials, screenshots, business-account records, and raw acceptance reports are excluded.

使用说明 / Guides: [README](https://github.com/sw897/mcplink/blob/v0.3.1/README.md) · [CLI](https://github.com/sw897/mcplink/blob/v0.3.1/docs/agent-cli.md) · [HTTP API](https://github.com/sw897/mcplink/blob/v0.3.1/docs/http-api.md) · [兼容矩阵 / Compatibility](https://github.com/sw897/mcplink/blob/v0.3.1/docs/agent-discovery.md) · [发布流程 / Release process](https://github.com/sw897/mcplink/blob/v0.3.1/docs/release.md)
