// Use the published Pi implementation, independently of mcplink's serializers.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [packageDir, agentDir, cwd, trusted] = process.argv.slice(2);
const base = join(packageDir, "dist/extensions/mcp");
const { loadMcpConfig } = await import(pathToFileURL(join(base, "config.js")));
const { McpServerConnection, createDefaultTransport, McpOAuthCredentialStore } =
  await import(pathToFileURL(join(base, "runtime.js")));
const loaded = loadMcpConfig({ agentDir, cwd, projectTrusted: trusted === "true" });
if (loaded.errors.length) throw new Error(JSON.stringify(loaded.errors));
const credentials = new McpOAuthCredentialStore(join(agentDir, "mcp-auth.json"));
const reports = [];
for (const entry of loaded.servers) {
  if (entry.config.enabled === false) {
    reports.push({ name: entry.name, enabled: false });
    continue;
  }
  const connection = new McpServerConnection({
    entry, cwd, credentials, createTransport: createDefaultTransport,
    onTools() {},
  });
  try {
    await connection.getClient();
    const result = await connection.callTool("read_document", { id: "weekly-fixture" }, {});
    if (result.isError) throw new Error(JSON.stringify(result));
    const value = JSON.parse(result.content[0].text);
    if (value.workspace !== "fixture-workspace") throw new Error("Unexpected synthetic document");
    reports.push({ name: entry.name, scope: entry.scope, state: connection.state, toolCall: "PASS" });
  } finally {
    await connection.close();
  }
}
const pkg = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"));
process.stdout.write(JSON.stringify({ package: pkg.name, version: pkg.version, projectTrusted: trusted === "true", servers: reports }));
