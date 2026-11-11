# HTTP API → MCP Adapter

Register an ordinary HTTP API as a managed service. mcplink exposes its configured operations as MCP tools and uses the same member connection ID for single-service entries, aggregate entries, direct tool calls, and generated CLIs. Upstream services do not need an MCP implementation.

## Register and call

From a source checkout:

```sh
mcplink schema
mcplink connections save --stdin < examples/http-api/quickchart.json
mcplink connections save --stdin < examples/http-api/open-meteo.json
```

Record the returned connection IDs. `connections tools ID` discovers the configured tools without contacting the API. `connections probe ID` runs the explicitly configured probe, if present. These sample probes render a small chart and fetch weather for a public city coordinate.

```sh
mcplink connections tools OPEN_METEO_ID
mcplink connections probe OPEN_METEO_ID
mcplink call OPEN_METEO_ID get_weather --stdin <<'JSON'
{"latitude":31.23,"longitude":121.47,"timezone":"Asia/Shanghai"}
JSON

mcplink call QUICKCHART_ID render_chart --stdin <<'JSON'
{"chart":{"type":"bar","data":{"labels":["A","B"],"datasets":[{"label":"Example","data":[3,7]}]}},"width":320,"height":200}
JSON
```

The weather result contains text JSON and `structuredContent`; the chart result contains an MCP `image` block with `mimeType` and Base64 `data`, plus byte count and SHA-256 in `structuredContent`. Tool CLI JSON preserves those content types. To save a chart from CLI output, read `ok.output.content[0].data`, Base64-decode it, and write a PNG. Base64 is protocol image data, not a text description of the image.

## Unified entry and generated CLI

Select both saved connections in the desktop's aggregate-entry page, or save an entry with `gateway save --stdin`:

```json
{"title":"HTTP services","connectionIds":["QUICKCHART_ID","OPEN_METEO_ID"]}
```

When updating an existing entry, include its `id` and current `editVersion` from `gateway list`, retaining all existing member IDs and tool selections. The entry only references member connections; it never copies their credentials.

Connect the returned entry using the existing configuration transactions:

```sh
mcplink changes preview --connection ENTRY_ID --target codex:user --target hermes:user --mode shared
mcplink changes apply PLAN_ID
mcplink connections tools ENTRY_ID
```

Native clients discover tools named `MEMBER_NAMESPACE__render_chart` and `MEMBER_NAMESPACE__get_weather`. The entry assigns stable member namespaces; use the exact names returned by discovery. The same aggregate tools can be selected with `cli generate ENTRY_ID --name api-tools --output ./api-tools --tool TOOL_NAME`. Direct native MCP export is rejected for ordinary HTTP APIs, because their URLs do not speak MCP.

## Definition contract

`connections save --stdin` accepts the existing connection envelope. Set `definition.url` to the API origin, `definition.httpApi` to the operation definitions, and authentication in `definition.headers`. Complete examples are in [QuickChart](../examples/http-api/quickchart.json) and [Open-Meteo](../examples/http-api/open-meteo.json).

An operation has these fields:

| Field | Meaning |
| --- | --- |
| `name`, `description` | Stable tool name and agent-facing instructions |
| `inputSchema` | Object JSON Schema; validated before every HTTP call, including native MCP calls |
| `method` | `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, or `HEAD` |
| `path` | Absolute path on the configured origin, e.g. `/v1/items/{id}` |
| `pathParams` | Templates for whole-segment `{id}` placeholders; values are percent-encoded |
| `query` | Query parameter templates; scalar arrays are joined by commas |
| `body` | Optional typed JSON body template; unavailable for GET/HEAD |
| `response` | Result type, optional JSON Pointer and expected image MIME type |
| `annotations` | Optional MCP tool annotations; advisory metadata |

A template value may be a literal, nested object/array, or an input reference:

```json
{"$input":"/field","default":"optional fallback"}
```

The reference uses JSON Pointer, including an empty pointer for the complete argument object. Missing references omit object/query members. Required path parameters and array entries fail when missing. An explicit JSON null stays null in a body and is omitted from query parameters. Inputs remain typed; there is no string interpolation, arbitrary script execution, or remote Schema retrieval.

Response mapping types:

| `response.type` | Conversion |
| --- | --- |
| `json` | Parse JSON; optional `pointer` selects a subtree; provide text and structured content |
| `text` | Return UTF-8 text |
| `image` | Validate PNG/JPEG/WebP MIME type and signature; return MCP image content |
| `base64Image` | Decode a Base64 string, optionally selected by `pointer` from a JSON response; `mimeType` defaults to `image/png` |

`response.mimeType` can constrain image output to `image/png`, `image/jpeg`, or `image/webp`.

## Authentication and verification

Static authentication uses the existing managed `headers`, including `Authorization` or a custom API-key header. Values are stored in the system credential store and masked on read. `credentials update ID --stdin` preserves its existing semantics: values are saved verbatim, so include `Bearer ` when required. Headers can use existing environment references. Keep credentials out of tool templates and input defaults.

This adapter does not perform MCP OAuth discovery against ordinary APIs. It does not provide a separate account store. Credential changes, disconnects, shared pause, global filters, entry tool policies, connection revisions and contract checks use the existing managed paths.

Tool discovery is local. A fresh connection remains `configured` until a real operation succeeds. `connections tools` reports `verification.upstream: not_checked`; it can still list valid tools. A successful actual operation marks the member `ready`, and later checks report `previous_call`. An optional `httpApi.probe` is an explicit operation name and argument object. Only `connections probe` executes that sample; it may have the operation's cost or side effects. Without a configured probe or a prior successful call, probe returns a non-ready result. Discovery is never evidence that a key, network route, or business permission has been verified.

## Limits and validation

- HTTPS is required, except for loopback HTTP. Requests stay on the configured origin, do not follow redirects, and are not automatically retried.
- Each definition contains 1–128 tools and at most 512 KiB of configuration. Responses are bounded to 8 MiB. The existing per-connection timeout applies to headers and the entire response body.
- HTTP errors return an MCP tool error with the status code. Error response bodies and transport URLs are not echoed. JSON APIs that encode business failures in successful bodies retain their own result contract.
- A timeout or interrupted call can leave execution uncertain. Cancelling a downstream request does not promise upstream rollback.
- This version supports explicit operation definitions. OpenAPI import, multipart/binary upload, pagination orchestration, streaming partial images, remote artifact hosting, and asynchronous job APIs are not implemented.
- QuickChart's public endpoint is rate-limited. Open-Meteo's free endpoint is for non-commercial use. Consult [QuickChart](https://quickchart.io/documentation/usage/post-endpoint/) and [Open-Meteo](https://open-meteo.com/en/docs) before production use.

Run the synthetic CLI/Bridge acceptance with `python3 tests/e2e/http_api.py --record artifacts/e2e/http-api-local`. Its checks cover mapping, types, image results, validation, errors, credential reuse, aggregate routing, native configuration transactions and shared pause. Public service and native model acceptance are separate checks; passing the local fixture does not establish those results.
