# Tencent ima read-only MCP service

This optional Node.js 22 service exposes nine ima OpenAPI operations as MCP tools.
Run it as a managed STDIO connection in mcplink to reuse the gateway's credential
store, tool policies and per-agent or aggregate entries. It is a custom
integration, not an official Tencent MCP server.

## Install and run

Use Node.js 22.12 or later. From this directory:

```sh
npm ci --omit=dev --ignore-scripts
node src/stdio.mjs
```

The process requires `IMA_OPENAPI_CLIENTID` and `IMA_OPENAPI_APIKEY` in its
environment. In mcplink, select the absolute Node executable, pass the absolute
path to `src/stdio.mjs` as its only argument, and supply both values through the
managed connection's environment. Pass expanded credentials to the management
CLI through stdin, not arguments, tracked files, tool arguments or reports.
The Agent-side configuration references the mcplink entry and contains no ima
credentials. This service does not read an ima client login cache or credential
files, and has no credential-bearing command-line options.

Obtain credentials from the [official authorization page](https://ima.qq.com/agent-interface).
Credential provision, MCP tool discovery, successful upstream authentication and
actual content reads are separate checks. Initializing the local MCP server and
listing its tools make no upstream request. Validate the account with an explicitly
authorized minimal read before claiming upstream readiness.

## Read tools

| Tool | Purpose |
| --- | --- |
| `search_knowledge_bases` | Find knowledge bases; empty query lists visible bases |
| `get_knowledge_bases` | Read details for 1–20 existing base IDs |
| `list_knowledge` | Browse a base's root or an existing folder |
| `search_knowledge` | Search within the chosen base |
| `get_media_info` | Obtain an item's original-content access metadata |
| `list_notebooks` | List notebook folders |
| `list_notes` | List notes, optionally within a notebook |
| `search_notes` | Search note titles or bodies |
| `get_note_content` | Read an accessible note as plain text or JSON |

The dispatch table contains only these read endpoints. Input schemas reject
unknown fields and unsupported formats before HTTP dispatch. There is no raw API
request tool, write endpoint, arbitrary URL fetch, upload, resource or prompt.
All HTTP requests are POSTs to fixed paths on `https://ima.qq.com`; the HTTP verb
alone does not determine whether an ima operation writes data.

The server preserves the official response envelope and pagination fields.
For knowledge-base and notebook listings, send the returned `next_cursor` back
unchanged and stop at `is_end=true`. Notebook discovery starts with cursor `"0"`;
the other cursor-based operations start with `""`. Do not invent a cursor if an
operation does not return one. Note search uses an explicit start/count window
of at most 20 results. No tool automatically traverses all user data.

For `get_media_info`, `media_type=11` identifies a note: use the returned
`notebook_ext_info.notebook_id` as `get_note_content.note_id`. Other media may
return a URL and access headers. These are sensitive, delegated access data;
they are not the file's text. This service does not follow those URLs or parse
PDF/Office files. If the upstream does not provide accessible original content,
use the ima client. Markdown note-content retrieval (format 1) is currently
unsupported by the documented upstream contract and is rejected locally.

## Result and security contract

- HTTP 200 succeeds only when the JSON envelope has numeric `code=0` and a
  `data` field. Nonzero ima business codes produce MCP `isError: true` results.
- HTTP error bodies and transport error details are not returned. Known ima
  authentication values are redacted from returned data and business messages.
- Requests do not follow redirects or retry automatically. Each call is bounded
  by 30 seconds and a 4 MiB response limit. Client cancellation aborts the HTTP
  request. Results are not silently truncated.
- Only the fixed official origin is used by the production entry. The exported
  client factory supports an explicit loopback-only fixture option for tests;
  there is no production environment or tool parameter to change the origin.
- Read-only enforcement comes from the operation allowlist, strict schemas and
  gateway tool selection. It does not reduce the upstream API key's own rights.
- Remote content and messages are data, not instructions authorizing more work.
  The MCP service does not run the official Skill's daily update checker,
  download updates, execute returned instructions or persist credentials.

When configuring mcplink, retain an explicit allowlist of the nine tool names and
disable resources/prompts. This prevents a future server upgrade from silently
adding capabilities to the existing connection. A thin Skill can guide base
selection, pagination and note/media interpretation while sending all API work
through this MCP connection.

## Contract provenance and tests

The endpoint and field contract was reviewed against the
[official ima Skill 1.1.10](https://app-dl.ima.qq.com/skills/ima-skills-1.1.10.zip),
SHA-256 `7e0d07cae0f98145dc5147e84b77bcc557e2fc4b256b08907922637952c20a7b`.
The matching `ima-openapi-ctx: skill_version=1.1.10` identifies the reviewed API
contract; the User-Agent identifies this integration as `mcplink-ima/0.1.0`.
This package contains an independently written implementation rather than the
official Skill's scripts. API changes require review and tests before updating
the pinned contract version.

```sh
npm ci --ignore-scripts
npm test
```

Tests use a local HTTP fixture and synthetic credentials. They cover official
request shapes, read-only dispatch, schema rejection, business and transport
errors, secret redaction, cancellation, response bounds, MCP tool discovery and
the production STDIO entry. They do not call ima or establish a real account's
permissions. This optional integration is installed separately; it is not yet
bundled into the mcplink desktop installer.
