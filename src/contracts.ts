import { invoke } from "@tauri-apps/api/core";

export type Agent =
  | "codex"
  | "hermes"
  | "claude"
  | "opencode"
  | "zcode"
  | "deepseek"
  | "openclaw"
  | "kimicode";
export const agents: Agent[] = [
  "codex",
  "hermes",
  "claude",
  "opencode",
  "zcode",
  "deepseek",
  "openclaw",
  "kimicode",
];
export type LocalAgent = {
  id: Agent;
  label: string;
  detected: boolean;
  installed: boolean;
  configurable: boolean;
  profilePath: string;
  management: "writable" | "readOnly" | "discovery";
};
export type Json = Record<string, unknown>;
export type Connection = {
  id: string;
  name: string;
  title: string;
  transport: string;
  endpoint: string;
  accountLabel: string;
  authKind: string;
  active: boolean;
  status: string;
  checkedAt?: number;
  toolCount?: number;
  lastError?: string;
  detected?: boolean;
  sourceId?: string;
};
export type Source = {
  id: string;
  agent: Agent;
  label: string;
  path: string;
  exists: boolean;
  error?: string;
  readOnly?: boolean;
  scope?: string;
  project?: string;
};
export type Binding = {
  id: string;
  connectionId?: string;
  sourceId: string;
  agent: Agent;
  name: string;
  enabled: boolean;
  changed: boolean;
  needsUpdate?: boolean;
  missing?: boolean;
  mode: string;
  overriddenIn?: { sourceId: string; project: string; scope: string }[];
};
export type Operation = {
  cursor: number;
  id: string;
  name: string;
  sourceId: string;
  status: string;
  createdAt: number;
  error?: string;
};
export type Snapshot = {
  agents: LocalAgent[];
  connections: Connection[];
  bindings: Binding[];
  sources: Source[];
  history: Operation[];
  gateway: { paused: boolean; sessions: number };
  platform: string;
  version: string;
};
export type DesktopStatus = {
  supported: boolean;
  autostartEnabled?: boolean;
  platform?: string;
};
export type Change = {
  id: string;
  agent: Agent;
  sourceId: string;
  sourceLabel: string;
  path: string;
  name: string;
  mode: string;
  action: string;
  reload: string;
  before?: Json;
  after?: Json;
};
export type Plan = {
  planId: string;
  changes: Change[];
  connection: Connection;
};
export type Tool = { name: string; description?: string; inputSchema?: Json };
export type Definition = {
  command?: string;
  args: string[];
  env: Record<string, string>;
  cwd?: string;
  url?: string;
  headers: Record<string, string>;
  enabled: boolean;
  oauth: boolean;
  clientId?: string;
  clientSecret?: string;
  oauthScopes?: string[];
  oauthCallbackPort?: number;
  includeTools?: string[];
  excludeTools?: string[];
  timeout?: number;
  startupTimeout?: number;
  includeSpecified?: boolean;
  resourcesDisabled?: boolean;
  promptsDisabled?: boolean;
};
export type Dialog =
  | {
      kind: "confirm";
      title: string;
      message: string;
      method: string;
      params: Json;
    }
  | { kind: "sources" }
  | { kind: "addSource"; agent: Agent }
  | { kind: "history" }
  | { kind: "background" }
  | { kind: "editor"; connection?: Connection }
  | { kind: "connect"; connections: Connection[]; targetAgent?: Agent }
  | { kind: "detail"; connection: Connection }
  | { kind: "credential"; connection: Connection };

export const agentLabel = (agent: Agent) =>
  ({
    codex: "Codex",
    hermes: "Hermes",
    claude: "Claude Code",
    opencode: "OpenCode",
    zcode: "ZCode",
    deepseek: "DeepSeek Harness",
    openclaw: "OpenClaw",
    kimicode: "Kimi Code",
  })[agent] || agent;
export const time = (value?: number) =>
  value
    ? new Date(value * 1000).toLocaleString("zh-CN", {
        month: "numeric",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      })
    : "尚未检查";

export async function rpc<T>(method: string, params: Json = {}): Promise<T> {
  const response = await invoke<{ ok?: T; error?: { message: string } }>(
    "rpc",
    { method, params },
  ).catch((error: unknown) => {
    throw error instanceof Error ? error : new Error(String(error));
  });
  if (response.error) throw new Error(response.error.message);
  if (response.ok === undefined) throw new Error("后台返回的结果不完整");
  return response.ok;
}
