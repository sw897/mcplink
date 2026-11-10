# Pi Agent

适配对象是 [Pi 官方项目](https://github.com/earendil-works/pi)的内置 MCP，已核对官方 npm 包 `@earendil-works/pi-coding-agent` 1.5.1（MIT）及其实际解析器、CLI 和运行时。本项目不依赖该包运行；仅在独立验收目录安装固定版本用于验证。旧 `@mariozechner/pi-coding-agent`、第三方 MCP 扩展或禁用内置 MCP 的会话不据此视为已接入。

## 路径和格式

| 来源 | 路径 | 边界 |
| --- | --- | --- |
| 用户 | `$PI_CODING_AGENT_DIR/mcp.json`，默认 `~/.pi/agent/mcp.json` | 环境目录须使用绝对路径或 `~/`；不读取 `auth.json`、`mcp-auth.json` 或会话历史 |
| 项目 | 显式选择的 `<项目>/.pi/mcp.json` | 由用户在 Pi 中信任项目；mcplink 不写 Pi 的信任登记 |

1.5.1 内置解析器使用严格 JSON，允许 UTF-8 BOM，**不接受 JSONC 注释和尾逗号**。不要使用第三方扩展的格式规则解释内置 MCP。

服务表为 `mcpServers`。STDIO 使用 `command`、逐项 `args`、`env` 和绝对 `cwd`；HTTP 使用 `url` 和 `headers`。写入 `type:stdio/http` 及布尔 `enabled`。服务名称只允许英文字母、数字、下划线和连字符；连字符与下划线产生相同命名空间时拒绝变更。

mcplink 提供预览、并发检查、应用、启停、移除和按条目恢复。支持直连和共享 Bridge；应用后在 Pi 运行 `/reload`，用 `pi mcp list --json` 核对加载，并实际调用工具。原生写入只显示待客户端加载。

## 保留与拒绝

- 保留无关条目、根偏好（例如 `autoEnableCodemode`）、BOM、描述及工具暴露设置。目标条目的 `description`、`exposure`、`toolExposure` 纳入计划和指纹，不能在接入或同步时丢失。
- 项目中仅有 `enabled` / `exposure` / `toolExposure` 的条目是用户服务的偏好覆盖，不能当作完整上游导入，不替换成新执行配置。
- `!command` 动态凭据、`${...}` 变量、执行字段中的 `~/` 扩展、相对工作目录、原生账号认证和未验证字段不能等价托管时明确拒绝；不会执行原生凭据命令或复制账号令牌。
- `hidden` / `toolExposure` 限制不能当作无限制上游导入。规范连接上的工具过滤、资源/提示词限制和 OAuth 参数不能等价直连时使用共享模式，由 mcplink 在上游执行限制。
- Pi 的请求超时会随进度通知重置，不能等价于当前托管超时；本版不互相转换，也不伪造启动超时字段。

项目完整条目覆盖同名用户条目；偏好覆盖则继承用户服务。mcplink 只管理显式来源，不推断当前会话的项目覆盖或第三方扩展状态。[Pi 官方 MCP 文档](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/mcp.md)说明信任、工具暴露、认证和替代扩展的行为。

## 验收

`tests/e2e/pi_agent.py` 使用真实 mcplink CLI 检查安装门禁、用户/项目路径、严格 JSON、BOM、共享与直连、暴露设置保留、启停、恢复、冲突和拒绝条件。传 `--pi-package /absolute/package` 时，还运行该固定官方包的 `pi mcp list --json`，并用其配置加载器与 MCP 运行时实际调用合成工具，分别核对项目可信与不可信的结果。

这些调用使用隔离配置和合成认证，不使用真实 Pi 模型账号；MCP 运行时调用通过不等于模型侧业务回合通过。双平台 CI 已加入同一固定版本的验收步骤，每次远端结果以对应提交的 Actions 为准。
