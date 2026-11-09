# mcplink: Desktop MCP manager and local gateway

[![CI](https://github.com/sw897/mcplink/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/sw897/mcplink/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

English · [简体中文](README.zh-CN.md)

mcplink is a desktop MCP (Model Context Protocol) manager for people who use multiple AI agents. Discover configured MCP servers, manage connections and account authorization, and connect services to the agents that need them. It supports native direct connections and shared access through a local gateway, subject to each client's configuration support.

The desktop app is free and MIT-licensed. Local management requires no mcplink account; upstream MCP services keep their own authentication and authorization requirements.

[Quick start](#quick-start) · [Install and run](#install-and-run) · [Build from source](#build-from-source) · [CLI guide](docs/agent-cli.md)

## Features

| Feature                     | What you can do                                                                                                                                                      |
| --------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Local inventory             | View existing MCP servers, configuration sources, connected agents, and connection status. Add project or profile configurations explicitly.                         |
| Connections and accounts    | Import or create connections, maintain account labels, tokens, and OAuth authorization, and inspect available tools.                                                 |
| Access across agents        | Choose services and connection modes for each client, and connect multiple agents to the same managed connection.                                                    |
| Preview and recovery        | Review target files and edits before writing, detect concurrent changes, back up configuration, and restore individual operations.                                   |
| Tool CLI                    | Manage connections through a JSON CLI, validate tool arguments against their schemas, invoke tools, and generate small command-line entry points for selected tools. |
| Desktop background controls | Use the macOS menu bar or Windows tray for gateway status, login startup, and pause controls. Choose whether quitting also stops shared services.                    |

The desktop, CLI, and bundled management Skill use the same managed data and configuration workflow. Updating credentials in mcplink lets existing shared agent entries keep using the same connection.

## Direct or shared access

| Mode                 | How it works                                                                                                      | When to use it                                                                                   |
| -------------------- | ----------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Native direct access | mcplink writes the client's native configuration. The agent starts or connects to the upstream MCP server itself. | You want the client to run the service and manage its native authentication independently.       |
| Shared gateway       | The agent uses a local Bridge to access mcplink's managed connection, credentials, and tool filters.              | Multiple agents need the same service account, or you want to update authorization in one place. |

Shared entries store only the local launcher and connection references in native configuration. Upstream credentials stay in the system credential store. Each service keeps its own entry name; the shared gateway preserves upstream tool names and schemas. Clients decide how those names appear in their own interfaces.

Closing the window can leave the app in the menu bar or tray. The gateway keeps its paused state across restarts. Full quit waits for shared-service cleanup; quitting only the interface keeps the proxy running. Later agent requests can also start the backend on demand. See the [macOS](docs/macos-background.md) and [Windows](docs/windows-background.md) background guides.

## Supported agents

mcplink supports Codex, Hermes, Claude Code, OpenCode, ZCode, DeepSeek Harness, OpenClaw, and Kimi Code.

mcplink detects client installation and discovers supported user configurations. Uninstalled clients stay visible; install the client and refresh to enable configuration. Add project and other profile sources explicitly. Plugin, managed, and other read-only sources retain their permissions.

See [eight-Agent integration](docs/agent-discovery.md) and [configuration adapters](docs/agent-adapters.md) for native formats, writable sources, and enable/disable behavior.

## Quick start

1. [Build mcplink from source](#build-from-source) and keep the application or standalone CLI in a permanent location. Shared configurations reference its executable path.
2. Open the app and review local services and installed agents. Add real project or profile configuration files as needed.
3. Import an existing connection, or create one with a STDIO command or remote MCP URL. Supply credentials or complete OAuth authorization, then inspect its tools.
4. Select the target agent and direct or shared mode, review the configuration preview, and apply it.
5. Reload the agent's MCP configuration as prompted and make an actual tool call to confirm access.

Use the account and service views to update managed authorization, and the change history to inspect or restore operations. After moving the executable, preview and apply shared configuration again from the new location.

## Install and run

[Build from source](#build-from-source) to use the desktop app or standalone CLI:

| Platform            | Installation                                                                                                            |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| macOS Apple Silicon | Run `npm run release:macos` to generate a DMG and App ZIP, then install `mcplink.app` in `/Applications`.               |
| Windows x64         | Run `npm run bundle` to generate an NSIS installer, then run the installer.                                             |
| Standalone CLI      | Run `cargo install --path crates/mcplink-core --locked` from the checkout and add Cargo's executable directory to PATH. |

macOS release candidates go to `artifacts/releases/<version>/<timestamp>/`; Windows installers go to `target/release/bundle/nsis/`. Check the checksums and source manifest supplied with each build.

Default macOS preview builds use ad-hoc signing; Windows installers are unsigned. See the [release guide](docs/release.md) for signing and notarization. Intel Mac and Linux have no validated installer packages.

## CLI and management Skill

The CLI provides JSON output and stdin input for scripts and agent automation. With `mcplink` on your PATH, inspect the current build's command contract and connection state:

```sh
mcplink schema
mcplink status
```

For example, connect a service named `notes` from Codex to Hermes:

```sh
mcplink connections import --source codex:user --name notes
mcplink connections probe CONNECTION_ID
mcplink changes preview --connection CONNECTION_ID --target hermes:user --mode shared
mcplink changes apply PLAN_ID
```

Confirm source IDs with `status`, replace `notes` with an existing service name, and use the connection and plan IDs returned by the preceding commands. Pass credentials through stdin.

Install the bundled management Skill for Codex, Hermes, Claude Code, or OpenCode to let an agent use the same CLI workflow:

```sh
mcplink skill install --agent codex
```

The [CLI guide](docs/agent-cli.md) covers tool invocation, generated command-line entry points, account authorization, and operation recovery.

## Build from source

mcplink uses Tauri 2, Rust, React, and TypeScript. Development and CI use Node.js 22 LTS, pinned in [.node-version](.node-version), with a minimum of 22.12.0. Rust is pinned in [rust-toolchain.toml](rust-toolchain.toml). Install Python 3.11+ and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/) as well.

macOS requires Xcode Command Line Tools; Windows requires the MSVC build tools and WebView2.

```sh
git clone https://github.com/sw897/mcplink.git
cd mcplink
npm ci
npm run desktop
```

Build and run static checks:

```sh
npm run check
npm run bundle
```

Generate a macOS release candidate with installers, archives, checksums, and acceptance reports:

```sh
npm run release:macos
```

Output goes to `artifacts/releases/<version>/<timestamp>/`. See the [release guide](docs/release.md) for signing and publication. To install the standalone CLI from the checkout, run `cargo install --path crates/mcplink-core --locked`.

## Data and authorization

Connection metadata, configuration history, and the gateway stay on your computer. Managed credentials use the system credential store. Connections contact their configured upstream MCP services. Configuration backups may retain plaintext credentials already present in native files; treat those backups as sensitive data.

mcplink supports STDIO and Streamable HTTP, with ordinary tools, resources, and prompts requests. Configure tokens and OAuth according to the upstream service's contract. Authorization, refresh, and business tool calls are validated separately for each service. See [supported scope](docs/compatibility.md) for protocol limits and [real upstream validation](docs/real-upstreams.md) for recorded integrations.

Use synthetic configurations and examples in issue reports. Keep real credentials, databases, and business content out of them. Report security issues through the channels in [SECURITY.md](SECURITY.md).

## Documentation and contributing

Detailed guides are currently in Chinese:

- [Architecture](docs/architecture.md) · [Supported scope](docs/compatibility.md)
- [Eight-Agent integration](docs/agent-discovery.md) · [Configuration adapters](docs/agent-adapters.md)
- [CLI guide](docs/agent-cli.md) · [Real upstream validation](docs/real-upstreams.md)
- [Release guide](docs/release.md) · [Current build notes](docs/release-notes.md)
- [Roadmap](ROADMAP.md) · [Contributing](CONTRIBUTING.md) · [Issues](https://github.com/sw897/mcplink/issues)

## License

[MIT](LICENSE). Keep [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt) when redistributing; see [dependency and license details](docs/DEPENDENCIES.md).
