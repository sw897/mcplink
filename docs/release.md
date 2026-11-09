# 构建与发布

使用 `.node-version` 固定 Node.js 22 LTS（最低支持 22.12.0），`rust-toolchain.toml` 固定 Rust 工具链，`npm ci` 和 Cargo `--locked` 固定依赖。开发、CI 和发布入口使用同一 Node 版本。Python 3.11+ 运行验收脚本。锁定环境便于重跑；签名时间和平台工具链会影响产物字节，不宣称二进制逐字节可重现。

## 本地 macOS 候选

```sh
npm ci
npm run release:macos
```

入口按序检查版本、运行 Rust 静态检查、重建 sidecar/桌面/DMG、核对包内 CLI/Skill 和真实架构、从 DMG 安装到隔离目录并验证凭据更新、后台恢复和路径迁移，再运行 CLI 与后台生命周期场景。成功后才生成 `artifacts/releases/<版本>/<时间>/`：

- DMG、保留可执行权限的 app ZIP、含许可声明的 CLI tar.gz。
- `SHA256SUMS`、包含版本/源码哈希/产物哈希的 `release-manifest.json`。
- 从 `docs/release-notes.md` 复制的发布说明，以及汇总签名和隔离验收结论的 `verification.json`。

包校验只在临时副本中去签名、按相同参数重签，再比较 Mach-O 内容，避免签名区域尺寸造成误报，并拒绝旧 sidecar。原始安装包保持签名不变。隔离环境使用合成原生配置和显式 fixture Vault，不能替代新 macOS 账号或真实钥匙串验收。

`bundle` 为 Rust 编译设置路径映射；Windows MSVC 构建还通过 `_CL_` 裁剪原生依赖 `__FILE__` 中的用户目录，并保留已有编译选项。裁剪选项参考 [Windows App SDK 构建设置](https://github.com/microsoft/WindowsAppSDK/blob/main/Directory.Build.props)。包验收检查可执行文件是否仍残留当前构建机的用户目录或工作区路径。开发目录回退仅在 debug 构建中编译。

ZIP/tar 使用规范化的归档元数据，不保留构建者的用户名、UID/GID 或扩展属性。CLI 归档同时包含 MIT 和第三方许可声明；生成后通过原生解包、签名校验和离线 CLI 调用验收。

覆盖安装兼容验收可另行指定旧的 app：

```sh
python3 tests/e2e/installed_app.py --record artifacts/e2e/install-upgrade --previous-app /absolute/path/to/previous/mcplink.app
```

测试只替换自己的临时安装目录，保留隔离数据和原生配置。相同版本的前后构建兼容不代表已验收未来版本的数据迁移。

## Developer ID 与公证

本机钥匙串安装证书后，通过进程环境设置 `APPLE_SIGNING_IDENTITY`。公证支持完整的一组 App Store Connect 变量 `APPLE_API_KEY`、`APPLE_API_ISSUER`、`APPLE_API_KEY_PATH`；或完整的 `APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID`。这些值不写入仓库或命令参数。执行：

```sh
npm run release:macos -- --signed
```

缺少身份或公证配置时，入口在构建前失败。构建后必须通过 Developer ID、app/DMG 的 stapler 和 Gatekeeper 验证，才生成正式签名候选。`--signed` 不降级为 ad-hoc。依据：[Tauri macOS 签名与公证](https://v2.tauri.app/distribute/sign/macos/)、[Tauri 环境变量](https://v2.tauri.app/reference/environment-variables/)。

## GitHub Actions

`ci.yml` 为 macOS Apple Silicon 和 Windows x64 分别运行检查、构建和合成 CLI 场景。macOS 另验收 DMG。Windows 结果须以实际 workflow 报告和桌面实机验收为准；配置工作流本身不表示通过。CI 不运行需要原生 Agent 登录的真实模型测试，也不上传测试 home、Vault 或原始配置。

`release-macos.yml` 仅手动运行，输入必须是与源码版本匹配的已有 tag。仓库需要以下 secrets：

| Secret | 用途 |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 编码的 Developer ID `.p12`，由 Tauri 导入临时钥匙串 |
| `APPLE_CERTIFICATE_PASSWORD` | `.p12` 的导出密码 |
| `APPLE_SIGNING_IDENTITY` | 完整 `Developer ID Application: …` 身份名 |
| `APPLE_API_KEY` / `APPLE_API_ISSUER` | App Store Connect Key ID 与 Issuer |
| `APPLE_API_PRIVATE_KEY` | `.p8` 正文，只落在 runner 临时目录，结束后清理 |

签名、安装包和 CLI 验收都通过后，工作流创建带产物的 **Release 草稿**；正式公开发布从草稿单独执行。工作流固定第三方 Action 的提交 SHA。Runner 平台依据 [GitHub 官方标签表](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job)。

发布前核对待公开的源码、文档和 Git 历史，扫描凭据，避免纳入本机配置、账号验收记录或签名材料。根据两平台 CI 的实际结果记录兼容性；合成测试不能代替 Windows 桌面实机验收。

Windows CI 在打包前用 `scripts/prepare_windows_bundler.py` 通过 GitHub Release API 准备 NSIS 和 Tauri 插件。下载有限重试，缓存资源每次校验固定的 SHA-256，再重建 Tauri 工具目录；校验失败会停止构建。工具版本和目录结构对应锁定的 [Tauri CLI 2.12.4](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs)，升级 CLI 时须同时复核。后续安装包构建及端到端验收照常执行。

## 预览包交付

预览包可按 GitHub Pre-release 交付 macOS Apple Silicon 的 DMG、App ZIP、独立 CLI，以及 Windows x64 NSIS 安装包。签名和公证状态以实际构建的 `verification.json` 为准。

固定源码提交后，在干净的公开源码检出目录构建，并等待同一提交的两平台 CI 完成。Windows 安装包从对应的成功 CI 获取，核对 CLI、桌面程序、许可及编译路径；Tauri 会将桌面 PE 内唯一的 bundle-type 标记从 `UNK` 改为 `NSS`，其余内容应与构建清单一致。

为两平台产物汇总 `release-manifest.json` 和 `SHA256SUMS`，记录相同的源码提交、逐文件哈希、平台验收范围与签名状态。公开清单只保留版本、源码相对路径和产物元数据，不附测试 home、系统凭据库、机器绝对路径或原始日志。

发布说明统一维护在 `docs/release-notes.md`。核对 tag 与源码构建版本一致后，先创建 Release 草稿并上传已验证产物，检查说明及校验值后发布为 Pre-release。发布后重新下载附件核对哈希，并确认 tag 指向、Release 状态和下载入口。签名状态不同的产物应单独标明来源及校验值。
