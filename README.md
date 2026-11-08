# mcplink — MCP Manager for AI Agents

[![CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sw897/mcplink/actions/workflows/ci.yml)
[![Version](https://img.shields.io/badge/version-0.1.0%20preview-blue)](https://github.com/sw897/mcplink/releases/tag/v0.1.0)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

**English · [简体中文](README.zh-CN.md)**

mcplink is a local desktop **MCP (Model Context Protocol) manager and gateway** for **Codex** and **Hermes**, with read-only discovery and import from **Claude Code**. Inspect MCP servers, manage credentials, and share connections across AI agents from a Tauri desktop app or a JSON CLI.

本地 MCP 接入管理器：统一查看服务与账号，预览配置变更，并在多个 AI Agent 之间复用连接和托管凭据。

**v0.1.0 is a preview release for macOS Apple Silicon and Windows x64.** macOS packages are ad-hoc signed and not notarized; the Windows installer is unsigned. See [platform support](#platform-support) before installing.

## What you can do

- **Discover existing MCP servers:** inspect native agent configuration and import selected connections into mcplink.
- **Choose direct or shared access:** keep a connection in the agent's configuration or route it through the local MCP gateway. Shared connections continue working after the desktop window closes.
- **Preview configuration changes:** review target files and edits before applying, detect concurrent changes, and restore individual operations from history.
- **Reuse managed credentials:** store secrets in the system credential store and update shared authorization in one place.
- **Use desktop or CLI:** the UI and JSON CLI share the same Rust management core, gateway, and transaction logic.
- **Let agents manage connections:** install the bundled mcplink management Skill for supported agents.

mcplink supports STDIO and Streamable HTTP MCP connections. Authentication and protocol details are documented in [compatibility](docs/compatibility.md) and [architecture](docs/architecture.md).

## Download and install

Get the [v0.1.0 release](https://github.com/sw897/mcplink/releases/tag/v0.1.0), including checksums and a source/artifact manifest.

| Platform | Download |
| --- | --- |
| macOS · Apple Silicon | [DMG installer](https://github.com/sw897/mcplink/releases/download/v0.1.0/mcplink_0.1.0_aarch64.dmg) · [App ZIP](https://github.com/sw897/mcplink/releases/download/v0.1.0/mcplink_0.1.0_aarch64.app.zip) |
| Windows · x64 | [NSIS installer](https://github.com/sw897/mcplink/releases/download/v0.1.0/mcplink_0.1.0_x64-setup.exe) |
| macOS · Apple Silicon · CLI only | [CLI tar.gz](https://github.com/sw897/mcplink/releases/download/v0.1.0/mcplink_cli_0.1.0_aarch64-apple-darwin.tar.gz) |
| SHA-256 checksums | [SHA256SUMS](https://github.com/sw897/mcplink/releases/download/v0.1.0/SHA256SUMS) |

On macOS, copy `mcplink.app` from the DMG to `/Applications` before setting up shared connections. If macOS blocks this preview, review [Apple's guidance for opening an app from an unidentified developer](https://support.apple.com/en-us/102445). On Windows, run the x64 installer; Windows may display an unsigned-publisher warning.

Compare the downloaded file's SHA-256 with its entry in `SHA256SUMS`:

```sh
# macOS
shasum -a 256 mcplink_0.1.0_aarch64.dmg
```

```powershell
# Windows PowerShell
Get-FileHash .\mcplink_0.1.0_x64-setup.exe -Algorithm SHA256
```

Keep the application or standalone CLI in a permanent location. Shared agent configuration uses its absolute path; after moving it, preview and apply the connection again from the new location.

## Get started

1. Open mcplink and review discovered services and configuration sources. Add a real configuration file to include another project or profile.
2. Select a connection and inspect its tools and authentication state.
3. Choose the target agent and direct or shared mode, review the preview, then apply it.
4. Reload MCP or start a new agent session as prompted.

Configuration discovery and a successful file write do not prove that a running agent has loaded the connection.

### Agent support

| Agent | Discover and import | Write configuration |
| --- | --- | --- |
| Codex | User configuration and explicitly added TOML files | Yes |
| Hermes | User configuration and explicitly added YAML files | Yes |
| Claude Code | User configuration and enabled user plugins; explicitly added JSON files | Read-only |

This table describes v0.1.0. Source development adds writable Claude Code user/local/project scopes and version-aware OpenCode V1/V2 JSON/JSONC adapters. These changes are not included in the release downloads; see [adapter scope and validation](docs/agent-adapters.md). Client approvals, policies, and login caches remain under the client’s control.

### CLI and management Skill

Unpack the standalone CLI archive and keep its license files, or use the CLI inside the installed macOS app at `/Applications/mcplink.app/Contents/MacOS/mcplink`. With `mcplink` on your PATH:

```sh
mcplink schema
mcplink status
mcplink skill install --agent codex
mcplink skill install --agent hermes
```

Import an existing service and preview sharing it with another agent:

```sh
mcplink connections import --source codex:user --name notes
mcplink connections probe CONNECTION_ID
mcplink changes preview --connection CONNECTION_ID --target hermes:user --mode shared
mcplink changes apply PLAN_ID
```

Replace `notes` with an existing server name; use the connection and plan IDs returned by the preceding commands. Pass credentials through stdin. The [CLI guide](docs/agent-cli.md) covers JSON input, semantic exit codes, MCP requests, and recovery. `mcplink schema` exposes the command contract.

## Platform support

| Platform | v0.1.0 validation | Signing |
| --- | --- | --- |
| macOS Apple Silicon | Local desktop flow; CI build and 44 isolated core/CLI/source scenarios; DMG and 5 installed-app scenarios | Ad-hoc; no Developer ID or notarization |
| Windows x64 | CI build and 44 isolated core/CLI/source scenarios; installer payload, license, and build-path checks | Unsigned |
| Intel Mac / Linux | No release package or real-device acceptance | — |

Windows desktop interaction, first Keychain access under a new macOS account, and downloaded-app Gatekeeper acceptance remain unverified. Isolated tests use synthetic MCP/OAuth services and fixture credentials; upstream services require their own compatibility checks. The preview has no automatic updater.

## Build from source

mcplink uses **React + TypeScript, Tauri 2, and Rust**. Install the Node.js and Rust versions pinned in [.node-version](.node-version) and [rust-toolchain.toml](rust-toolchain.toml), Python 3.11+, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). macOS requires Xcode Command Line Tools; Windows requires the MSVC build tools and WebView2.

```sh
git clone https://github.com/sw897/mcplink.git
cd mcplink
npm ci
npm run desktop
```

For a standalone CLI built from the checkout:

```sh
cargo install --path crates/mcplink-core --locked
```

Build and verify:

```sh
npm run build
npm run check
npm run bundle
python3 tests/e2e/run.py
python3 tests/e2e/agent_cli.py --record artifacts/e2e/agent-cli/local
python3 tests/e2e/claude_sources.py --record artifacts/e2e/claude/local
```

`npm run check` builds the debug sidecar used by the default tests. Set `MCPLINK_E2E_BINARY` to test another binary. The tests use isolated homes and synthetic services; reports go to the ignored `artifacts/` directory. `npm run release:macos` builds and validates macOS release candidates with checksums and normalized archives. See the [release guide](docs/release.md) for signing and publication.

## Data and credentials

Configuration, management metadata, and the gateway stay on your computer. SQLite stores management metadata; managed secrets use the system credential store. Configuration backups may retain sensitive fields already present in native client files. Connections contact their configured upstream services.

Standard OAuth, manual tokens, and native agent login have separate support boundaries. See [architecture and authentication](docs/architecture.md). Do not include real credentials, configurations, or databases in bug reports; report vulnerabilities through [SECURITY.md](SECURITY.md).

## Documentation and contributing

Detailed guides are currently in Chinese:

- [Compatibility](docs/compatibility.md) · [Architecture](docs/architecture.md) · [CLI guide](docs/agent-cli.md)
- [Agent adapters](docs/agent-adapters.md) · [Real upstream validation](docs/real-upstreams.md)
- [Release guide](docs/release.md) · [v0.1.0 notes](docs/releases/v0.1.0.md)
- [Contributing](CONTRIBUTING.md) · [Roadmap](ROADMAP.md) · [Issues](https://github.com/sw897/mcplink/issues)

## License

[MIT](LICENSE). Keep [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) when redistributing; see [dependency and license details](docs/DEPENDENCIES.md).
