# WorkBuddy 实机验证

2026-11-10 核对本机 WorkBuddy 5.9.3，应用包 `WorkBuddy.app`、bundle ID `com.tencent.workbuddy.mac`，完成深度签名验证。测试只使用唯一名称的合成服务；原有账号、推荐连接器与插件不参与测试。

## 配置与策略依据

在该版本签名安装包的 `main/server.js` 和 `main/authority-snapshot.js` 核对到：

- `UserMcpProvider.readServers` 通过 `JSON.parse` 读取用户 `mcp.json`。据此收紧为严格 JSON；旧隔离用例接受注释并不能证明客户端接受 JSONC。
- `parseMcpServerEntry` / `toTransport` 读取 STDIO、HTTP、启动参数、环境、工作目录、请求头和超时等信息。`toTimeoutMs` 将小于 1000 的正数作为秒，其余作为毫秒；初始化与后续操作使用该预算。规范启动和请求超时可等价时才转换；导出使用毫秒值。
- 用户配置的 `disabledTools` 没有进入传输/条目解析结果。实际工具禁用列表由 `McpToolPolicyStore` 从独立的 `mcp-tool-policy.json` 读取，并在会话投影及工具调用时执行。mcplink 不向用户 JSON 写该字段来表示过滤生效；有工具限制的规范连接使用共享模式，在 mcplink 上游执行过滤。
- 首次加载自定义服务需要在原客户端点“信任”。信任、原生启停和工具策略继续由 WorkBuddy 维护；配置写入不等于服务已加载。

官方使用入口见 [WorkBuddy MCP 指南](https://www.workbuddy.ai/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/MCP-Guide)和[连接器字段规范](https://open.workbuddy.cn/docs/connector)。上述实现细节以本次实际安装版本为依据，不能直接推广到所有历史或后续版本。

## 执行范围

通过 mcplink 配置事务写入用户来源，分别验证 STDIO 直连、Streamable HTTP 直连和共享 Bridge；在 WorkBuddy 管理页确认加载，并由真实模型调用合成工具，核对服务端实际收到的 nonce 和返回标记。项目来源另有隔离事务及 Bridge 验收，未据此宣称桌面项目加载已验收。

退出进程后以正常启动参数重新打开应用，三种接入再次完成实际模型调用；重启后的进程未启用远程调试。验收结束撤销六个合成条目，恢复本次创建配置文件原先不存在的状态，停止专用后台与测试服务。重新打开客户端核对原有连接器和插件仍保留。

首次在项目卷及系统凭据库路径验证时出现 STDIO 初始化超时，真实钥匙串调用遇到系统授权等待。随后使用临时目录、独立管理数据和仅含合成值的 fixture 凭据库复核接入链路；不把这部分结果当作真实钥匙串或业务账号授权通过。常规 HTTP 直连另已完成实际模型调用。

用户完成系统授权后，另将合成 HTTP 凭据保存到实际 macOS 系统钥匙串，通过 WorkBuddy 的共享 Bridge 调用受认证保护的合成服务。客户端和 mcplink 后台均重启后，保存的凭据仍可读取，上游检查与真实模型调用再次通过。这次使用正常凭据库，未启用 fixture；共享原生条目不含上游凭据。结束后通过配置恢复及 CLI 删除本次连接和测试凭据，确认原有连接器与插件保留。

原厂源码、应用截图、私有路径和完整验收工件保存在忽略的 `artifacts/e2e/`，不进入公开仓库。每项通过结果及重启、恢复和清理状态以对应报告为准；Windows 厂商桌面、其他版本和真实业务数据调用不在本次实机范围。
