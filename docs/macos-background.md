# macOS 后台生命周期

当前源码提供菜单栏常驻和当前用户登录自启。关闭主窗口会隐藏窗口并切回仅菜单栏模式，后台继续提供共享 MCP。Dock 不再显示本应用的运行状态；用户固定的 Dock 快捷方式由系统保留。点击菜单栏图标可显示主窗口、暂停/恢复共享、调整登录启动，或选择退出方式。桌面“后台运行”面板提供共享与登录启动设置，并可打开原生菜单。

## 本机构建的签名与钥匙串

钥匙串按程序的签名要求跟踪授权。CLI 固定标识为 `io.mcplink.cli`，桌面为 `io.mcplink.desktop`。公开预览和普通 CI 仍使用 ad-hoc 签名，不能承诺其授权跨不同构建保留。本机长期使用可明确创建一个自签名身份：

```sh
python3 scripts/setup_macos_signing.py --create-local-identity
npm run bundle -- --local-signing --bundles app
```

身份私钥直接导入当前用户钥匙串，生成与传递只经过内存和私有管道；本机 `.local/macos-signing.json` 仅保存公开指纹。后续构建复用同一身份，不重复创建证书，不导出私钥或修改系统信任。自签名仅供本机使用，不等于 Developer ID、公证或可分发的发布者信任；正式发布仍遵守发布指南。已有配置和服务凭据保留，首次切换身份可能需要在系统窗口逐项授权；希望持续授权时需选择“始终允许”，不放开为所有应用可读。

桌面通过核心库的内存 Session 访问后台，不为每次请求或巡检创建 CLI。Bridge 的工具请求与目录观察共用自己的 Session。通信凭据按数据目录和已核验后台实例复用，实例更换后重新认证；拒绝访问后不循环申请，桌面明确点击“刷新”或重新启动客户端才重试。已有业务调用不会因通信失败自动重放。

## 覆盖部署验收

部署入口会自动保存不含凭据的配置摘要；只需单独盘点时可运行：

```sh
python3 scripts/verify_macos_deployment.py --snapshot-only --record artifacts/e2e/deploy-before
```

完成候选构建与隔离测试后，可用 `npm run deploy:macos -- --record artifacts/e2e/local-install` 构建本机签名候选并安装；已有候选可直接运行 `python3 scripts/install_macos.py --record artifacts/e2e/local-install`。入口在改动程序前完成签名与配置摘要检查，正常退出桌面和后台，原子替换 App 与固定路径 CLI，并保留旧程序回退副本；不改数据库或服务凭据。对已核对且可结束的 Bridge 可逐项传 `--stop-bridge PID`，不自动终止未知进程。

安装后状态明确为“仍待授权及 Agent 重载”，不将安装成功当作全部验收通过。替换文件不更新 Agent 中已运行的进程：必须使用所属 Agent 的原生 MCP 重载，例如 Hermes 的 `/reload-mcp`、Codex 的 MCP 重载；有未完成调用时先结束或核对，不自动重发。不要以修改或重建原生配置、删除凭据处理旧进程。

真实授权完成后，重新启动 App，以 `--no-auth-ui` 冷启动验证已有授权，再核对实际界面、关窗常驻、暂停/恢复和后台恢复；最后运行：

```sh
python3 scripts/verify_macos_deployment.py \
  --baseline artifacts/e2e/local-install/before.json \
  --record artifacts/e2e/deploy-after
```

脚本要求至少观察三个十秒巡检周期，使用新 CLI 进程的无弹窗检查，比较配置/绑定/聚合成员与登录偏好，检查 App/CLI/运行中进程及后台身份。它拒绝动态校验失败的旧 Bridge（例如 `-67034`），不以磁盘哈希代替运行中代码检查。真实工具调用与业务成功码另行验收；fixture Vault、成功的签名检查或一次“允许”都不证明授权持久。观察结果只覆盖实际执行范围。

普通启动、再次打开应用或点击 Dock 图标会显示并聚焦已有主窗口。登录启动传入 `--background`，窗口从创建起隐藏，应用以 `LSUIElement` 和 Accessory 激活策略启动，只保留菜单栏图标；主动显示窗口后切换为 Regular 策略，支持 Dock 再次唤起。单实例检查防止重复创建桌面进程，后台启动已有实例时不抢占前台。

## 登录启动

应用使用 Tauri autostart 插件的 LaunchAgent 方式，作用于当前用户，无需管理员权限。设置读取实际登录项状态，不额外保存偏好。LaunchAgent 使用 `RunAtLoad`，不设置 `KeepAlive`，主动退出后不会被登录项立即拉起。请先将应用放入 `/Applications` 等长期位置，再启用登录启动；移动或重装后可关闭再开启，以更新启动路径。

Finder 和 LaunchAgent 不加载交互 Shell 的初始化文件。上游 STDIO 服务保留继承 PATH 的优先顺序，再补充已有的 `~/.local/bin`、`~/.cargo/bin`、`~/.bun/bin`、Homebrew 和系统目录。服务定义中显式设置的 PATH 优先。版本管理器的其他自定义路径应在连接定义中设置，或使用程序绝对路径。

## 退出与清理

macOS 应用菜单和菜单栏的“退出 mcplink（停止共享）”、Cmd+Q，以及桌面收到的 SIGTERM 会进入清理流程：停止后台巡检，拒绝新的桌面请求，通知共享后台退出，等待清理完成后关闭界面。清理失败时显示错误并保留应用，避免把失败显示为已退出。

后台收到停止请求、Ctrl+C 或 SIGTERM 后会拒绝新调用，取消进行中的握手和请求，关闭上游会话并清理所属进程组，移除属于本实例的 `runtime.json`，最后释放运行锁。上游服务使用独立进程组，初始化被取消或退出超时也会同步发出进程组终止信号。已经派发的工具调用可能已经产生业务结果，通信中断不会自动重试。

“只退出界面（代理继续运行）”停止桌面而保留共享后台。后台使用独立进程组，桌面退出不会连带终止它。后续 Agent 调用也可按需重新启动被正常停止的后台；持续禁用共享应使用“暂停共享”。暂停偏好保存在 SQLite，清理时的临时停用不会覆盖它。直接接入不受共享暂停影响。

强制退出、SIGKILL 和断电不执行应用清理回调。macOS 进程组不会像 Windows Job Object 一样在管理进程被强杀时由内核自动回收，不能把正常退出验收当作强杀保障。

也可使用 `mcplink gateway stop`（management API 11）。它等待当前后台释放运行锁；后台本来未运行时直接成功，不创建数据目录或启动后台。该命令停止共享后台，桌面仍在运行时会按巡检策略恢复后台；需要退出整个应用时使用菜单中的退出命令。

## 验收

macOS 平台专属代码和桌面行为需要在 Mac 上构建和实测；Windows 构建不覆盖这些分支。使用 Node 22、固定 Rust 工具链及 Xcode Command Line Tools：

```sh
npm ci
npm run release:macos
```

发布入口会检查 `.app` 中的 `LSUIElement`、打包内容、安装流程和 CLI，再用应用内的 CLI 运行 `tests/e2e/daemon_lifecycle.py`。测试使用隔离 home、fixture 凭据与合成 MCP 服务，不修改日常 Agent 配置。macOS 场景还验证最小 Finder PATH、独立进程组、SIGTERM 清理，以及 binary plist 应用包的实际可执行文件识别。单独运行：

```sh
MCPLINK_E2E_BINARY=/Applications/mcplink.app/Contents/MacOS/mcplink \
  python3 tests/e2e/daemon_lifecycle.py --record artifacts/e2e/macos-lifecycle
```

实机桌面验收需覆盖：普通启动；关窗后仅保留菜单栏且共享可用；重新显示后恢复 Dock 运行图标；菜单栏暂停恢复及勾选同步；Dock 和重复启动唤起；登录项开关及 `--background` 隐藏启动；两种退出方式；Cmd+Q 和 SIGTERM 清理；暂停后再次登录仍保持暂停。模拟登录项命令不等于已经登出再登录，记录应注明实际范围。
