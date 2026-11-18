# HTTP API → MCP Adapter

Register an ordinary HTTP API as a managed service. mcplink exposes its configured operations as MCP tools and uses the same member connection ID for single-service entries, aggregate entries, direct tool calls, and generated CLIs. Upstream services do not need an MCP implementation.

## Register and call

This source build uses management API 23. OpenAPI import, response conditions and protected credential relocation require matching desktop/CLI/backend builds; early 0.3.2 release packages with management API 22 do not implement them. Check the current `mcplink schema`; the CLI also rejects these configuration operations when the running backend is too old.

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
| `query`, `queryEncoding` | Query templates; arrays default to `comma`, with per-parameter `repeated`, `space` or `pipe` encoding |
| `body` | Optional typed JSON body template; unavailable for GET/HEAD |
| `requestContentType` | Optional JSON MIME type, e.g. `application/problem+json`; defaults to `application/json` |
| `response` | Result type, optional JSON Pointer and expected image MIME type |
| `annotations` | Optional MCP tool annotations; advisory metadata |

A template value may be a literal, nested object/array, or an input reference:

```json
{"$input":"/field","default":"optional fallback"}
```

The reference uses JSON Pointer, including an empty pointer for the complete argument object. Missing references omit object/query members, or the complete body when its root reference is absent. Required fields are enforced by inputSchema; path parameters and array entries fail when missing. An explicit JSON null stays null in a body and is omitted from query parameters. Inputs remain typed; there is no string interpolation, arbitrary script execution, or remote Schema retrieval.

Response mapping types:

| `response.type` | Conversion |
| --- | --- |
| `json` | Parse JSON; optional `pointer` selects a subtree; provide text and structured content |
| `text` | Return UTF-8 text |
| `image` | Validate PNG/JPEG/WebP MIME type and signature; return MCP image content |
| `base64Image` | Decode a Base64 string, optionally selected by `pointer` from a JSON response; `mimeType` defaults to `image/png` |

`response.mimeType` can constrain image output to `image/png`, `image/jpeg`, or `image/webp`.

## OpenAPI / Swagger import

In the desktop connection editor, choose HTTP API, expand **OpenAPI / Swagger 导入**, and select a local JSON/YAML file. Review the server, supported operations and tool names, then explicitly select operations and generate the editor draft. Saving the connection is a separate step. Authentication inputs remain empty until provided locally. HTTP methods and annotations do not establish business permissions.

The CLI converter also runs offline, without starting a daemon or opening a data directory:

```sh
mcplink http-api import --file service.yaml
mcplink http-api import --file service.yaml --operation 'GET /items/{id}'
mcplink http-api import --stdin
```

For stdin, provide `{ "source": "JSON or YAML text", "operations": ["GET /items/{id}"], "toolNames": {"GET /items/{id}": "get_item"} }`. Optional `serverUrl` overrides the server choice. The default selects nothing; `--all` / `all:true` explicitly selects every supported operation. Review unsupported operations even when using all. The response includes `operations`, `issues`, `credentialInputs` and a `definition` only for a valid nonempty selection. To save, place the reviewed definition inside a normal `connections save` envelope; a preview is not itself a save payload.

Supported documents are Swagger 2.0 and OpenAPI 3.0/3.1/3.2, bounded to 4 MiB, 1024 inspected operations and 128 selected tools. Conversion resolves bounded local references, preserves server base paths, maps scalar path/query parameters and JSON request bodies, and separates arguments into `path`, `query` and `body`. Query array serialization is preserved for supported encodings. Examples and Schema defaults are omitted instead of becoming executable request values. Header API keys, Basic, Bearer and manual OAuth tokens produce local credential-input hints; import does not perform OAuth.

External/recursive references, header/cookie parameters, object query encoding, multipart/binary uploads and unsupported media types remain visibly unsupported. Missing server addresses, conflicting authentication or tool-name collisions prevent draft generation. Import never fetches external references or invokes an API. Optional operation extension `x-mcplink-response` uses the same generic response mapping below; it contains no provider-specific execution code.

## Response conditions and credential filtering

For JSON APIs, a service can explicitly declare a scalar success condition:

```json
{"type":"json","success":{"pointer":"/code","equals":0},"sensitivePaths":["/data/privateField"]}
```

The condition is evaluated against the original JSON with exact value/type matching. A missing or mismatched field returns MCP `isError:true` and a nonzero tool CLI result, retaining the filtered response envelope for the Skill to interpret. Connection error metadata records a static failure reason, not the business response. Without a configured condition, mcplink does not infer a provider's business codes. Pagination, result interpretation, and operation sequencing belong in a Skill or service configuration.

Before returning JSON/text through either MCP or CLI, the HTTP Adapter filters values resolved from managed/custom request headers, including token-only, Basic password, cookie value, URL-encoded and JSON-escaped echoes. Common public protocol headers such as Content-Type and User-Agent are exempt unless listed in `httpApi.sensitiveHeaders`. Both text and structured JSON output are filtered. `response.sensitivePaths` masks exact JSON Pointers in the original response before subtree selection, on success and business failure.

These filters protect known request credentials and explicitly designated fields. They do not classify all private business data, detect arbitrary encodings, inspect image pixels or filter unrelated STDIO/HTTP MCP servers. A Skill still controls whether business content or delegated download URLs may be shown. Credentials belong in managed headers, not tool schemas, templates, URL queries or descriptions.

An existing connection can relocate protected fields through `connections save` with its current `id`, `definition.editVersion` and `credentialMappings`: an array of `{ "from":{"field":"env","name":"OLD_KEY"}, "to":{"field":"headers","name":"X-Api-Key"} }`. Values stay in backend memory and protected storage. This requires the same connection, cannot move values to public fields, rejects duplicate destinations and stale revisions, and cannot transfer old credentials while changing an HTTP origin. Keep the ID when replacing a STDIO wrapper with an HTTP API definition so aggregate namespaces remain stable.

## Authentication and verification

Static authentication uses the existing managed `headers`, including `Authorization` or a custom API-key header. Values are stored in the system credential store and masked on read. `credentials update ID --stdin` preserves its existing semantics: values are saved verbatim, so include `Bearer ` when required. Headers can use existing environment references. Keep credentials out of tool templates and input defaults.

This adapter does not perform MCP OAuth discovery against ordinary APIs. It does not provide a separate account store. Credential changes, disconnects, shared pause, global filters, entry tool policies, connection revisions and contract checks use the existing managed paths.

Tool discovery is local. A fresh connection remains `configured` until a real operation succeeds. `connections tools` reports `verification.upstream: not_checked`; it can still list valid tools. A successful actual operation marks the member `ready`, and later checks report `previous_call`. An optional `httpApi.probe` is an explicit operation name and argument object. Only `connections probe` executes that sample; it may have the operation's cost or side effects. Without a configured probe or a prior successful call, probe returns a non-ready result. Discovery is never evidence that a key, network route, or business permission has been verified.

In the desktop, **查看工具 / View tools** reads the saved definition without sending an HTTP request. Unverified tools remain visible. **运行探测 / Run probe** opens a separate confirmation showing the saved tool, HTTP method and path, without exposing managed credentials or argument values. Opening or cancelling it makes no API call. If no probe is configured, the dialog offers editing or an explicit call through Tool CLI.

Confirming performs one actual request against the previewed connection revision. A changed definition or credential rejects that confirmation before dispatch; reopen it to inspect the current operation. Each confirmation is single-use, including after a failure. Closing a running probe does not cancel or roll back an upstream request and does not reopen the dialog when it finishes. Check the service-side outcome before deliberately trying again. A failed actual call stays a failure even though the local tools remain inspectable.

Management callers can inspect `connections.tools`'s `verification.probe` (`null`, or `{tool, method, path}`) and pass its connection's `revision` as `expectedRevision` to `connections.probe` through RPC. This protects an explicit confirmation from configuration changes; it is not a replacement for authorization to execute the operation. The CLI's explicit `connections probe` command retains its existing behavior.

## Limits and validation

- HTTPS is required, except for loopback HTTP. Requests stay on the configured origin, do not follow redirects, and are not automatically retried.
- Each definition contains 1–128 tools and at most 512 KiB of configuration. Responses are bounded to 8 MiB. The existing per-connection timeout applies to headers and the entire response body.
- HTTP errors return an MCP tool error with the status code. Error response bodies and transport URLs are not echoed. JSON business failures use the service's optional explicit success condition.
- A timeout or interrupted call can leave execution uncertain. Cancelling a downstream request does not promise upstream rollback.
- Multipart/binary upload, pagination orchestration, streaming partial images, remote artifact hosting and asynchronous job APIs are not implemented.
- QuickChart's public endpoint is rate-limited. Open-Meteo's free endpoint is for non-commercial use. Consult [QuickChart](https://quickchart.io/documentation/usage/post-endpoint/) and [Open-Meteo](https://open-meteo.com/en/docs) before production use.

Run the synthetic CLI/Bridge acceptance with `python3 tests/e2e/http_api.py --record artifacts/e2e/http-api-local`. Its checks cover mapping, types, image results, validation, errors, credential reuse, aggregate routing, native configuration transactions and shared pause. Public service and native model acceptance are separate checks; passing the local fixture does not establish those results.

`python3 tests/e2e/openapi.py --record artifacts/e2e/openapi-local` covers offline JSON/YAML conversion, selection, unsupported operations, request serialization, response policies, credential filtering and protected same-ID migration.
