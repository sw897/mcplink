# 隐私说明 / Privacy policy

本说明适用于本仓库发布的 mcplink 桌面、CLI 和本地后台。mcplink 在本机管理服务、认证及 Agent 配置；使用这些本机功能无需注册 mcplink 账号。

This policy covers the mcplink desktop, CLI, and local backend distributed from this repository. mcplink manages services, authentication, and agent configurations locally. Its local features require no mcplink account.

## 本机数据 / Local data

连接摘要、统一入口、绑定和变更记录保存在本机应用数据目录。完整托管定义、Token、OAuth grant 及可能包含凭据的变更计划保存在系统凭据库。原生配置及其本地备份可能含原客户端的明文凭据；原生直连也可能写入上游要求的凭据。共享入口只保存本地程序入口和连接引用。

Connection summaries, unified entries, bindings, and change history are stored in the local application data directory. Complete managed definitions, tokens, OAuth grants, and potentially sensitive change plans are kept in the operating system credential store. Native configurations and local backups may contain credentials from the original client; native direct connections may also contain upstream credentials. Shared entries contain local launcher and connection references.

mcplink 不提供托管云同步，不自动上传这些配置、凭据、历史或业务正文。保留与移除方式见[卸载指南](docs/uninstall.md)；现有条目恢复不等同整库或跨设备凭据恢复。

mcplink provides no hosted cloud synchronization and does not automatically upload these configurations, credentials, history, or business content. See the uninstall guide for retention and removal. Per-entry recovery is not a complete database or cross-device credential backup.

## 网络请求与第三方 / Network requests and third parties

mcplink 不包含项目运营的遥测、广告或后台分析服务。程序只根据用户配置、请求或授权，与所选 MCP 服务、HTTP API、认证服务和本机 Agent 交互；已授权连接可按配置重新连接或刷新 OAuth。工具调用会将所需参数、内容和认证发送给相应服务，STDIO 服务和 Agent 也可能自行访问网络。

mcplink contains no project-operated telemetry, advertising, or analytics service. It interacts with selected MCP services, HTTP APIs, authentication services, and local agents according to user configuration, requests, or authorization. Authorized connections may reconnect or refresh OAuth as configured. Tool calls send necessary arguments, content, and authentication to the selected service. STDIO programs and agents may independently access the network.

上游服务的隐私政策、业务权限和数据保留由相应供应商决定。请在接入前核对所选服务的政策；mcplink 不代表用户同意上游条款，也不代替 Agent 的信任或审批设置。

Upstream providers determine their privacy policies, business permissions, and data retention. Review the policies of services you connect. mcplink does not accept upstream terms for users or replace agent trust and approval settings.

Windows 界面使用 Microsoft Edge WebView2，系统运行时的数据处理受 [Microsoft 隐私声明](https://privacy.microsoft.com/en-us/privacystatement)及系统设置约束；macOS 使用系统 WebKit，相关平台处理见 [Apple 隐私政策](https://www.apple.com/legal/privacy/)。mcplink 自身的字体和前端资源随应用分发，不依赖远程字体或分析脚本。

The Windows interface uses Microsoft Edge WebView2, subject to Microsoft's privacy statement and system settings. macOS uses system WebKit; platform processing is described in Apple's privacy policy. mcplink's own fonts and frontend resources are bundled, without remote font or analytics scripts.

## 发布与用户反馈 / Distribution and feedback

下载页面与问题反馈托管于 GitHub，适用 [GitHub 隐私声明](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement)。若未来启用 SignPath 签名，发布流程会向该服务提交公开源码构建的发布程序和构建来源信息；终端用户的 mcplink 不因此连接 SignPath，也不会向其发送用户的配置、凭据或业务内容。

Downloads and issue reports are hosted on GitHub under its privacy statement. If SignPath signing is enabled, release workflows submit release binaries built from public source and build-origin information. This does not make the end-user application contact SignPath or send it user configurations, credentials, or business content.

反馈由用户主动提交。请勿在公开问题或附件中加入凭据、数据库、原生配置或真实业务正文。普通问题可在仓库 Issues 联系维护者 [@sw897](https://github.com/sw897)；敏感安全问题按 [SECURITY.md](SECURITY.md) 处理。

Feedback is submitted by users. Do not include credentials, databases, native configurations, or real business content in public issues or attachments. Contact the maintainer through repository Issues for ordinary questions, or follow SECURITY.md for sensitive security reports.
