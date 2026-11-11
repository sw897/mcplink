import { invoke } from "@tauri-apps/api/core";

export type Agent =
  | "codex"
  | "hermes"
  | "claude"
  | "opencode"
  | "zcode"
  | "deepseek"
  | "openclaw"
  | "kimicode"
  | "workbuddy"
  | "qwenwork"
  | "pi"
  | "doubaowork";
export const agents: Agent[] = [
  "codex",
  "hermes",
  "claude",
  "opencode",
  "zcode",
  "deepseek",
  "openclaw",
  "kimicode",
  "workbuddy",
  "qwenwork",
  "pi",
  "doubaowork",
];
export type LocalAgent = {
  id: Agent;
  label: string;
  detected: boolean;
  installed: boolean;
  configurable: boolean;
  profilePath: string | null;
  management: "writable" | "readOnly" | "discovery" | "manual";
  supportsImport: boolean;
  managementReason?: string | null;
};
export type ImportConfig = {
  agent: Agent;
  mode: string;
  config: Json | null;
  entries: {
    name: string;
    definition: {
      type: string;
      command?: string;
      args?: string[];
      env?: Record<string, string>;
      url?: string;
      headers?: Record<string, string>;
    };
  }[];
  gatewayPaused: boolean;
  requiresImport: boolean;
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
  revision?: number;
  status: string;
  checkedAt?: number;
  toolCount?: number;
  lastError?: string;
  detected?: boolean;
  sourceId?: string;
  members?: {
    connectionId: string;
    namespace: string;
    includeTools?: string[] | null;
  }[];
  editVersion?: string;
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
  unavailable?: boolean;
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
  kind?: string;
  restoreEffect?: string;
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
  kind: string;
  fields: { field: string; key?: string; kind: string }[];
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
  | { kind: "gateway"; connection?: Connection }
  | { kind: "cli"; connection: Connection }
  | {
      kind: "connect";
      connections: Connection[];
      targetAgent?: Agent;
      binding?: Binding;
    }
  | {
      kind: "adopt";
      connections: Connection[];
      intent: "connect" | "probe" | "credential" | "detail";
      targetAgent?: Agent;
      binding?: Binding;
    }
  | { kind: "restore"; operation: Operation; change: Change; effect: string }
  | { kind: "detail"; connection: Connection }
  | { kind: "credential"; connection: Connection };

export const changeLabel = (kind?: string) =>
  ({
    add: "新增",
    update: "更新",
    remove: "移除",
    cleanup: "清理失效绑定",
    disable: "停用",
    enable: "启用",
    restore: "恢复",
  })[kind || ""] || "历史变更";

export const restoreLabel = (effect?: string) =>
  ({
    remove: "移除当前条目",
    add: "重新添加原配置",
    update: "还原该条目的配置",
    none: "无需写入配置文件",
  })[effect || ""] || "核对恢复内容";

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
    workbuddy: "WorkBuddy",
    qwenwork: "千问办公",
    pi: "Pi Agent",
    doubaowork: "豆包工作",
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
