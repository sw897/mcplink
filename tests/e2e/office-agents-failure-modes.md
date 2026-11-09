# 办公客户端接入的失败方式

实现前核对的官方入口（2026-11-09）：[WorkBuddy MCP 路径](https://www.codebuddy.cn/docs/workbuddy/From-Beginner-to-Expert-Guide/Function-Description/MCP-Guide)、[WorkBuddy 字段](https://open.workbuddy.cn/docs/connector)、[千问办公连接器](https://docs.qwenwork.cn/features/connectors)、[豆包工作自定义插件](https://www.doubao.com/work/docs/zh-cn/articles/705018132598-plugins)。

| 失败方式 | 预期及验收入口 |
| --- | --- |
| 安装残留误算已安装 | WorkBuddy 复用程序探测与写入门禁；千问办公/豆包工作可离线生成导入配置，安装状态与生成能力分别显示 |
| 猜测私有数据库或配置路径 | 千问办公仅生成官方导入 JSON；豆包工作生成逐项填写的字段，不创建原生来源，不改客户端存储 |
| WorkBuddy 用户与项目来源混淆 | 使用 `.workbuddy/mcp.json`，项目必须显式登记；预览不写文件，应用/恢复保留无关内容与并发检查 |
| 传输、超时与过滤发生语义变化 | WorkBuddy 使用 `streamableHttp`、毫秒连接超时、`disabledTools`；拒绝 SSE 托管、非整秒毫秒、未知 runtime/模板等不能等价执行的选项 |
| 启停或限制被静默丢弃 | 不虚构 WorkBuddy 的启停字段；不支持的直连选项明确拒绝并提示共享，所有客户端的共享入口保留托管限制 |
| 导入配置被误报为接入成功 | 生成或复制不产生绑定与历史写入，不宣称客户端已加载；UI 提供具体导入步骤 |
| 多服务同名、非法模式及旧后台 | 原子拒绝重复名称或无效请求；API 15 前拒绝新客户端操作，不把新命令发给旧后台 |
| 凭据意外泄露或复制无法使用 | 直连需显式选择且提示含明文；共享只含 Bridge 引用，复制保留路径空格、参数边界及环境变量 |
| 共享连接未验证、断开或暂停 | 生成共享入口前复核 ready/active，暂停时说明调用状态；导出 Bridge 实际调用合成 MCP 工具 |
| 界面只有名单没有操作入口 | 服务页及 Agent 页都可选择新增客户端；千问可复制 JSON，豆包可逐项复制字段，小窗口可滚动且操作按钮可见 |

通过真实 CLI、独立原生格式解析器和 MCP Bridge 做隔离端到端验证，并在浏览器验证构建后的界面。本机未安装三款办公客户端，以上验证不等于原生客户端加载或模型回合验收。
