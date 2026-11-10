// Independent native-format reader used to launch the generated MCP entries.
import { readFileSync } from "node:fs";
import JSON5 from "json5";
import { parse as parseYaml } from "yaml";

const [agent, path, name] = process.argv.slice(2);
const text = readFileSync(path, "utf8");
let entry;
if (agent === "hermes") {
  entry = parseYaml(text).mcp_servers[name];
} else if (agent === "deepseek") {
  const rows = parseYaml(text);
  const plugins = rows.flatMap((row) => [row, ...(row.insert || [])]);
  const row = plugins.find((row) => row.name === "@deepseek-ai/dsh-mcp-client" && row.config?.serverName === name);
  if (!row || row.disabled) throw new Error("Missing or disabled native MCP plugin");
  entry = row.config;
  if (entry.transport !== "stdio") throw new Error("Expected native stdio transport");
} else {
  const raw = agent === "pi" ? JSON.parse(text.replace(/^\uFEFF/, "")) : ["qwenwork", "workbuddy"].includes(agent) ? JSON.parse(text) : JSON5.parse(text);
  entry = ["zcode", "openclaw", "opencode"].includes(agent) ? raw.mcp.servers[name] : raw.mcpServers[name];
  if (agent === "opencode") {
    if (entry.type !== "local" || entry.disabled) throw new Error("Invalid OpenCode local entry");
    entry = { command: entry.command[0], args: entry.command.slice(1), env: entry.environment };
  }
}
if (!entry || typeof entry.command !== "string" || !Array.isArray(entry.args)) throw new Error("Invalid native command/args contract");
if (entry.enabled === false || entry.enable === false) throw new Error("Native entry disabled");
process.stdout.write(JSON.stringify({ command: entry.command, args: entry.args, env: entry.env || {}, ...(entry.cwd ? { cwd: entry.cwd } : {}) }));
