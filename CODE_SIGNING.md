# 代码签名政策 / Code signing policy

## 当前状态 / Current status

mcplink 正在准备申请 SignPath Foundation 的开源 Windows 签名服务。目前没有宣称获得批准，也没有发布经 SignPath 签名的版本。0.3.2 的 Windows 预览包未签名，macOS 预览包使用 ad-hoc 签名、未公证。每次发布的实际签名状态以对应 Release 和平台构建清单为准。

mcplink is preparing an application for SignPath Foundation's open-source Windows signing service. Approval and SignPath-signed releases are not yet claimed. The Windows 0.3.2 preview is unsigned; the macOS preview is ad-hoc signed and not notarized. Consult each release and its platform build manifest for its actual signing status.

如申请获批并完成接入，Windows 发布将使用以下服务声明：**Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org)**。该服务的证书发布者是 SignPath Foundation；这不代表基金会维护 mcplink 或批准某次 MCP 工具的业务操作。

If approved and integrated, Windows releases will carry this attribution: **Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org)**. The certificate publisher is SignPath Foundation. It does not maintain mcplink or authorize individual MCP tool operations.

## 维护与审批 / Maintenance and approval

| 职责 / Role                                       | 负责人及要求 / Responsibility                                                                                                                                                      |
| ------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 源码维护与外部贡献审查 / Committers and reviewers | [@sw897](https://github.com/sw897)；外部贡献由维护者审查后纳入可信发布源码。<br>External contributions are reviewed by the maintainer before inclusion in trusted release sources. |
| 正式签名批准 / Signing approver                   | [@sw897](https://github.com/sw897)；每次正式签名须由维护者在 SignPath 明确批准。<br>Each production signing request requires explicit maintainer approval in SignPath.             |
| CI 提交者 / CI submitter                          | 专用的提交身份只申请签名、获取结果，不授予批准权限。<br>A dedicated identity submits signing requests and retrieves results without approval permission.                           |

有权修改可信源码、管理签名或批准签名的成员，必须为 GitHub 和 SignPath 账号启用多因素认证。签名服务只有在权限、审批和构建来源约束配置完成后才启用；本文不作为已核验账号设置的证明。

Members who can change trusted source, administer signing, or approve signatures must enable multi-factor authentication for GitHub and SignPath. Signing is enabled only after access controls, approvals, and build-origin restrictions are configured; this policy is not evidence that account settings have already been verified.

## 构建与发布边界 / Build and release boundaries

- 只为 [sw897/mcplink](https://github.com/sw897/mcplink) 公开源码构建的 mcplink 自有程序申请签名。提交来源、版本、工件和文件范围须可核对。<br>Only mcplink-owned programs built from the public repository are submitted. Source revision, version, artifacts, and permitted files must be verifiable.
- 使用 GitHub 托管 runner，先上传工作流工件，再按工件 ID 提交签名；正式密钥由签名服务维护，不放入源码、安装包或普通 CI 日志。<br>Use GitHub-hosted runners, upload workflow artifacts, and submit them by artifact ID. Signing keys remain with the signing service, outside source, packages, and ordinary CI logs.
- 不以 mcplink 的签名身份重签第三方程序。安装包中的第三方组件保留其原有身份和许可；Windows WebView2 等系统运行时遵循原供应商的分发要求。<br>Third-party programs are not re-signed as mcplink. Bundled components retain their identities and licenses; system runtimes such as Windows WebView2 follow their vendor's distribution requirements.
- 下载签名结果后核验文件范围、发布者、签名、时间戳和安装后的程序，再生成最终校验值。失败时停止，不把未签名产物标记为正式签名发布。<br>Verify returned files, publisher, signatures, timestamps, and installed programs before producing final checksums. Failures stop the signed release path.
- 不覆盖已发布版本的同名附件，不改写已有发布标签。预览包和正式签名包分别说明版本与来源。<br>Published assets and release tags are not overwritten. Preview and signed packages identify their respective versions and origins.

## 隐私、卸载与问题报告 / Privacy, removal, and reports

数据处理见[隐私说明 / Privacy policy](PRIVACY.md)，停止后台及移除程序见[卸载指南 / Uninstall guide](docs/uninstall.md)。签名或来源核对有问题时，通过仓库 [Issues](https://github.com/sw897/mcplink/issues) 报告；涉及凭据或漏洞时按 [SECURITY.md](SECURITY.md) 私密报告。

See the privacy policy and uninstall guide for data handling and removal. Report signature or provenance problems in repository Issues; use the private process in SECURITY.md for credentials or vulnerabilities.

SignPath 接入以其[基金会条件](https://signpath.org/terms)和[GitHub 构建来源要求](https://docs.signpath.io/trusted-build-systems/github)为准。构建与发布操作见[发布指南](docs/release.md)。

SignPath integration is subject to its Foundation conditions and GitHub build-origin requirements. See the release guide for build and distribution procedures.
