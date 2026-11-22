# macOS 后台生命周期

当前源码提供菜单栏常驻和当前用户登录自启。关闭主窗口会隐藏窗口并切回仅菜单栏模式，后台继续提供共享 MCP。Dock 不再显示本应用的运行状态；用户固定的 Dock 快捷方式由系统保留。点击菜单栏图标可显示主窗口、暂停/恢复共享、调整登录启动，或选择退出方式。桌面“后台运行”面板提供共享与登录启动设置，并可打开原生菜单。

## 本机构建的签名与钥匙串

钥匙串按程序的签名要求跟踪授权。CLI 固定标识为 `io.mcplink.cli`，桌面为 `io.mcplink.desktop`。公开预览和普通 CI 仍使用 ad-hoc 签名，不能承诺其授权跨不同构建保留。本机长期使用可明确创建一个自签名身份：

```sh
python3 scripts/setup_macos_signing.py --create-local-identity
npm run bundle -- --local-signing --bundles app
```

身份私钥直接导入当前用户钥匙串，生成与传递只经过内存和私有管道；本机 `.local/macos-signing.json` 仅保存公开指纹。后续构建复用同一身份，不重复创建证书，不导出私钥或修改系统信任。自签名仅供本机使用，不等于 Developer ID、公证或可分发的发布者信任；正式发布仍遵守发布指南。固定证书和标识使签名要求稳定，但不能单独保证跨构建的钥匙串授权：macOS 对未归入 Apple 开发者团队的签名仍使用每个构建的 cdhash 作为分区标识。因此自签名版本采用下面的独立固定助手，不把“证书相同”当作验收通过。

### 独立固定凭据助手

首次接入的固定助手安装在原数据目录的 `keychain-helper/mcplink-keychain`；`active.json` 仅含绑定格式版本与助手 SHA256。v1 使用这个原路径；显式迁移后的 v2 使用 `keychain-helper/versions/<SHA256>/mcplink-keychain`，助手通信协议仍为 1。普通升级保留现有绑定字节及助手文件。凭据继续留在原服务、原 namespace 的系统钥匙串中，不导出或重写现有值，不放宽现有 ACL。App 和 CLI 都委托同一个助手，经父子进程私有管道读写；双方核验签名、固定标识及相同证书，核心同时核对助手散列。普通操作始终禁止系统交互；桌面启动、巡检和 Bridge 恢复也不请求口令。绑定损坏、悬空链接、非普通文件或未知版本明确失败，只有真正不存在绑定时才使用原凭据入口。

先准备候选，不接入真实凭据：

```sh
python3 scripts/prepare_macos_keychain.py --record artifacts/e2e/prepared-keychain
mcplink keychain preview --helper /absolute/path/to/prepared-keychain/mcplink-keychain
```

核对助手标识、散列、目标数据目录及逐项授权清单后，才运行 `keychain authorize --helper PATH --preview-hash HASH --confirm`。首次切换需要在系统窗口逐项选择“始终允许”；普通操作不能代替这个明确的授权步骤，`--no-auth-ui` 会拒绝执行它。清单涵盖通信密钥、当前托管定义、OAuth、待恢复事务及已提交后仍待清理的引用；已完成历史仍保留在钥匙串，未来需要恢复某次变更时，在预览和授权两步均显式添加 `--operation OPERATION_ID`。全部项目检查成功后才启用绑定；中断或拒绝不会删除原凭据，尚未启用时继续原来的入口。接入后重新启动 App、后台并重载 Agent MCP。

助手有独立生命周期：普通 App/CLI 构建和覆盖安装不编译、不替换数据目录内的助手。准备入口优先复用已接入的原字节，否则复用本机准备缓存；助手源码或签名身份变化须显式准备 `--new-helper-candidate`，核对并单独迁移，不能当成主程序的自动附带升级。不同设备仍需独立授权。助手散列固定才能维持自签名分区；换助手后仍可能再次需要系统授权。

### 助手升级与失败恢复

只有需要改变助手本身时才准备独立候选；普通主程序升级跳过本节：

```sh
python3 scripts/prepare_macos_keychain.py --new-helper-candidate --record artifacts/e2e/next-keychain
mcplink keychain migrate-preview --helper /absolute/path/to/next-keychain/mcplink-keychain
mcplink keychain migrate --helper /absolute/path/to/next-keychain/mcplink-keychain --preview-hash REVIEWED_HASH --confirm
```

预览不读取凭据值，绑定旧/新散列、签名证书、数据目录、现有恢复记录和逐项授权清单。核对后再提交；`--operation` 若使用须两步一致。候选写入独立的版本目录，使用同目录临时文件、同步落盘及不覆盖发布；全部权限检查成功后才原子切换绑定。拒绝、候选变化或授权失败时原绑定不变，保留完整候选供重新预览后重试。中断前已经批准的候选权限仍留在系统钥匙串，不自动撤销；重试先核对现有权限与剩余项目。`--no-auth-ui migrate` 仅检查候选已有权限，不请求授权，也不返回内部读取的值。助手更换或签名轮换的新增授权须另行明确批准；不能因主程序部署或 skill 同步而自动执行。

切换前先同步保存 `rollback.json`，其中只有旧绑定的原始 JSON、新绑定和预览散列；不含凭据。旧助手及所有已授权文件保持原字节，回退窗口内不要手工移动、重签或删除。恢复使用新的核对步骤：

```sh
mcplink keychain recover-preview
mcplink keychain recover --preview-hash REVIEWED_RECOVERY_HASH --confirm
```

已切换则恢复上一份绑定的原始字节；记录已落盘但尚未切换则取消待提交事务。每次只保留最近一次迁移的回退记录，新的迁移预览会包含其散列。原绑定或原文件被外部改变时拒绝覆盖。首次接入留下的未绑定半文件、以及 `.install-UUID.tmp` 暂存残留，须按恢复清单移入 `recovered/` 隔离保存后再预览重试，不直接覆盖或删除。恢复不重写钥匙串、不撤销既有授权；它也不回退数据库或业务配置。未知损坏状态需核对留存文件，不通过删除绑定回退到旧凭据入口。

签名身份轮换须使用已单独验证且与候选同签名的 CLI 执行迁移，并准备同签名 App/CLI 的部署或旧版本回退路径。切换和回退后均应停止旧后台、重启匹配签名的 App/后台并重载 Agent MCP；使用不匹配的主程序会明确失败。回退后新增凭据可能尚未向旧助手授权，应逐项核对，不能自动放宽 ACL。

原生更新器与本机安装脚本在停后台、写更新收据和替换文件之前，核对包内 App/桌面/CLI 的有效签名、固定角色和助手证书；v2 还要求 App 与 CLI 的已签名元数据声明支持该绑定版本。ad-hoc、其他证书或旧绑定格式的更新会被拒绝，保留当前程序与后台。更新包限制解包范围并拒绝越界路径、链接和特殊文件。安装、迁移与恢复共用排他锁，冲突时等待现有操作结束后重新预览，不并发切换。

桌面通过核心库的内存 Session 访问后台，不为每次请求或巡检创建 CLI。Bridge 的工具请求与目录观察共用自己的 Session。通信凭据按数据目录和已核验后台实例复用，实例更换后重新认证；拒绝访问或后台首次启动失败后暂停自动重试，桌面明确点击“刷新”或重新启动客户端才重试。较慢的启动稍后成功时仍能被发现。已有业务调用不会因通信失败自动重放。

## 覆盖部署验收

部署入口会自动保存不含凭据的配置摘要；只需单独盘点时可运行：

```sh
python3 scripts/verify_macos_deployment.py --snapshot-only --record artifacts/e2e/deploy-before
```

完成候选构建与隔离测试后，可用 `npm run deploy:macos -- --record artifacts/e2e/local-install` 构建本机签名候选并安装；已有候选可直接运行 `python3 scripts/install_macos.py --record artifacts/e2e/local-install`。入口在改动程序前完成签名与配置摘要检查，使用已核验的候选 CLI 检查凭据和停止后台，正常退出桌面，原子替换 App 与固定路径 CLI，并保留旧程序回退副本；不改数据库或服务凭据。对已核对且可结束的 Bridge 可逐项传 `--stop-bridge PID`，不自动终止未知进程。默认以 `--no-auth-ui` 启动 App 和它创建的后台，缺少授权即失败，避免验收自动触发一串密码请求。`--allow-auth-ui` 只放开安装时的读取/启动策略，不迁移助手或自动授予权限；首次授权必须走上面的单独清单流程，授权后仍须冷启动无弹窗验收。

安装后状态明确为“仍待授权及 Agent 重载”，不将安装成功当作全部验收通过。替换文件不更新 Agent 中已运行的进程：必须使用所属 Agent 的原生 MCP 重载，例如 Hermes 的 `/reload-mcp`、Codex 的 MCP 重载；有未完成调用时先结束或核对，不自动重发。不要以修改或重建原生配置、删除凭据处理旧进程。

真实授权完成后，退出 App 并停止已有后台，再以 `open -a /Applications/mcplink.app --args --no-auth-ui` 冷启动验证已有授权。只给新 CLI 增加该选项，不能禁止已经运行的交互式后台弹窗；验收脚本会先核对实际 App 和后台都以该选项启动。然后核对实际界面、关窗常驻、暂停/恢复和后台恢复；最后运行：

```sh
python3 scripts/verify_macos_deployment.py \
  --baseline artifacts/e2e/local-install/before.json \
  --record artifacts/e2e/deploy-after
```

脚本要求至少观察三个十秒巡检周期，使用新 CLI 进程的无弹窗检查，比较配置/绑定/聚合成员与登录偏好，检查 App/CLI/运行中进程及后台身份。它拒绝动态校验失败的旧 Bridge（例如 `-67034`），不以磁盘哈希代替运行中代码检查。真实工具调用与业务成功码另行验收；fixture Vault、成功的签名检查或一次“允许”都不证明授权持久。同一服务名称可能对应运行通信、聚合定义和各成员的多个独立钥匙串项目；切换助手的首次逐项授权与后续两个不同主程序构建复用固定助手应分别记录。观察结果只覆盖实际执行范围。

普通启动、再次打开应用或点击 Dock 图标会显示并聚焦已有主窗口。登录启动传入 `--background`，窗口从创建起隐藏，应用以 `LSUIElement` 和 Accessory 激活策略启动，只保留菜单栏图标；主动显示窗口后切换为 Regular 策略，支持 Dock 再次唤起。单实例检查防止重复创建桌面进程，后台启动已有实例时不抢占前台。

## 登录启动

应用使用 Tauri autostart 插件的 LaunchAgent 方式，作用于当前用户，无需管理员权限。设置读取实际登录项状态，不额外保存偏好。LaunchAgent 使用 `RunAtLoad`，不设置 `KeepAlive`，主动退出后不会被登录项立即拉起。请先将应用放入 `/Applications` 等长期位置，再启用登录启动；移动或重装后可关闭再开启，以更新启动路径。

Finder 和 LaunchAgent 不加载交互 Shell 的初始化文件。上游 STDIO 服务保留继承 PATH 的优先顺序，再补充已有的 `~/.local/bin`、`~/.cargo/bin`、`~/.bun/bin`、Homebrew 和系统目录。服务定义中显式设置的 PATH 优先。版本管理器的其他自定义路径应在连接定义中设置，或使用程序绝对路径。

## 退出与清理

macOS 应用菜单和菜单栏的“退出 mcplink（停止共享）”、Cmd+Q、SIGTERM 和 SIGINT 共用退出流程。桌面先停止巡检和接收新请求，等待已接收请求完成，再停止共享后台。等待超过 30 秒时显示失败并恢复桌面及巡检，可再次通过菜单或信号退出。仅退出界面也等待已接收请求完成，保留共享后台。

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
