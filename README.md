# mcplink: Desktop MCP manager and local gateway

[![CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sw897/mcplink/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

English · [简体中文](README.zh-CN.md)

mcplink brings MCP services, account authorization, and AI agent connections into one desktop app. Discover existing configurations, manage the services you use, and connect them to your agents through direct access or a shared local gateway.

Free and MIT-licensed, with no mcplink account required. Upstream services keep their own authentication and access requirements.

[Quick start](#quick-start) · [Install and build](#install-and-build) · [CLI guide](docs/agent-cli.md) · [Documentation](docs/README.md)

## Features

- **Service inventory:** inspect local MCP services, configuration sources, connection status, and available tools.
- **Connections and accounts:** create or import connections, maintain account labels, and manage tokens or OAuth authorization.
- **Agent access:** select services and access modes for each agent, and share a managed connection across clients.
- **Preview and recovery:** review configuration edits, detect concurrent changes, back up files, and restore individual operations.
- **Desktop and CLI:** use the same management core from the desktop, JSON CLI, or bundled management Skill; invoke tools or generate command-line entry points.
- **Background controls:** manage the gateway from the macOS menu bar or Windows tray, with login startup, persistent pause, and exit controls.

## Supported agents

Codex, Hermes, Claude Code, OpenCode, ZCode, DeepSeek Harness, OpenClaw, Kimi Code, WorkBuddy, QwenWork (千问办公), Pi Agent, and Doubao Work (豆包工作).

Configuration scope, supported operations, and client requirements are listed in the [Agent compatibility matrix](docs/agent-discovery.md). Project and profile sources are added explicitly; plugin and managed sources retain their permissions.

## Direct or shared access

| Mode | How it works | Typical use |
| --- | --- | --- |
| Direct | The agent starts or connects to the upstream service using its own configuration. | Let the client run the service and maintain its authorization independently. |
| Shared gateway | A local Bridge uses mcplink's managed connection, credentials, and tool filters. | Reuse a service account across agents and update managed authorization in one place. |

Direct configurations include complete URLs, credentials, and launch arguments. Review where they will be stored or shared before applying them. Shared configurations contain local launcher and connection references; upstream credentials stay in the system credential store.

Each service keeps its own access name. The shared gateway preserves upstream tool names and schemas; clients determine how tools appear in their interfaces.

## Quick start

1. Build and open mcplink. Keep the application or standalone CLI in a permanent location before setting up shared connections.
2. Review discovered services and agents. Add project or profile configuration sources as needed.
3. Import an existing connection or create one with a STDIO command or remote MCP URL. Supply the required authorization and check its tools.
4. Select the target agent and access mode, review the proposed configuration, and complete setup as prompted.
5. Reload the client's MCP configuration and make an actual tool call to confirm access.

Use the service and account views to update managed authorization. Review configuration operations in change history. After moving the executable, regenerate shared entries from its new location.

## Install and build

The desktop uses Tauri 2, React, TypeScript, and Rust. Install Node.js 22 LTS as pinned in [.node-version](.node-version) (minimum 22.12.0), the toolchain in [rust-toolchain.toml](rust-toolchain.toml), Python 3.11+, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).

macOS requires Xcode Command Line Tools. Windows requires MSVC build tools and WebView2.

```sh
git clone https://github.com/sw897/mcplink.git
cd mcplink
npm ci
npm run desktop
```

| Target | Build and installation |
| --- | --- |
| macOS Apple Silicon | `npm run release:macos` creates a DMG and App ZIP; install `mcplink.app` in `/Applications`. |
| Windows x64 | `npm run bundle` creates an NSIS installer in `target/release/bundle/nsis/`. |
| Standalone CLI | `cargo install --path crates/mcplink-core --locked`; add Cargo's executable directory to PATH. |

Run `npm run build` for the frontend and `npm run check` for Rust formatting and static checks. macOS release output goes to `artifacts/releases/<version>/<timestamp>/`.

Default macOS preview packages use ad-hoc signing; Windows installers are unsigned. Signing, notarization, build manifests, and checksums are covered in the [release guide](docs/release.md). Intel Mac and Linux do not have validated installer packages.

## CLI and management Skill

The CLI provides JSON output and stdin input for scripts and agents. Inspect the installed build's commands and connection state first:

```sh
mcplink schema
mcplink status
```

The bundled management Skill uses the same CLI and configuration transactions. See the [CLI guide](docs/agent-cli.md) for installation targets, connection setup, authorization, tool invocation, generated commands, and recovery.

## Data and authorization

Connection metadata and configuration history stay on your computer. Managed credentials use the system credential store. Configuration backups may contain plaintext credentials already present in native files and need the same protection as those files.

mcplink supports STDIO and Streamable HTTP with ordinary tools, resources, and prompts requests. Authentication and tool behavior follow each upstream service's contract. See [supported scope](docs/compatibility.md) for protocol and platform limits, and use synthetic data in issue reports. Security reports follow [SECURITY.md](SECURITY.md).

## Documentation and contributing

- [Documentation index](docs/README.md) · [Agent integration](docs/agent-discovery.md) · [CLI guide](docs/agent-cli.md)
- [Architecture](docs/architecture.md) · [Configuration adapters](docs/agent-adapters.md) · [Compatibility](docs/compatibility.md)
- [Release guide](docs/release.md) · [Roadmap](ROADMAP.md) · [Contributing](CONTRIBUTING.md) · [Issues](https://github.com/sw897/mcplink/issues)

## License

[MIT](LICENSE). Keep [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) when redistributing; see [dependency and license details](docs/DEPENDENCIES.md).
