# mcplink: Unified MCP access and MCP-to-CLI

[![CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sw897/mcplink/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

English · [简体中文](README.zh-CN.md)

mcplink configures multiple MCP servers in one place, exposes selected services through one MCP entry per agent, and turns MCP tools into commands for terminals, scripts, and automation.

Single-service entries, aggregate entries, and tool CLIs reuse managed authentication: authorize a service account once and use it from multiple agents. Native direct connections remain optional. See the [product definition](docs/product.md) for the behavior and acceptance criteria.

Free and MIT-licensed, with no mcplink account required. Upstream services keep their own authentication and access requirements.

[Quick start](#quick-start) · [Install and build](#install-and-build) · [CLI guide](docs/agent-cli.md) · [Documentation](docs/README.md)

## Core capabilities

| Capability | User goal | Current status |
| --- | --- | --- |
| Unified access to multiple MCP servers | Configure one `mcplink` entry per agent; select and maintain its services in mcplink | Aggregated tools, resources and prompts, namespace routing, pagination, and upstream failure diagnostics |
| MCP-to-CLI | Call MCP tools directly or generate named commands with argument validation, JSON input/output, and automation support | Tool calls and CLI generation for service or aggregate entries, contract checks, and rebinding |

The desktop, authorization management, client adapters, and background process support these two capabilities.

## Supporting features available today

- **Service inventory:** inspect local MCP services, configuration sources, connection status, and available tools.
- **Connections and accounts:** create or import connections, maintain account labels, and manage tokens or OAuth authorization.
- **Agent configuration:** preview single-service, aggregate, or optional direct connections; explicitly replace existing member-service shared entries when migrating.
- **Preview and recovery:** review configuration edits, detect concurrent changes, back up files, and restore individual operations.
- **Management interfaces:** use the same core from the desktop, JSON CLI, or bundled management Skill.
- **Background controls:** manage the gateway from the macOS menu bar or Windows tray, with login startup, persistent pause, and exit controls.

## Supported agents

Codex, Hermes, Claude Code, OpenCode, ZCode, DeepSeek Harness, OpenClaw, Kimi Code, WorkBuddy, QwenWork (千问办公), Pi Agent, and Doubao Work (豆包工作).

Configuration scope, supported operations, and client requirements are listed in the [Agent compatibility matrix](docs/agent-discovery.md). Project and profile sources are added explicitly; plugin and managed sources retain their permissions.

## Agent integration and authentication

Add a service and complete its managed authentication, then choose how to connect it:

| Mode | How it works | Typical use |
| --- | --- | --- |
| Single-service entry | An agent's service entry uses a Bridge to call its managed connection. | Per-service configuration with managed authentication. |
| Aggregate entry | One `mcplink` entry exposes the selected services. | Add or remove services while retaining one client entry. |
| Native direct connection (optional) | The agent launches or connects to the upstream itself. | Client-managed connections and authentication. |

Direct configurations include complete URLs, credentials, and launch arguments. Review where they will be stored or shared before applying them. Shared configurations contain local launcher and connection references; upstream credentials stay in the system credential store.

Create an entry and select services on the desktop's aggregate-entry page, then connect it to an agent. Agents can use different views or share one entry. Selection updates retain the native configuration; refresh the client's tool list or reload MCP as needed. Single-service entries preserve upstream names, while aggregate entries use stable namespaces for tools, prompts, and resource URIs.

Authentication reuse applies to the same managed service connection, account, and authorization scope within one mcplink data directory. Refresh is supported when the upstream permits it; revoked or nonrenewable grants need reauthorization. Native direct connections do not share managed OAuth sessions. Tool CLIs reference the same managed connections without creating another account.

## Quick start

1. Build and open mcplink. Keep the application or standalone CLI in a permanent location before setting up shared connections.
2. Review discovered services and agents. Add project or profile configuration sources as needed.
3. Import an existing connection or create one with a STDIO command or remote MCP URL. Supply the required authorization and check its tools.
4. Create an aggregate entry, select its services, and connect it to the target agent. Alternatively, connect a single service through a managed entry or native direct configuration. Review the proposed changes and complete setup as prompted.
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

## MCP-to-CLI

Call a managed connection's tools directly or generate a named CLI for selected tools. The caller does not need a native MCP connection or the desktop UI. Replace the placeholders with an existing managed connection ID and an actual upstream tool name:

```sh
mcplink connections tools CONNECTION_ID --tool TOOL_NAME
mcplink call CONNECTION_ID TOOL_NAME --stdin
mcplink cli generate CONNECTION_ID --name notes --output ./notes-cli --tool TOOL_NAME
```

`call --stdin` accepts the tool's argument JSON. Generated commands use the local mcplink runtime and managed authorization without embedding credentials. Usage and error boundaries are covered in the [CLI guide](docs/agent-cli.md).

## Management CLI and Skill

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

- [Product definition](docs/product.md) · [Documentation index](docs/README.md) · [Agent integration](docs/agent-discovery.md) · [CLI guide](docs/agent-cli.md)
- [Architecture](docs/architecture.md) · [Configuration adapters](docs/agent-adapters.md) · [Compatibility](docs/compatibility.md)
- [Release guide](docs/release.md) · [Roadmap](ROADMAP.md) · [Contributing](CONTRIBUTING.md) · [Issues](https://github.com/sw897/mcplink/issues)

## License

[MIT](LICENSE). Keep [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) when redistributing; see [dependency and license details](docs/DEPENDENCIES.md).
