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

升级保留本机数据、系统凭据库、连接 ID 和 Agent 配置。保持应用和 CLI 路径稳定。安装前停止巡检和接收新的桌面请求，最多等待 30 秒让已接收请求完成，再停止共享后台。等待或清理失败时撤销本次安装并恢复当前应用。

Updates preserve local data, system credentials, connection IDs and agent configurations. Keep installed paths stable. Before installation, the desktop stops polling and accepting new requests, waits up to 30 seconds for admitted requests to finish, and then stops the backend. If draining or cleanup fails, the current app resumes and installation can be retried.

应用自带 CLI 与桌面一并替换。更新器检查当前用户的 `~/.cargo/bin`、`~/.local/bin` 及用户目录内的 PATH 位置，只将 SHA-256 与当前应用自带 CLI 一致的普通文件列入确认页，并在新版首次启动时同步。发现阶段只读文件。其他独立 CLI 需手动更新。同步失败可重试。收据与当前安装版本不匹配时，可归档收据并保留恢复所需的备份。Windows 上若 Agent 仍占用应用内程序，先停止相关 MCP 连接，再安装。

The bundled CLI updates with the desktop. Regular files in the user's Cargo/local bin directories or home-scoped PATH qualify for synchronization when their SHA-256 matches the current bundled CLI. The updater reads candidate files and shows the matching paths before confirmation. Other standalone copies require manual updates. Synchronization failures can be retried, and a receipt for a different installed version can be archived while keeping its backups. On Windows, stop agent MCP connections that still hold the app's executables before installing.

Windows 安装程序启动后由其显示安装结果；如果安装取消或失败而应用未重新打开，请手动重开 mcplink，查看升级恢复提示，解除程序占用后重新检查更新。

On Windows, the launched installer reports its own result. If it is cancelled or fails without reopening the app, reopen mcplink to inspect the recovery notice, release any executable locks and check for updates again.

## 验证与网络 / Verification and network

下载前核对仓库、tag、平台、URL 和声明大小。更新包下载到私有暂存文件，上限 256 MiB，连接、空闲和总时长都有明确上限。下载后核对 SHA-256、版本绑定的 Minisign 签名及 macOS 包结构，再交给 Tauri updater 独立核验和安装。macOS 装有固定钥匙串助手时，会额外核对候选应用的签名兼容性。应用使用内置公钥。[Tauri updater](https://v2.tauri.app/plugin/updater/)

Packages must match the repository, tag, platform, URL, size and SHA-256, and carry a version-bound Minisign signature. Downloads use private staging with a 256 MiB size cap and connection, idle and total timeouts. macOS checks the application archive structure and, where a fixed Keychain helper is installed, checks signing compatibility. Tauri independently verifies the package before installation. The app embeds the trusted public key.

更新请求匿名发送到 GitHub API、GitHub Release 下载及其 HTTPS CDN，包含程序版本 User-Agent 和平台所需文件名，不携带 MCP 凭据、配置、业务正文或浏览器 Cookie。更新包签名与 macOS Developer ID／Windows Authenticode 系统签名用途不同；实际系统签名状态仍以公开平台构建清单为准。详见[隐私说明](../PRIVACY.md)及[发布流程](release.md#应用内更新包--updater-assets)。

Update requests are anonymous and send the app's version User-Agent and platform asset names to GitHub and its HTTPS download CDN, without MCP credentials, configuration, business content or browser cookies. Updater signatures are distinct from operating-system publisher signatures; consult the public platform build manifest for the latter.
