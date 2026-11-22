# 构建与发布

使用 `.node-version` 固定 Node.js 22 LTS（最低支持 22.12.0），`rust-toolchain.toml` 固定 Rust 工具链，`npm ci` 和 Cargo `--locked` 固定依赖。开发、CI 和发布入口使用同一 Node 版本。Python 3.11+ 运行验收脚本。锁定环境便于重跑；签名时间和平台工具链会影响产物字节，不宣称二进制逐字节可重现。

发布前运行 `python3 scripts/third-party-notices.py --check`。检查许可声明标题与应用版本一致，并逐项比对 Cargo 锁文件及 npm 生产依赖的名称和版本；CI 与发布版本预检均执行该检查。依赖变更后，在安装锁定依赖的环境运行不带 `--check` 的同一脚本重新收集许可正文，再审查差异。

## 本地 macOS 候选

```sh
npm ci
npm run release:macos
```

入口先检查版本并运行 Rust 静态检查，再用独立的 `test-fixtures` DMG 验证隔离安装、凭据更新、后台恢复、路径迁移和 CLI 生命周期。随后默认构建发布 DMG，核对包内 CLI、Skill、架构和系统凭据库边界。全部通过后才生成 `artifacts/releases/<版本>/<时间>/`。

- DMG、保留可执行权限的 app ZIP、含许可声明的 CLI tar.gz。
- `SHA256SUMS`、包含版本/源码哈希/产物哈希的 `release-manifest.json`。
- 从 `docs/release-notes.md` 复制的发布说明，以及汇总签名和隔离验收结论的 `verification.json`。

包校验只在临时副本中去签名、按相同参数重签，再比较 Mach-O 内容，避免签名区域尺寸造成误报，并拒绝旧 sidecar。原始安装包保持签名不变。隔离环境使用合成原生配置和显式 fixture Vault，不能替代新 macOS 账号或真实钥匙串验收。

macOS CLI 内嵌固定 `CFBundleIdentifier=io.mcplink.cli`，避免 Tauri 重新签名时由文件名/Mach-O UUID 生成不同标识。独立 CLI 与 sidecar 使用同一显式签名身份；包验收再次检查最终标识。本机长期自签名构建和真实钥匙串持久授权验收见[macOS 后台](macos-background.md)，该身份不用于公开预览或 Developer ID 发布入口。

`bundle` 为 Rust 编译设置路径映射；Windows MSVC 构建还通过 `_CL_` 裁剪原生依赖 `__FILE__` 中的用户目录，并保留已有编译选项。裁剪选项参考 [Windows App SDK 构建设置](https://github.com/microsoft/WindowsAppSDK/blob/main/Directory.Build.props)。包验收检查可执行文件是否仍残留当前构建机的用户目录或工作区路径。开发目录回退仅在 debug 构建中编译。

ZIP/tar 使用规范化的归档元数据，不保留构建者的用户名、UID/GID 或扩展属性。CLI 归档同时包含 MIT 和第三方许可声明；生成后通过原生解包、签名校验和离线 CLI 调用验收。

覆盖安装兼容验收可另行指定旧的 app：

```sh
python3 tests/e2e/installed_app.py --record artifacts/e2e/install-upgrade --dmg /absolute/path/to/fixture.dmg --previous-app /absolute/path/to/previous/mcplink.app
```

测试先在支持合成凭据库的旧版初始化账号与原生共享配置，再替换同一临时安装目录中的程序，核对新版保留连接 ID、凭据和配置。也可用 `--previous-dmg`，或设置 `MCPLINK_PREVIOUS_DMG`。同版本的前后构建用于验证覆盖兼容。

## Developer ID 与公证

本机钥匙串安装证书后，通过进程环境设置 `APPLE_SIGNING_IDENTITY`。公证支持完整的一组 App Store Connect 变量 `APPLE_API_KEY`、`APPLE_API_ISSUER`、`APPLE_API_KEY_PATH`；或完整的 `APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID`。这些值不写入仓库或命令参数。执行：

```sh
npm run release:macos -- --signed
```

缺少身份或公证配置时，入口在构建前失败。构建后必须通过 Developer ID、app/DMG 的 stapler 和 Gatekeeper 验证，才生成正式签名候选。`--signed` 不降级为 ad-hoc。依据：[Tauri macOS 签名与公证](https://v2.tauri.app/distribute/sign/macos/)、[Tauri 环境变量](https://v2.tauri.app/reference/environment-variables/)。

## Windows 签名候选与安装生命周期

Windows x64 使用 `npm run release:windows` 生成未签名预览候选；`npm run release:windows -- --signed` 要求当前用户证书库中的系统受信任且非自签名的代码签名身份，设置 `WINDOWS_SIGNING_CERTIFICATE_SHA1` 为其准确指纹。私钥可由证书提供程序或硬件维护，不放入参数或源码。可通过 `WINDOWS_SIGNTOOL` 指定 Windows SDK SignTool，`WINDOWS_TIMESTAMP_URL` 指定 RFC 3161 服务，默认使用 DigiCert 时间戳服务。

签名入口拒绝缺失、过期、自签名或不可用身份。独立 CLI 与打包 sidecar 先签名；Tauri 的自定义签名命令签署桌面程序和 NSIS 安装包。所有文件必须通过指定发布者、Authenticode 信任与时间戳检查，安装后再次核对 CLI 和桌面签名；失败不降级为未签名候选。依据：[Tauri Windows 签名](https://v2.tauri.app/distribute/sign/windows/)、[Microsoft SignTool](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool)。CI 的自签名拒绝测试仅验证错误边界，不等于正式签名成功。

`tests/e2e/windows_installer.py --installer FIXTURE_PATH --previous-installer OLD_PATH --record artifacts/e2e/windows-upgrade` 用支持合成凭据库的安装包创建 Token/OAuth 账号及原生绑定，再覆盖安装、实际调用 MCP、重装并卸载。保留的账号与配置用验收 CLI 核对。发布包以 `--production` 单独验证隔离安装和系统凭据库边界，签名候选用 `--require-signature` 核对安装程序、桌面和 CLI 的发布者签名。发布脚本可从 `MCPLINK_PREVIOUS_INSTALLER` 获取支持合成凭据库的旧包。没有旧包时报告当前安装生命周期的结果。

Windows 候选包含 NSIS 安装包、含许可文件的规范化 CLI ZIP、源码及产物哈希、签名和验收摘要。正式签名不保证 SmartScreen 对新发布者或新文件已建立信誉。

### SignPath 开源签名

项目正在准备 SignPath Foundation 申请，政策与审批角色见 [Code signing policy](../CODE_SIGNING.md)。当前 Windows Release 仍未签名；已有 `release-windows.yml` 是本机证书／PFX 路径，不是已接通的 SignPath 工作流。

SignPath 接入须使用其[可信 GitHub 构建流程](https://docs.signpath.io/trusted-build-systems/github)：在 GitHub 托管 runner 构建，将明确选定的文件上传为工作流工件，再按 artifact ID 请求签名。提交身份只允许申请，正式签名由维护者批准；服务凭据使用 CI Secret，项目及策略标识使用配置变量。不得向签名服务上传验收 home、用户配置、凭据库、截图或原始日志。

签名顺序须覆盖独立 CLI、安装包中的 sidecar 和桌面程序，以及 NSIS 安装／卸载程序；最终安装后的文件仍需核验。Tauri 打包会更新桌面 PE 的 bundle-type 标记，不能在签名后修改该内容。产品名和版本资源、允许的文件范围、来源提交和签名返回结果都须校验。当前 CLI 预览包尚缺 Windows 版本资源，必须在正式接入中补齐；NSIS 卸载程序的签名路径也须实际验证，不能只签最终 setup.exe 就宣称完整覆盖。

此接入尚未启用。获批并完成构建、签名、安装验收后，才更新 Release 的签名状态和最终 SHA-256；未获得真实服务结果的检查不算签名成功。

## GitHub Actions

`ci.yml` 为 macOS Apple Silicon 和 Windows x64 分别构建合成验收包与发布包，验收 DMG 或 NSIS 安装，并检查实际发布 CLI 对测试凭据库的拒绝路径。打包前清理该目标目录的历史 bundle。升级验收按近期 main 的成功报告筛选支持合成凭据库的旧包，核对版本，按哈希合并相同副本，优先挑选不同版本并记录来源、提交、版本和哈希。缺少合适旧包时记录当前安装生命周期的覆盖范围。Windows 结果以实际 workflow 和原生桌面验收为准。CI 的 `packages-*` 工件只上传发布包、CLI、校验值与公开构建来源清单。验收包、home、Vault、日志、截图和报告留在运行环境中。

常规 Windows CI 在完整端到端验收前分别报告 NTFS ACL、Credential Manager 迁移和 NSIS 覆盖安装。原生迁移使用 [v0.3.2 的公开 Windows 包](https://github.com/sw897/mcplink/releases/tag/v0.3.2)，按固定修订和 SHA-256 验证 CLI ZIP、NSIS 及构建清单。三项通过后，后续验收复用已核对的候选 NSIS。故障注入只允许在本仓库的 `codex/release-acceptance-*` 分支手动触发，并使用独立的检查名。

`release-macos.yml` 仅手动运行，输入必须是与源码版本匹配的已有 tag。仓库需要以下 secrets：

| Secret | 用途 |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 编码的 Developer ID `.p12`，由 Tauri 导入临时钥匙串 |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` 的导出密码 |
| `APPLE_SIGNING_IDENTITY` | 完整 `Developer ID Application: …` 身份名 |
| `APPLE_API_KEY` / `APPLE_API_ISSUER` | App Store Connect Key ID 与 Issuer |
| `APPLE_API_PRIVATE_KEY` | `.p8` 正文，只落在 runner 临时目录，结束后清理 |

`release-windows.yml` 也仅手动运行，tag 与版本须匹配。需要 `WINDOWS_CERTIFICATE`（PFX 的 Base64）、`WINDOWS_CERTIFICATE_PASSWORD` 和 `WINDOWS_SIGNING_CERTIFICATE_SHA1` secrets；可选仓库变量 `WINDOWS_TIMESTAMP_URL`。PFX 只经 runner 临时文件导入当前用户证书库，结束时清理文件和指定私钥。使用非导出硬件或签名服务时，在持有该身份的 Windows 环境运行本地签名入口，按相同验证标准交付，不把服务机密写进配置。

两个签名发布工作流还要求 `TAURI_SIGNING_PRIVATE_KEY`（Tauri 更新签名私钥文件的完整正文），以及可选的 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。更新签名身份独立于 Apple、Windows 系统代码签名；工作流缺少更新签名密钥时拒绝生成更新附件，不降级为无签名升级。

签名、安装包和 CLI 验收都通过后，工作流创建或补充匹配 tag 的 **Release 草稿**，拒绝覆盖已有同名文件或修改已发布 Release；两平台元数据使用平台前缀。上传入口按安装包白名单、源码修订与 SHA-256 验证文件，仅输出允许的构建来源字段与签名布尔值；`verification.json` 和原始验收材料留在本地或临时 runner，不上传。可用 `--prepare-only --output NEW_DIRECTORY` 先检查公开文件集。正式公开发布从草稿单独执行。工作流固定第三方 Action 的提交 SHA。Runner 平台依据 [GitHub 官方标签表](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job)。

发布前核对待公开的源码、文档和 Git 历史，扫描凭据，避免纳入本机配置、账号验收记录或签名材料。根据两平台 CI 的实际结果记录兼容性；合成测试不能代替 Windows 桌面实机验收。

Windows CI 在打包前用 `scripts/prepare_windows_bundler.py` 通过 GitHub Release API 准备 NSIS 和 Tauri 插件。下载有限重试，缓存资源每次校验固定的 SHA-256，再重建 Tauri 工具目录；校验失败会停止构建。工具版本和目录结构对应锁定的 [Tauri CLI 2.12.4](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs)，升级 CLI 时须同时复核。后续安装包构建及端到端验收照常执行。

## 预览包交付

预览包可按 GitHub Pre-release 交付 macOS Apple Silicon 的 DMG、App ZIP、独立 CLI，以及 Windows x64 NSIS 安装包。本地签名和公证检查来自 `verification.json`；公开附件中的平台构建清单只保留对应签名状态字段，不附原始验收报告。

固定源码提交后，在干净的公开源码检出目录构建，并等待同一提交的两平台 CI 完成。Windows 安装包从对应的成功 CI 获取，核对 CLI、桌面程序、许可及编译路径；Tauri 会将桌面 PE 内唯一的 bundle-type 标记从 `UNK` 改为 `NSS`，其余内容应与构建清单一致。

分支推送和 Pull Request 运行完整 CI，tag 推送不重复执行同一套检查。创建 tag 前确认其源码提交已有成功的 macOS、Windows 和源码审计结果；附件构建清单的修订必须与 tag 解引用后的提交一致。没有对应成功记录时，可用 `workflow_dispatch` 对该引用运行完整验收，不能用其他提交或未完成作业替代。分支与 tag 过滤规则见 [GitHub 工作流语法](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#onpushbranchestagsbranches-ignoretags-ignore)。

为两平台产物汇总 `release-manifest.json` 和 `SHA256SUMS`，记录相同的源码提交、逐文件哈希、平台验收范围与签名状态。公开清单只保留版本、源码相对路径和产物元数据，不附测试 home、系统凭据库、机器绝对路径或原始日志。

发布说明统一维护在 `docs/release-notes.md`。核对 tag 与源码构建版本一致后，先创建 Release 草稿并上传已验证产物，检查说明及校验值后发布为 Pre-release。发布后重新下载附件核对哈希，并确认 tag 指向、Release 状态和下载入口。签名状态不同的产物应单独标明来源及校验值。

## 应用内更新包 / Updater assets

在正常候选验收之外，用 `--with-updates` 生成应用内更新附件。密钥只通过环境传入：`TAURI_SIGNING_PRIVATE_KEY_PATH` 指向本机私钥，或 CI 的 `TAURI_SIGNING_PRIVATE_KEY` 提供正文；密码使用 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。不要把密钥写进源码、命令参数、报告或发布附件。

```sh
npm run release:macos -- --with-updates
npm run release:windows -- --with-updates
# 有系统签名身份时，两项参数可以同时使用：--signed --with-updates
```

源码中的 `src-tauri/updater.pub` 是更新信任根，必须与 `tauri.conf.json` 一致，构建时会检查。维护者须离线备份对应私钥；丢失后，已安装版本无法接受新密钥签出的包，须通过受信任的手动安装迁移。私钥不随软件分发，用户不需要签名身份。

| 平台 | 新增 Release 文件 |
| --- | --- |
| macOS Apple Silicon | `macos-updater.json`、`mcplink_update_VERSION_aarch64.app.tar.gz`、对应 `.sig` |
| Windows x64 | `windows-updater.json`、已验收 NSIS `mcplink_VERSION_x64-setup.exe` 的 `.sig`；复用原安装包 |

`scripts/prepare_updater.py` 从候选文件清单核对原始哈希，再把已验收的 macOS App ZIP 转为规范化 tar.gz，或复用 Windows NSIS。签名必须用 Tauri CLI `signer sign --app-version VERSION` 绑定版本；随后用与桌面相同的 Rust 验证器核验公钥、内容与签名版本，再补入候选清单和校验值。JSON 包含对应平台的 URL、签名、SHA-256 和大小。签名内容的版本、清单版本、Release tag 必须一致。

上传脚本会再次使用源码公钥验证签名，扩展明确的文件白名单，拒绝错误公钥、版本、URL、哈希或混入其他文件。公开平台构建清单的 `signing.updaterSignatureVerified` 记录更新签名核验结果；它与 Developer ID、公证或 Authenticode 字段分别记录。两平台使用独立 JSON，向同一草稿添加附件时不会覆盖对方的更新清单。

普通 CI 不接触发布私钥，仍构建原有预览安装包，并用临时测试身份运行 `tests/e2e/updater.py`。测试密钥和下载夹具只在标记的隔离 home 使用；release 构建没有测试更新源或公钥覆盖能力，上传脚本也不接受测试身份。原生 Windows 应用内升级须另外从已安装旧版实际确认安装、重启与保留数据，不能把 HTTP 契约测试算作安装通过。

The updater uses version-bound Minisign signatures independently of OS publisher signing. Release maintainers retain the private key; only the public trust root is committed. `--with-updates` signs verified platform packages and creates separate platform metadata. Public upload rechecks the signatures, exact version and file whitelist. Standard CI uses ephemeral fixture keys, never the release identity. See [Tauri updater](https://v2.tauri.app/plugin/updater/) for the platform installer format and signing model.
