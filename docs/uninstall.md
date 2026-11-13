# 卸载与数据保留 / Uninstall and data retention

卸载前先在 mcplink 中核对 Agent 接入，预览并移除不再使用的 mcplink 共享绑定，再让相应 Agent 重载 MCP。生成的工具 CLI 也依赖本机 mcplink；移除运行时后这些入口无法继续调用。需要保留的原生直连可继续由原客户端管理。

Before uninstalling, review agent connections in mcplink, preview and remove shared bindings you no longer need, and reload MCP in the affected agents. Generated tool CLIs also depend on the local mcplink runtime and stop working when it is removed. Native direct connections may remain under their original client's management.

若正在执行工具，先核对上游结果。退出或卸载不能撤销已经派发的业务操作。

Check upstream outcomes for in-flight tools. Exiting or uninstalling cannot undo dispatched business operations.

## 停止后台 / Stop the background process

在“后台运行”或菜单栏／托盘菜单关闭登录自启，选择 **退出 mcplink（停止共享）**，等待清理完成。单纯关窗会隐藏界面，“只退出界面”会保留代理；尚未移除的 Agent 共享入口也可能按需重新启动后台。生命周期细节见 [macOS](macos-background.md) 和 [Windows](windows-background.md)。

Disable login startup from Background or the menu-bar/tray menu, then choose **Quit mcplink (stop sharing)** and wait for cleanup. Closing the window hides it; quitting only the interface preserves the proxy. Remaining shared agent entries can restart the backend on demand. See the platform guides for lifecycle details.

## 移除程序 / Remove the application

| 平台 / Platform           | 操作 / Action                                                                                                                                                                                                                                                                                |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows NSIS              | 在系统“设置 → 应用 → 已安装的应用”选择 mcplink 并卸载。<br>Uninstall mcplink from Settings → Apps → Installed apps.                                                                                                                                                                          |
| macOS                     | 退出后，将实际安装位置的 `mcplink.app` 移到废纸篓。<br>After quitting, move the installed mcplink.app to Trash.                                                                                                                                                                              |
| 独立 CLI / Standalone CLI | 移除自行解压安装的 mcplink 可执行文件；若使用 Cargo 安装，运行 `cargo uninstall mcplink-core`。检查 PATH 中是否还有其他副本。<br>Remove the executable you installed from the standalone archive; for a Cargo installation, run `cargo uninstall mcplink-core`. Check PATH for other copies. |

## 数据保留与彻底移除 / Retain or remove data

程序移除与数据删除分别处理。Windows 卸载流程保留本机数据和外部 Agent 配置；移除 macOS App 或独立 CLI 也不会自动删除这些数据。默认管理数据目录为：

Program removal and data deletion are separate. Windows uninstallation retains local data and external agent configurations; removing the macOS application or CLI does not automatically erase them either. Default management data locations are:

- macOS：`~/Library/Application Support/io.mcplink.mcplink`
- Windows：`%LOCALAPPDATA%\mcplink\mcplink\data`

命令行的 `--data-dir` 或 `MCPLINK_DATA_DIR` 可以指定其他目录。先核对实际使用位置，不要删除同名的无关目录。

The CLI's `--data-dir` or `MCPLINK_DATA_DIR` can select another directory. Verify the actual location before removing anything.

如需彻底移除，应在程序和系统凭据库仍可访问时，先解除共享绑定，将服务从统一入口移出，再通过 mcplink 删除不再保留的托管连接；核对没有待处理的凭据清理。需要撤销上游授权时，另在相应供应商处撤销。最后停止并卸载程序，再按自己的备份与保留要求删除数据目录、配置备份及生成的 CLI。

For complete removal, while the program and credential store remain accessible, remove shared bindings, remove services from unified entries, and delete managed connections you no longer want through mcplink. Verify that no credential cleanup remains pending. Revoke upstream grants separately with their providers if needed. Then stop and uninstall the program and remove data directories, configuration backups, and generated CLIs according to your backup and retention needs.

需要保留当前原生直连时，按 [CLI 指南](agent-cli.md)使用 `mcplink connections delete CONNECTION_ID --keep-native` 退出托管。该操作要求先处理共享绑定，保留当前原生文件内容；不要为了卸载而移除仍要使用的原生条目。

To retain native direct configurations, follow the CLI guide and use `mcplink connections delete CONNECTION_ID --keep-native` to leave management. Shared bindings must be handled first; this preserves current native file contents. Keep native entries you intend to continue using.

仅删除 SQLite 或应用数据目录不会可靠地清除系统凭据库中的机密，也不会自动清理外部 Agent 配置。不要通过批量删除其他应用的凭据来清理 mcplink。

Deleting SQLite or the application data directory alone does not reliably erase operating system credential-store secrets or external agent configurations. Do not remove other applications' credentials to clean up mcplink.
