# 软件更新 / Software updates

桌面侧栏的「软件更新」可从 [GitHub Releases](https://github.com/sw897/mcplink/releases) 检查、下载并安装新版本。打开面板不会自动联网；点击「检查更新」才查询发布列表。默认包含预览版，可取消勾选以仅检查正式版。

The desktop's **Software updates** panel checks GitHub Releases, downloads a platform-specific package and installs it after confirmation. Opening the panel makes no network request. Preview releases are included by default; disable that option to check stable releases only.

## 使用 / Usage

1. 检查更新，核对版本及更新说明。
2. 下载更新包。可以取消，或关闭面板后继续下载；关闭整个程序会丢弃尚未安装的下载，下次须重新下载。
3. 点击「安装并重新启动」，核对中断提醒和将同步的独立 CLI 路径，再确认安装。
4. 应用清理共享后台、安装并重新启动；让 Agent 重新加载 MCP 入口。

Check the version and notes, download the package, then review and confirm installation. Downloads can be cancelled. The app stops its shared backend before installing and restarting. Reload the MCP entry in connected agents afterwards.

只接受本项目已发布 Release 对应的 macOS Apple Silicon / Windows x64 更新包。新版本缺少本平台签名更新文件时，会显示「尚未提供本平台的签名更新包」，可以打开发布页面手动下载；不会把缺文件或网络失败显示成「已是最新」。当前已发布的 0.3.2 没有应用内更新功能或签名更新文件，需要手动安装首个带更新功能的版本。

Only packages for this repository's published macOS Apple Silicon and Windows x64 releases are accepted. Missing updater files are reported separately from an up-to-date result. The existing published 0.3.2 release has neither this updater nor signed updater assets; install the first updater-enabled release manually.

## 数据与 CLI / Data and CLI

升级保留本机数据、系统凭据库、连接 ID 和 Agent 配置。保持应用及 CLI 路径稳定；升级不是配置迁移或备份操作。安装前如有正在执行的管理操作，会等待其结束；无法完成清理时停止安装并恢复当前应用。

Updates preserve local data, system credentials, connection IDs and agent configurations. Keep installed paths stable. An update is not a configuration migration or backup. If the backend cannot stop safely, installation is refused and the current app resumes.

应用自带 CLI 与桌面一并替换。还会检查当前用户的 `~/.cargo/bin`、`~/.local/bin` 及用户目录内的 PATH 位置：只有版本与当前桌面一致、文件普通且未变化的 `mcplink` 才列入确认页，并在新版首次启动时同步。其他位置、符号链接、版本不同或被修改的 CLI 保持原样，需要手动更新；同步失败会在面板中持续显示并允许重试。Windows 上若 Agent 仍占用应用内程序，先停止相关 MCP 连接，再安装。

The bundled CLI updates with the desktop. Recognized, unchanged standalone `mcplink` executables of the current version in the user's Cargo/local bin directories or home-scoped PATH can also be synchronized on first startup. Their paths appear before confirmation. Other locations, symlinks, different versions and edited files require manual updates. Synchronization failures remain visible with a retry action. On Windows, stop agent MCP connections that still hold the app's executables before installing.

Windows 安装程序启动后由其显示安装结果；如果安装取消或失败而应用未重新打开，请手动重开 mcplink，查看升级恢复提示，解除程序占用后重新检查更新。

On Windows, the launched installer reports its own result. If it is cancelled or fails without reopening the app, reopen mcplink to inspect the recovery notice, release any executable locks and check for updates again.

## 验证与网络 / Verification and network

下载前核对仓库、tag、平台、URL 和声明大小，下载后核对 SHA-256 及绑定版本号的 Minisign 签名，再交给 [Tauri updater](https://v2.tauri.app/plugin/updater/) 独立核验和安装。拒绝降级、缺签名及旧签名包冒充新版本。下载上限 256 MiB，发布查询和下载均有超时，可失败后重试。只使用内置公钥，不支持从界面更换来源或签名身份。

Packages must match the repository, tag, platform, URL, size and SHA-256, and carry a version-bound Minisign signature. Tauri independently verifies the package before installation. Downgrades, unsigned packages and replaying an older signed package as a newer version are rejected. Downloads are capped at 256 MiB and operations have timeouts. The app embeds the trusted public key.

更新请求匿名发送到 GitHub API、GitHub Release 下载及其 HTTPS CDN，包含程序版本 User-Agent 和平台所需文件名，不携带 MCP 凭据、配置、业务正文或浏览器 Cookie。更新包签名与 macOS Developer ID／Windows Authenticode 系统签名用途不同；实际系统签名状态仍以公开平台构建清单为准。详见[隐私说明](../PRIVACY.md)及[发布流程](release.md#应用内更新包--updater-assets)。

Update requests are anonymous and send the app's version User-Agent and platform asset names to GitHub and its HTTPS download CDN, without MCP credentials, configuration, business content or browser cookies. Updater signatures are distinct from operating-system publisher signatures; consult the public platform build manifest for the latter.
