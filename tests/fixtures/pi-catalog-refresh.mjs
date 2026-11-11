// Verify live refresh using Pi's published MCP runtime, without a model turn.
import { readFileSync, writeFileSync, renameSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [packageDir, agentDir, cwd, control] = process.argv.slice(2);
const base = join(packageDir, "dist/extensions/mcp");
const { loadMcpConfig } = await import(pathToFileURL(join(base, "config.js")));
const { McpServerConnection, createDefaultTransport, McpOAuthCredentialStore } = await import(pathToFileURL(join(base, "runtime.js")));
const loaded = loadMcpConfig({ agentDir, cwd, projectTrusted: true });
if (loaded.errors.length || loaded.servers.length !== 1) throw new Error("Expected one native mcplink entry");
let refreshes = 0;
const connection = new McpServerConnection({ entry: loaded.servers[0], cwd,
  credentials: new McpOAuthCredentialStore(join(agentDir, "mcp-auth.json")),
  createTransport: createDefaultTransport, onTools() { refreshes++; },
});
try {
  const client = await connection.getClient();
  const previous = await client.listTools();
  if (!previous.some(t => t.name.endsWith("__read_1"))) throw new Error("Initial catalog missing");
  const state = JSON.parse(readFileSync(control, "utf8"));
  writeFileSync(control + ".pi-tmp", JSON.stringify({ ...state, tools: 2 }));
  renameSync(control + ".pi-tmp", control);
  const deadline = Date.now() + 12000;
  while (!connection.tools.some(t => t.name.endsWith("__read_2"))) {
    if (Date.now() >= deadline) throw new Error("Pi did not refresh its tool catalog");
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  if (refreshes < 2) throw new Error("Missing Pi onTools refresh callback");
  process.stdout.write(JSON.stringify({ conclusion: "PASS", client: "official Pi MCP runtime", entryCount: 1, refreshes, modelCalls: false, toolCalls: false }));
} finally {
  await connection.close();
}
