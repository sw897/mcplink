# Windows 后台与共享代理验收

Windows 桌面关闭窗口后收起到系统托盘。左键点击图标显示主窗口，右键菜单提供共享状态、暂停共享、开机启动和退出选项；桌面侧栏“后台运行”也可调整暂停和开机启动，并打开同一份原生托盘菜单，便于图标被系统折叠时使用。

开机启动指当前用户登录 Windows 后启动，不要求管理员权限，也不安装 Windows 系统服务。启动项运行安装目录中的 `mcplink-desktop.exe --background`，窗口从创建起隐藏。重复启动只复用现有桌面实例；普通启动显示窗口，后台启动不会抢占前台。该设置读取实际系统启动项状态，不另存一份容易失去同步的偏好。

托盘每 10 秒检查共享后台；后台退出后按需重新启动。“退出 mcplink（停止共享）”先停止检查、拒绝新的桌面请求，等待后台及其上游服务清理后退出；失败时保留界面并报告错误。“只退出界面（代理继续运行）”则保留后台。后续 Agent 调用仍可按需重新启动已停止的后台；需要持续阻止共享时使用“暂停共享”。暂停/恢复状态写入 SQLite，写入成功后才确认，后台重启后保持原状态。退出清理不改变这份暂停偏好，直接接入的服务独立于共享暂停。

后台、桌面 RPC 助手和上游 STDIO 服务都隐藏控制台。上游进程放入带关闭回收标志的 Windows Job Object，暂停或后台异常退出后清理完整子进程树。已经派发的工具调用失败或超时仍要求核对结果，不自动重发。

运行当前版本需要前后台使用同一构建；轻量 `gateway status` 和持久化暂停需要 management API 10，等待清理完成的 `gateway stop` 需要 API 11。停止命令不启动原本未运行的后台，不创建数据目录或凭据。

Windows Store 版 Codex 通过当前用户已注册的 MSIX 包定位并验证实际可执行文件。托盘和登录启动不依赖终端的 PATH，也不要求枚举受保护的 WindowsApps 目录；隔离测试目录不会借用宿主机安装状态。

## 验收入口

以下测试创建隔离配置、数据目录和合成 MCP 服务，不改动用户当前 Agent 配置。先用 Node 22 构建 `npm run bundle`，将 `MCPLINK_E2E_BINARY` 指向构建或安装目录中的 `mcplink.exe`。Windows 原生进程测试：

```powershell
python tests/e2e/windows_background.py --record artifacts/e2e/windows-background
python tests/e2e/daemon_lifecycle.py --record artifacts/e2e/daemon-lifecycle
```

覆盖暂停/恢复跨强制重启保留、无控制台启动、崩溃后的子进程树回收、已有 Bridge 重连和暂停清理。生命周期测试另外覆盖正常退出、调用或初始化进行中的退出、不响应退出的进程树，以及停止后的再次启动。CI 在 Windows 上运行这些入口。

双 Agent 验收使用本机已经安装的原生 CLI：

```powershell
python tests/e2e/native_shared_agents.py --record artifacts/e2e/native-shared `
  --codex C:/path/to/codex.exe --opencode C:/path/to/opencode-cli.exe
```

Codex 使用本机 `app-server` 的 MCP 工具调用接口；OpenCode 使用本机 v2 服务和默认 Code Mode。OpenCode 的模型响应由固定的本机 Chat Completions 服务提供，只请求合成 MCP 的只读工具，不访问外部模型。验收包含两个客户端并发调用、共用一个已认证上游会话、中央凭据更新后复用新会话和原生客户端暂停恢复。

共享前的连接检查会建立并关闭一个临时会话。验收将该预检与绑定后两个 Agent 共用的上游会话分开计数，并核对后台会话数为 1。

桌面验收另外检查托盘菜单、关闭后留驻、重复启动、启动项开关、`--background` 隐藏启动和后台故障恢复。模拟运行系统登记的启动命令不等于已经重启 Windows；记录中应明确实际验收范围。
