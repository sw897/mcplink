# 真实上游验收

核对日期：2026-11-08。以下新增能力属于源码开发分支，未包含在 v0.1.0 发布附件中。合成 OAuth、真实账号调用、原生客户端加载分别记录。

| 上游 | 已验证 | 尚未验证 |
| --- | --- | --- |
| GitHub 官方远程 MCP | 手工 Token、官方只读仓库文件、双 Bridge、通用/生成 CLI、托管凭据更新 | 真实 OAuth 的用户同意、刷新和撤销；需要可用的预注册应用 |
| 腾讯会议官方 MCP | 专用 Token、只读会议列表、双 Bridge、通用/生成 CLI、托管凭据更新、无效值后恢复、后台重启 | 会议详情/纪要、其他业务工具、真实 Token 重置或账号切换；此入口不计为 OAuth 刷新验收 |
| 飞书 | 官方接入契约已登记 | 真实应用、账号权限、用户授权及刷新 |
| 腾讯 IMA | 官方开放入口已登记 | 官方 MCP 契约及测试知识库；不把第三方封装标为官方或预设 OAuth |

## 腾讯会议

来源是[腾讯会议官方文档](https://meeting.tencent.com/support/articles/14/index.html)和[官方 Skill 页面](https://meeting.tencent.com/ai-skill.html)。当次官方包为 `v1.1.0`，服务端自报 `open_platform_mcp` / `1.0.0`，实际协商协议为 `2025-03-26`。官方包下载地址为 [tencent-meeting-mcp.zip](https://updatecdn.meeting.qq.com/openmeeting/meetingskill/tencent-meeting-mcp.zip)，当次 SHA-256：

```text
f0c3583f646f3d4e6188bbf301bbc90eadb14a98e37fda966a95143dfe2d9001
```

包内脚本是一次性 JSON-RPC 命令代理，不是持续运行的 STDIO MCP 服务。其配置声明的云端地址可由 mcplink 通过 Streamable HTTP 直接接入，已验证完整初始化、通知、发现和只读调用：

```json
{
  "name": "tencent_meeting_readonly",
  "definition": {
    "url": "https://mcp.meeting.tencent.com/mcp/wemeet-open/v1",
    "headers": {"X-Skill-Version": "v1.1.0"},
    "includeTools": ["get_user_meetings"]
  }
}
```

通过 `connections save --stdin` 创建连接，再用 `credentials update ID --stdin` 设置 `X-Tencent-Meeting-Token` 头。凭据对象为 `{"header":"X-Tencent-Meeting-Token","value":"<通过本机安全输入提供的 Token>"}`；真实值只经 stdin 进入系统凭据库，不放入命令参数、配置样例或生成 CLI。先从官方页面核对当前包和版本要求，再配置请求头。

该入口使用长期专用 Token。托管凭据更新后，两个已建立的 Bridge、已有生成 CLI 和后台重启后的调用均已恢复，无须重写原生配置。验收只使用允许列表中的 `get_user_meetings`，被过滤的写工具在本地派发前拒绝。未重置或撤销用户现有 Token，也未执行会议写操作。

### MCP 成功与业务成功

该上游在 MCP `content[].text` 中返回 JSON，包含业务 HTTP 的 `status_code`、`headers` 和字符串 `body`。日期及追踪头会变化，对照验收比较解析后的业务正文，不能直接比较整个响应字符串。

真实无效 Token 对照返回了业务 `status_code: 500`，但 MCP `isError` 为 `false`，所以通用 `call` 保留上游结果并退出 `0`。自动化须另行检查包装中的 HTTP 状态及解析后 `body` 的业务错误码；HTTP/MCP 成功本身不能作为业务断言。mcplink 不按某个服务的正文结构推断所有工具是否成功，也不因此自动重发请求。

当次有效凭据的包装状态为 `200`，已读到会议列表，且通用 CLI、两个 Bridge 和生成 CLI 的业务正文摘要一致。临时托管连接、系统凭据及后台已清理；验收工件只保存状态、计数和摘要，不包含 Token 或会议正文。

## 后续验收

真实 OAuth 仍需所选上游提供 MCP 可用的授权入口、注册应用或登录同意；专用 Token 更换不能代替刷新令牌轮转。继续按[路线图](../ROADMAP.md)核对飞书与 IMA 的实际认证归属，再进行只读调用、更新/刷新、撤销和权限隔离。实施前使用[失败方式清单](../tests/e2e/real-upstreams-failure-modes.md)，未执行项保留为待验收。
