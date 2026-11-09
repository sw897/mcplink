import React, { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeftRight,
  ArrowRight,
  Check,
  CheckCircle2,
  ChevronDown,
  CircleHelp,
  Clock3,
  FolderPlus,
  History,
  KeyRound,
  Link2,
  Loader2,
  MoreHorizontal,
  Plus,
  RefreshCw,
  Search,
  Server,
  Settings2,
  ShieldCheck,
  Unplug,
  Users,
  X,
} from "lucide-react";
import "./style.css";

type Agent =
  | "codex"
  | "hermes"
  | "claude"
  | "opencode"
  | "zcode"
  | "deepseek"
  | "openclaw"
  | "kimicode";
const agents: Agent[] = [
  "codex",
  "hermes",
  "claude",
  "opencode",
  "zcode",
  "deepseek",
  "openclaw",
  "kimicode",
];
type LocalAgent = {
  id: Agent;
  label: string;
  detected: boolean;
  installed: boolean;
  configurable: boolean;
  profilePath: string;
  management: "writable" | "readOnly" | "discovery";
};
type Json = Record<string, unknown>;
type Connection = {
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
type Source = {
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
type Binding = {
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
type Operation = {
  id: string;
  name: string;
  sourceId: string;
  status: string;
  createdAt: number;
  error?: string;
};
type Snapshot = {
  agents: LocalAgent[];
  connections: Connection[];
  bindings: Binding[];
  sources: Source[];
  history: Operation[];
  gateway: { paused: boolean; sessions: number };
  platform: string;
  version: string;
};
type DesktopStatus = {
  supported: boolean;
  autostartEnabled?: boolean;
  platform?: string;
};
type Change = {
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
type Plan = { planId: string; changes: Change[]; connection: Connection };
type Tool = { name: string; description?: string; inputSchema?: Json };
type Definition = {
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
type Dialog =
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

const agentLabel = (agent: Agent) =>
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
const time = (value?: number) =>
  value
    ? new Date(value * 1000).toLocaleString("zh-CN", {
        month: "numeric",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      })
    : "尚未检查";

async function rpc<T>(method: string, params: Json = {}): Promise<T> {
  const response = await invoke<{ ok?: T; error?: { message: string } }>(
    "rpc",
    { method, params },
  );
  if (response.error) throw new Error(response.error.message);
  if (response.ok === undefined) throw new Error("后台返回的结果不完整");
  return response.ok;
}

function Status({ value, label }: { value: string; label?: string }) {
  const labels: Record<string, string> = {
    ready: "检查可用",
    configured: "已配置",
    error: "连接异常",
    needs_auth: "需要认证",
    disconnected: "已断开",
    authorizing: "等待授权",
    applied: "待 Agent 加载",
    failed: "操作失败",
    restored: "已恢复",
    conflict: "需要核对",
    pending: "处理中",
  };
  return (
    <span className={`status ${value}`}>
      <i />
      {label || labels[value] || value}
    </span>
  );
}

function Modal({
  title,
  children,
  onClose,
  wide,
}: {
  title: string;
  children: React.ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  useEffect(() => {
    const close = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [onClose]);
  return (
    <div
      className="overlay"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <section
        className={`modal ${wide ? "wide" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
      >
        <header>
          <h2>{title}</h2>
          <button className="icon" onClick={onClose} aria-label="关闭">
            <X size={19} />
          </button>
        </header>
        {children}
      </section>
    </div>
  );
}

function App() {
  const [page, setPage] = useState<"services" | "accounts" | "agents">(
    "services",
  );
  const [data, setData] = useState<Snapshot>();
  const [desktop, setDesktop] = useState<DesktopStatus>();
  const [activeAgent, setActiveAgent] = useState<Agent>("codex");
  const [agentOnlyConfigured, setAgentOnlyConfigured] = useState(false);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [selected, setSelected] = useState<string[]>([]);
  const [dialog, setDialog] = useState<Dialog>();
  const [busy, setBusy] = useState("");
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [failure, setFailure] = useState("");
  const [checked, setChecked] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const loadSequence = useRef(0);
  const [tools, setTools] = useState<Record<string, Tool[]>>({});

  const load = useCallback(async () => {
    const sequence = ++loadSequence.current;
    setRefreshing(true);
    try {
      const next = await rpc<Snapshot>("snapshot");
      if (sequence !== loadSequence.current) return;
      if (
        !Array.isArray(next.agents) ||
        next.agents.length !== agents.length ||
        next.agents.some(
          (client) =>
            !agents.includes(client.id) ||
            client.configurable !== client.installed,
        )
      )
        throw new Error(
          "后台版本或支持名单与当前桌面不一致，请退出并重新启动 mcplink。",
        );
      setData(next);
      void invoke<DesktopStatus>("desktop_status")
        .then(setDesktop)
        .catch((error) => {
          setNotice({ text: String(error), error: true });
        });
      setDialog((previous) =>
        previous?.kind === "addSource" &&
        !next.agents.some(
          (client) => client.id === previous.agent && client.configurable,
        )
          ? undefined
          : previous,
      );
      setSelected((previous) =>
        previous.filter((id) =>
          next.connections.some((connection) => connection.id === id),
        ),
      );
      setFailure("");
      setChecked(Date.now());
    } catch (error) {
      if (sequence !== loadSequence.current) return;
      setFailure(error instanceof Error ? error.message : "无法读取本机配置");
    } finally {
      if (sequence === loadSequence.current) setRefreshing(false);
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    const subscriptions = [
      listen("gateway-changed", () => void load()),
      listen<DesktopStatus>("desktop-status-changed", (event) =>
        setDesktop(event.payload),
      ),
      listen<string>("desktop-error", (event) =>
        setNotice({ text: event.payload, error: true }),
      ),
    ];
    return () => {
      subscriptions.forEach(
        (subscription) => void subscription.then((unlisten) => unlisten()),
      );
    };
  }, [load]);
  useEffect(() => {
    const refresh = () => {
      void load();
    };
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [load]);
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(undefined), 7000);
    return () => clearTimeout(timer);
  }, [notice]);

  const action = async <T,>(
    label: string,
    method: string,
    params: Json = {},
  ) => {
    setBusy(label);
    try {
      const result = await rpc<T>(method, params);
      await load();
      return result;
    } catch (error) {
      setNotice({
        text: error instanceof Error ? error.message : "操作未完成",
        error: true,
      });
      return undefined;
    } finally {
      setBusy("");
    }
  };
  const manage = async (connection: Connection) =>
    connection.detected
      ? action<Connection>("读取连接", "connections.import", {
          sourceId: connection.sourceId,
          name: connection.name,
        })
      : connection;
  const connect = (connections: Connection[], targetAgent?: Agent) => {
    setDialog({ kind: "connect", connections, targetAgent });
  };
  const probe = async (connection: Connection) => {
    const managed = await manage(connection);
    if (!managed) return;
    const result = await action<{
      tools: Tool[];
      connection: Connection;
      error?: unknown;
    }>("检查连接", "connections.probe", { id: managed.id });
    if (result) {
      setTools((previous) => ({
        ...previous,
        [managed.id]: result.tools || [],
      }));
      setNotice({
        text:
          result.connection.status === "ready"
            ? `已连接，发现 ${result.tools.length} 个工具`
            : result.connection.lastError || "上游返回错误，请查看连接详情",
        error: result.connection.status !== "ready",
      });
      setDialog({ kind: "detail", connection: result.connection });
    }
  };
  const ownBindings = (connection: Connection) =>
    data?.bindings.filter(
      (binding) =>
        binding.connectionId === connection.id || binding.id === connection.id,
    ) || [];
  const list =
    data?.connections.filter((connection) => {
      const matches =
        `${connection.title} ${connection.name} ${connection.endpoint} ${connection.accountLabel}`
          .toLowerCase()
          .includes(query.toLowerCase());
      return (
        matches &&
        (filter === "all" ||
          filter === connection.transport ||
          (filter === "shared" &&
            ownBindings(connection).some(
              (binding) => binding.mode === "shared",
            )))
      );
    }) || [];
  const sourcesFor = (agent: Agent) =>
    data?.sources.filter((source) => source.agent === agent) || [];
  const localAgents = data?.agents || [];
  const isInstalled = (agent?: Agent) =>
    localAgents.some((client) => client.id === agent && client.configurable);

  const currentAgent =
    localAgents.find((client) => client.id === activeAgent) || localAgents[0];
  const currentSources = currentAgent ? sourcesFor(currentAgent.id) : [];
  const currentBindings =
    data?.bindings.filter(
      (binding) => binding.agent === currentAgent?.id && !binding.missing,
    ) || [];
  const currentServices =
    data?.connections.filter(
      (connection) =>
        !agentOnlyConfigured ||
        ownBindings(connection).some(
          (binding) => binding.agent === currentAgent?.id && !binding.missing,
        ),
    ) || [];
  const chooseAgent = (agent: Agent) => {
    setActiveAgent(agent);
    setAgentOnlyConfigured(false);
  };

  const bindingCell = (connection: Connection, agent: Agent) => {
    if (!isInstalled(agent)) return <span className="muted">未安装</span>;
    const bindings = ownBindings(connection).filter(
      (binding) => binding.agent === agent,
    );
    if (!bindings.length) {
      const available = sourcesFor(agent);
      const missing = available.length === 0;
      const writable = available.some((source) => !source.readOnly);
      return (
        <button
          className="cell-add"
          disabled={!writable && !missing}
          onClick={() =>
            missing
              ? setDialog({ kind: "addSource", agent })
              : void connect([connection], agent)
          }
        >
          <Plus size={13} />
          {missing ? "添加来源" : writable ? "接入" : "只读来源"}
        </button>
      );
    }
    const changed = bindings.some(
      (binding) => binding.changed || binding.needsUpdate,
    );
    const enabled = bindings.some((binding) => binding.enabled);
    return (
      <button
        className={`binding ${changed ? "changed" : ""} ${enabled ? "" : "disabled"}`}
        onClick={() => setDialog({ kind: "detail", connection })}
      >
        <span>
          {changed ? (
            <CircleHelp size={14} />
          ) : enabled ? (
            <Check size={14} />
          ) : (
            <Unplug size={14} />
          )}
          {changed
            ? bindings.some((b) => b.changed)
              ? "配置已变化"
              : "待同步"
            : enabled
              ? "已配置"
              : "已停用"}
        </span>
        <small>
          {bindings.some((binding) => binding.mode === "shared")
            ? "共享账号"
            : "直连"}
          {bindings.length > 1 ? ` · ${bindings.length} 个位置` : ""}
        </small>
      </button>
    );
  };

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <a className="brand" onClick={() => setPage("services")}>
          <span className="brand-mark">
            <Link2 size={24} strokeWidth={2.3} />
          </span>
          <span>mcplink</span>
        </a>
        <nav>
          {(
            [
              { id: "services", label: "MCP 服务", icon: Server },
              { id: "accounts", label: "账号", icon: KeyRound },
              { id: "agents", label: "Agent", icon: Users },
            ] as const
          ).map((item) => (
            <button
              key={item.id}
              className={page === item.id ? "active" : ""}
              onClick={() => {
                setPage(item.id);
                setSelected([]);
              }}
            >
              <item.icon size={18} />
              {item.label}
              {item.id === "services" && data && (
                <span>{data.connections.length}</span>
              )}
            </button>
          ))}
        </nav>
        <div className="sidebar-bottom">
          <button
            className="utility"
            onClick={() => setDialog({ kind: "background" })}
          >
            <Server size={16} />
            后台运行
          </button>
          <button
            className="utility"
            onClick={() => setDialog({ kind: "sources" })}
          >
            <Settings2 size={16} />
            配置来源
          </button>
          <button
            className="utility"
            onClick={() => setDialog({ kind: "history" })}
          >
            <History size={16} />
            变更与恢复
          </button>
          <div className="runtime">
            <i className={data?.gateway.paused ? "paused" : ""} />
            <span>
              本机管理{data?.gateway.paused ? " · 共享已暂停" : "已连接"}
            </span>
            <small>v{data?.version || "0.1.0"}</small>
          </div>
        </div>
      </aside>
      <main>
        <header className="page-header">
          <div>
            <p className="eyebrow">本机工具环境</p>
            <h1>
              {page === "services"
                ? "MCP 服务"
                : page === "accounts"
                  ? "账号"
                  : "Agent"}
            </h1>
            <p className="subtitle">
              {page === "services"
                ? "查看已有连接，接给你常用的 Agent。"
                : page === "accounts"
                  ? "查看认证由谁维护，以及哪些 Agent 正在使用。"
                  : "选择客户端，查看配置位置与 MCP 服务接入。"}
            </p>
          </div>
          <div className="header-actions">
            <button
              className="button"
              disabled={!!busy || refreshing}
              aria-busy={refreshing}
              onClick={() => void load()}
              aria-label="刷新本机配置"
            >
              <RefreshCw size={15} className={refreshing ? "spin" : ""} />
              {refreshing ? "刷新中…" : "刷新"}
            </button>
            <button
              className="button primary"
              onClick={() => setDialog({ kind: "editor" })}
            >
              <Plus size={16} />
              添加 MCP
            </button>
          </div>
        </header>
        {failure && (
          <div className="error-banner" role="alert">
            <CircleHelp size={18} />
            <div>
              <strong>本机数据暂时无法读取</strong>
              <p>{failure}</p>
            </div>
            <button className="text-button" onClick={() => void load()}>
              重试
            </button>
          </div>
        )}
        {!data && !failure && (
          <div className="empty loading">
            <Loader2 className="spin" size={25} />
            <p>正在读取本机配置…</p>
            <p className="muted">
              如有系统文件访问提示，请先处理；完成后可点击右上角“刷新”。
            </p>
          </div>
        )}
        {data && (
          <>
            <div className="scope-line">
              <span className="live-dot" />
              <span>
                {data.sources.filter((source) => source.exists).length}{" "}
                个配置位置
              </span>
              <span className="divider">/</span>
              <button onClick={() => setPage("agents")}>
                支持 {localAgents.length} 类 · 已安装{" "}
                {localAgents.filter((agent) => agent.installed).length} 类
                <ArrowRight size={12} />
              </button>
              <button onClick={() => setDialog({ kind: "sources" })}>
                查看范围
                <ArrowRight size={12} />
              </button>
              <small>
                更新于{" "}
                {new Date(checked).toLocaleTimeString("zh-CN", {
                  hour: "2-digit",
                  minute: "2-digit",
                  second: "2-digit",
                })}
              </small>
            </div>
            {data.sources.some((source) => source.error) && (
              <div className="warning-banner">
                部分配置读取失败，已显示其他可读取的服务。
                <button
                  className="text-button"
                  onClick={() => setDialog({ kind: "sources" })}
                >
                  查看来源
                </button>
              </div>
            )}
            {page === "services" && (
              <>
                <div className="toolbar">
                  <div className="filters">
                    {[
                      ["all", "全部"],
                      ["stdio", "本地进程"],
                      ["http", "HTTP"],
                      ["shared", "共享账号"],
                    ].map(([id, label]) => (
                      <button
                        key={id}
                        className={filter === id ? "active" : ""}
                        onClick={() => setFilter(id)}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                  <label className="search">
                    <Search size={16} />
                    <input
                      placeholder="搜索服务或账号"
                      value={query}
                      onChange={(event) => setQuery(event.target.value)}
                    />
                  </label>
                </div>
                {selected.length > 0 && (
                  <div className="selection-bar">
                    <span>已选 {selected.length} 项</span>
                    <button
                      className="button small"
                      disabled={!!busy}
                      onClick={() =>
                        void connect(
                          data.connections.filter((connection) =>
                            selected.includes(connection.id),
                          ),
                        )
                      }
                    >
                      <ArrowLeftRight size={14} />
                      批量接入
                    </button>
                    <button
                      className="text-button"
                      onClick={() => setSelected([])}
                    >
                      取消选择
                    </button>
                  </div>
                )}
                <section className="service-list" aria-label="MCP 服务列表">
                  {list.length > 0 && (
                    <label className="service-list-heading">
                      <input
                        type="checkbox"
                        aria-label="选择全部服务"
                        checked={list.every((connection) =>
                          selected.includes(connection.id),
                        )}
                        onChange={(event) =>
                          setSelected(
                            event.target.checked
                              ? list.map((connection) => connection.id)
                              : [],
                          )
                        }
                      />
                      <span>服务连接</span>
                      <small>{list.length} 项</small>
                    </label>
                  )}
                  {list.map((connection) => {
                    const linkedAgents = localAgents.filter((client) =>
                      ownBindings(connection).some(
                        (binding) =>
                          binding.agent === client.id && !binding.missing,
                      ),
                    );
                    return (
                      <article className="service-card" key={connection.id}>
                        <div className="service-card-heading">
                          <input
                            type="checkbox"
                            aria-label={`选择 ${connection.title}`}
                            checked={selected.includes(connection.id)}
                            onChange={(event) =>
                              setSelected((previous) =>
                                event.target.checked
                                  ? [...previous, connection.id]
                                  : previous.filter(
                                      (id) => id !== connection.id,
                                    ),
                              )
                            }
                          />
                          <button
                            className="service-name"
                            onClick={() =>
                              setDialog({ kind: "detail", connection })
                            }
                          >
                            <span
                              className={`service-icon ${connection.transport}`}
                            >
                              <Server size={18} />
                            </span>
                            <span>
                              <strong>{connection.title}</strong>
                              <small title={connection.endpoint}>
                                {connection.endpoint}
                              </small>
                              {connection.authKind !== "none" && (
                                <em>{connection.accountLabel}</em>
                              )}
                            </span>
                          </button>
                          <div className="service-health">
                            <Status value={connection.status} />
                            {connection.checkedAt && (
                              <small className="checked-at">
                                {time(connection.checkedAt)}
                              </small>
                            )}
                          </div>
                          <button
                            className="icon"
                            aria-label={`${connection.title} 详情`}
                            onClick={() =>
                              setDialog({ kind: "detail", connection })
                            }
                          >
                            <MoreHorizontal size={19} />
                          </button>
                        </div>
                        <div className="service-card-footer">
                          <div
                            className="linked-agents"
                            aria-label="已接入 Agent"
                          >
                            {linkedAgents.length ? (
                              linkedAgents.map((client) => (
                                <button
                                  className="agent-tag"
                                  key={client.id}
                                  onClick={() => {
                                    chooseAgent(client.id);
                                    setPage("agents");
                                  }}
                                >
                                  <span className={`agent-avatar ${client.id}`}>
                                    {client.label[0]}
                                  </span>
                                  {client.label}
                                  <small>
                                    {!client.installed
                                      ? "未安装"
                                      : ownBindings(connection).some(
                                            (binding) =>
                                              binding.agent === client.id &&
                                              binding.enabled,
                                          )
                                        ? ownBindings(connection).some(
                                            (binding) =>
                                              binding.agent === client.id &&
                                              binding.mode === "shared",
                                          )
                                          ? "共享"
                                          : "直连"
                                        : "已停用"}
                                  </small>
                                </button>
                              ))
                            ) : (
                              <span className="muted">尚未接入 Agent</span>
                            )}
                          </div>
                          <button
                            className="button small"
                            disabled={
                              !!busy ||
                              !localAgents.some((client) => client.configurable)
                            }
                            onClick={() => connect([connection])}
                          >
                            <Plus size={14} />
                            接给 Agent
                          </button>
                        </div>
                      </article>
                    );
                  })}
                  {!list.length && (
                    <div className="empty">
                      <Server size={30} />
                      <h3>
                        {query ? "没有匹配的服务" : "把已有的 MCP 接到一起"}
                      </h3>
                      <p>
                        {query
                          ? "尝试其他名称或筛选条件。"
                          : "支持的配置会自动显示，也可以添加服务或配置位置。"}
                      </p>
                      {!query && (
                        <button
                          className="button"
                          onClick={() => setDialog({ kind: "sources" })}
                        >
                          <FolderPlus size={16} />
                          添加配置位置
                        </button>
                      )}
                    </div>
                  )}
                </section>
                <footer className="table-footer">
                  <span>{list.length} 项服务连接</span>
                  <span>
                    <ShieldCheck size={13} />
                    托管认证保存在系统凭据库
                  </span>
                </footer>
              </>
            )}
            {page === "accounts" && (
              <div className="account-list">
                {data.connections
                  .filter(
                    (connection) =>
                      connection.authKind !== "none" ||
                      connection.transport === "http",
                  )
                  .map((connection) => (
                    <article className="account-card" key={connection.id}>
                      <div className="account-heading">
                        <span className="service-icon http">
                          <KeyRound size={21} />
                        </span>
                        <div>
                          <h3>{connection.title}</h3>
                          <p>{connection.accountLabel || "账号信息待确认"}</p>
                        </div>
                        <Status value={connection.status} />
                      </div>
                      <dl>
                        <div>
                          <dt>认证维护</dt>
                          <dd>
                            {connection.detected
                              ? "原客户端 / 服务"
                              : connection.authKind === "oauth"
                                ? "mcplink · OAuth"
                                : "mcplink · 令牌"}
                          </dd>
                        </div>
                        <div>
                          <dt>关联 Agent</dt>
                          <dd>
                            {[
                              ...new Set(
                                ownBindings(connection).map((binding) =>
                                  agentLabel(binding.agent),
                                ),
                              ),
                            ].join("、") || "尚未接入"}
                          </dd>
                        </div>
                        <div>
                          <dt>最近核验</dt>
                          <dd>{time(connection.checkedAt)}</dd>
                        </div>
                      </dl>
                      {connection.lastError && (
                        <p className="inline-error">{connection.lastError}</p>
                      )}
                      <div className="card-actions">
                        <button
                          className="button small"
                          disabled={!!busy}
                          onClick={() => void probe(connection)}
                        >
                          检查连接
                        </button>
                        <button
                          className="button small"
                          onClick={async () => {
                            const managed = await manage(connection);
                            if (managed)
                              setDialog({
                                kind: "credential",
                                connection: managed,
                              });
                          }}
                        >
                          更新认证
                        </button>
                        {!connection.detected && (
                          <button
                            className="text-button danger"
                            onClick={() =>
                              setDialog({
                                kind: "confirm",
                                title: "断开托管认证",
                                message: `断开 ${connection.title} 的认证？共享此账号的 Agent 将需要重新认证；原生直连凭据需在相应客户端处理。`,
                                method: "credentials.disconnect",
                                params: { id: connection.id },
                              })
                            }
                          >
                            断开
                          </button>
                        )}
                      </div>
                    </article>
                  ))}
                {!data.connections.some(
                  (connection) =>
                    connection.authKind !== "none" ||
                    connection.transport === "http",
                ) && (
                  <div className="empty">
                    <KeyRound size={30} />
                    <h3>账号会随服务一起出现</h3>
                    <p>添加需要认证的 HTTP MCP，或从已有配置纳入管理。</p>
                  </div>
                )}
              </div>
            )}
            {page === "agents" && currentAgent && (
              <div className="agent-workspace">
                <aside className="agent-directory" aria-label="选择 Agent">
                  {[true, false].map((installed) => (
                    <div
                      className="agent-directory-group"
                      key={String(installed)}
                    >
                      <h2>
                        {installed ? "已安装" : "未安装"}
                        <span>
                          {
                            localAgents.filter(
                              (client) => client.installed === installed,
                            ).length
                          }
                        </span>
                      </h2>
                      {localAgents
                        .filter((client) => client.installed === installed)
                        .map((client) => (
                          <button
                            className={`agent-choice ${currentAgent.id === client.id ? "active" : ""}`}
                            key={client.id}
                            aria-pressed={currentAgent.id === client.id}
                            aria-label={`查看 ${client.label}${client.installed ? "" : "（未安装）"}`}
                            onClick={() => chooseAgent(client.id)}
                          >
                            <span className={`agent-avatar ${client.id}`}>
                              {client.label[0]}
                            </span>
                            <span className="agent-choice-name">
                              {client.label}
                            </span>
                            {client.installed && (
                              <span
                                className="installed-dot"
                                aria-label="已安装"
                              />
                            )}
                          </button>
                        ))}
                    </div>
                  ))}
                </aside>
                <section
                  className="agent-panel"
                  aria-label={`${currentAgent.label} MCP 接入`}
                >
                  <header className="agent-panel-heading">
                    <div className="agent-panel-title">
                      <span className={`agent-avatar large ${currentAgent.id}`}>
                        {currentAgent.label[0]}
                      </span>
                      <div>
                        <h2>{currentAgent.label}</h2>
                        <span
                          className={`presence-badge ${currentAgent.installed ? "installed" : ""}`}
                        >
                          {currentAgent.installed ? "已安装" : "未安装"}
                        </span>
                      </div>
                    </div>
                    <div className="agent-panel-actions">
                      <button
                        className="button small"
                        disabled={!currentAgent.configurable}
                        onClick={() =>
                          setDialog({
                            kind: "addSource",
                            agent: currentAgent.id,
                          })
                        }
                      >
                        <FolderPlus size={14} />
                        添加来源
                      </button>
                      <button
                        className="button small primary"
                        disabled={
                          !currentAgent.configurable ||
                          !data.connections.length ||
                          !currentSources.some((source) => !source.readOnly)
                        }
                        onClick={() =>
                          connect(data.connections, currentAgent.id)
                        }
                      >
                        <Plus size={14} />
                        接入服务
                      </button>
                    </div>
                  </header>
                  {!currentAgent.installed ? (
                    <div className="agent-unavailable">
                      <Unplug size={19} />
                      <div>
                        <strong>未安装，暂不可配置</strong>
                        <p>
                          安装 {currentAgent.label} 后点击刷新，即可接入 MCP
                          服务。
                          {currentAgent.detected
                            ? "检测到的配置残留仅供查看。"
                            : ""}
                        </p>
                      </div>
                    </div>
                  ) : (
                    <div className="agent-metrics">
                      <span>
                        <strong>
                          {
                            currentSources.filter((source) => source.exists)
                              .length
                          }
                        </strong>
                        处配置
                      </span>
                      <span>
                        <strong>{currentBindings.length}</strong>项已接入
                      </span>
                      <span>
                        <strong>
                          {
                            currentBindings.filter(
                              (binding) => binding.mode === "shared",
                            ).length
                          }
                        </strong>
                        项共享代理
                      </span>
                    </div>
                  )}
                  <div className="agent-service-heading">
                    <h3>MCP 服务</h3>
                    <div
                      className="agent-service-filters"
                      aria-label="按接入状态筛选"
                    >
                      <button
                        className={!agentOnlyConfigured ? "active" : ""}
                        onClick={() => setAgentOnlyConfigured(false)}
                      >
                        全部 {data.connections.length}
                      </button>
                      <button
                        className={agentOnlyConfigured ? "active" : ""}
                        onClick={() => setAgentOnlyConfigured(true)}
                      >
                        已接入{" "}
                        {
                          new Set(
                            currentBindings.map(
                              (binding) => binding.connectionId || binding.id,
                            ),
                          ).size
                        }
                      </button>
                    </div>
                  </div>
                  <div className="agent-services">
                    {currentServices.map((connection) => (
                      <div className="agent-service-row" key={connection.id}>
                        <button
                          className="service-name"
                          onClick={() =>
                            setDialog({ kind: "detail", connection })
                          }
                        >
                          <span
                            className={`service-icon ${connection.transport}`}
                          >
                            <Server size={17} />
                          </span>
                          <span>
                            <strong>{connection.title}</strong>
                            <small title={connection.endpoint}>
                              {connection.endpoint}
                            </small>
                          </span>
                        </button>
                        {bindingCell(connection, currentAgent.id)}
                      </div>
                    ))}
                    {!currentServices.length && (
                      <div className="agent-empty">
                        <Server size={22} />
                        <p>
                          {agentOnlyConfigured
                            ? "该客户端尚未接入 MCP 服务"
                            : "暂无 MCP 服务，可先在 MCP 服务页添加"}
                        </p>
                      </div>
                    )}
                  </div>
                  <details className="agent-source-details">
                    <summary>
                      配置位置<span>{currentSources.length}</span>
                      <ChevronDown size={14} />
                    </summary>
                    <div>
                      {currentSources.map((source) => (
                        <div className="agent-source-item" key={source.id}>
                          <strong>
                            {source.label}
                            <small>
                              {source.readOnly
                                ? "只读"
                                : !currentAgent.installed
                                  ? "不可配置"
                                  : source.exists
                                    ? "已找到"
                                    : "尚未创建"}
                            </small>
                          </strong>
                          <code>{source.path}</code>
                          {source.error && (
                            <p className="inline-error">{source.error}</p>
                          )}
                        </div>
                      ))}
                    </div>
                  </details>
                </section>
              </div>
            )}
          </>
        )}
      </main>
      {notice && (
        <div className={`toast ${notice.error ? "error" : ""}`} role="status">
          {notice.error ? <CircleHelp size={18} /> : <CheckCircle2 size={18} />}
          <span>{notice.text}</span>
          <button
            className="icon"
            onClick={() => setNotice(undefined)}
            aria-label="关闭提示"
          >
            <X size={14} />
          </button>
        </div>
      )}
      {busy && (
        <div className="busy-indicator">
          <Loader2 className="spin" size={15} />
          {busy}
        </div>
      )}
      {dialog?.kind === "sources" && (
        <Modal title="配置来源" wide onClose={() => setDialog(undefined)}>
          <div className="modal-body">
            <p className="muted">
              支持八类 Agent 的配置管理和 MCP
              共享代理。未安装的客户端不可配置，安装后刷新即可接入；配置目录残留不代表已安装。插件、托管及共享回退来源按各自权限维护。
            </p>
            {data?.sources.map((source) => (
              <div className="source-row" key={source.id}>
                <span className={`agent-avatar ${source.agent}`}>
                  {agentLabel(source.agent)[0]}
                </span>
                <div>
                  <strong>
                    {agentLabel(source.agent)} · {source.label}
                  </strong>
                  <code>{source.path}</code>
                  {source.project && <code>项目：{source.project}</code>}
                  <small className={source.error ? "inline-error" : "muted"}>
                    {(!isInstalled(source.agent)
                      ? "未安装 · 不可配置"
                      : source.error) ||
                      (source.exists ? "已找到配置" : "尚未创建配置文件") +
                        (source.readOnly ? " · 只读来源" : "")}
                  </small>
                </div>
              </div>
            ))}
            <div className="split-buttons">
              {agents.map((agent) => (
                <button
                  className="button"
                  key={agent}
                  disabled={!isInstalled(agent)}
                  onClick={async () => {
                    if (agent !== "codex" && agent !== "hermes") {
                      setDialog({ kind: "addSource", agent });
                      return;
                    }
                    const path = await open({
                      title: `选择 ${agentLabel(agent)} 配置`,
                      multiple: false,
                      filters: [
                        {
                          name: "配置文件",
                          extensions:
                            agent === "codex" ? ["toml"] : ["yaml", "yml"],
                        },
                      ],
                    });
                    if (path)
                      await action("添加来源", "sources.add", { agent, path });
                  }}
                >
                  <FolderPlus size={15} />
                  {isInstalled(agent)
                    ? `添加 ${agentLabel(agent)} 配置`
                    : `${agentLabel(agent)} · 未安装`}
                </button>
              ))}
            </div>
          </div>
        </Modal>
      )}
      {dialog?.kind === "addSource" && (
        <SourceDialog
          agent={dialog.agent}
          onClose={() => setDialog({ kind: "sources" })}
          onSaved={async () => {
            await load();
            setDialog({ kind: "sources" });
          }}
        />
      )}
      {dialog?.kind === "history" && (
        <Modal title="变更与恢复" wide onClose={() => setDialog(undefined)}>
          <div className="modal-body">
            <p className="muted">
              恢复仅处理该次操作涉及的条目，后续改动会先核对。
            </p>
            {data?.history.length ? (
              data.history.map((operation) => (
                <div className="history-row" key={operation.id}>
                  <Clock3 size={17} />
                  <div>
                    <strong>{operation.name}</strong>
                    <p>
                      {
                        data.sources.find(
                          (source) => source.id === operation.sourceId,
                        )?.agent
                      }{" "}
                      · {time(operation.createdAt)}
                    </p>
                    {operation.error && (
                      <small className="inline-error">{operation.error}</small>
                    )}
                  </div>
                  <Status value={operation.status} />
                  {operation.status === "applied" && (
                    <button
                      className="button small"
                      disabled={
                        !!busy ||
                        !isInstalled(
                          data.sources.find(
                            (source) => source.id === operation.sourceId,
                          )?.agent,
                        )
                      }
                      onClick={async () => {
                        const result = await action(
                          "恢复配置",
                          "changes.restore",
                          { operationId: operation.id },
                        );
                        if (result)
                          setNotice({
                            text: "配置已恢复，请在对应 Agent 重新加载",
                          });
                      }}
                    >
                      恢复
                    </button>
                  )}
                </div>
              ))
            ) : (
              <div className="empty">
                <History size={27} />
                <p>配置变更后，恢复记录会显示在这里。</p>
              </div>
            )}
            <div className="gateway-control">
              <div>
                <strong>共享后台</strong>
                <p className="muted">
                  {data?.gateway.paused
                    ? "共享调用已暂停"
                    : "共享连接按调用需要启动上游服务"}
                </p>
              </div>
              <button
                className="button"
                onClick={() =>
                  void action(
                    "调整共享后台",
                    data?.gateway.paused ? "gateway.resume" : "gateway.pause",
                  )
                }
              >
                {data?.gateway.paused ? "恢复共享" : "暂停共享"}
              </button>
            </div>
          </div>
        </Modal>
      )}
      {dialog?.kind === "background" && (
        <Modal title="后台运行" onClose={() => setDialog(undefined)}>
          <div className="modal-body background-settings">
            <div className="gateway-control">
              <div>
                <strong>
                  {data?.gateway.paused ? "MCP 共享已暂停" : "MCP 共享运行中"}
                </strong>
                <p className="muted">
                  暂停后停止共享调用，后台重启后仍保持暂停。
                </p>
              </div>
              <button
                className="button"
                disabled={!!busy || !data}
                onClick={() =>
                  void action(
                    "调整共享后台",
                    data?.gateway.paused ? "gateway.resume" : "gateway.pause",
                  )
                }
              >
                {data?.gateway.paused ? "恢复共享" : "暂停共享"}
              </button>
            </div>
            {desktop?.supported && (
              <>
                <div className="gateway-control">
                  <div>
                    <strong>
                      {desktop.platform === "macos" ? "登录时启动" : "开机启动"}
                    </strong>
                    <p className="muted">
                      登录 {desktop.platform === "macos" ? "macOS" : "Windows"}{" "}
                      后启动{desktop.platform === "macos" ? "菜单栏" : "托盘"}
                      和后台，主窗口保持隐藏。
                    </p>
                  </div>
                  <button
                    className="button"
                    disabled={!!busy}
                    aria-pressed={desktop.autostartEnabled}
                    onClick={async () => {
                      setBusy("设置开机启动");
                      try {
                        const next = await invoke<DesktopStatus>(
                          "desktop_autostart",
                          { enabled: !desktop.autostartEnabled },
                        );
                        setDesktop(next);
                        setNotice({
                          text: next.autostartEnabled
                            ? "已开启开机启动"
                            : "已关闭开机启动",
                        });
                      } catch (error) {
                        setNotice({ text: String(error), error: true });
                      } finally {
                        setBusy("");
                      }
                    }}
                  >
                    {desktop.autostartEnabled
                      ? "已开启 · 关闭"
                      : "已关闭 · 开启"}
                  </button>
                </div>
                <div className="background-note">
                  <strong>关闭窗口后继续运行</strong>
                  <p>
                    {desktop.platform === "macos"
                      ? "关闭窗口后应用留在菜单栏。菜单栏的“显示主窗口”、Dock 图标或再次启动应用都能重新打开窗口。"
                      : "点击关闭会收起到系统托盘。左键点击图标可打开窗口，右键可暂停共享或调整开机启动。"}
                  </p>
                  <p>
                    “退出
                    mcplink”会停止后台并清理共享服务；“只退出界面”保留代理运行。后续
                    Agent 请求仍可按需启动后台。
                  </p>
                  <button
                    className="button small"
                    disabled={!!busy}
                    onClick={async () => {
                      try {
                        await invoke("desktop_menu");
                      } catch (error) {
                        setNotice({ text: String(error), error: true });
                      }
                    }}
                  >
                    {desktop.platform === "macos"
                      ? "打开菜单栏菜单"
                      : "打开托盘菜单"}
                  </button>
                </div>
              </>
            )}
          </div>
        </Modal>
      )}
      {dialog?.kind === "detail" && (
        <Modal
          title={dialog.connection.title}
          wide
          onClose={() => setDialog(undefined)}
        >
          <div className="modal-body">
            <div className="detail-status">
              <Status value={dialog.connection.status} />
              <span className="muted">{time(dialog.connection.checkedAt)}</span>
            </div>
            <dl className="details">
              <div>
                <dt>接入名称</dt>
                <dd>
                  <code>{dialog.connection.name}</code>
                </dd>
              </div>
              <div>
                <dt>类型</dt>
                <dd>
                  {dialog.connection.transport === "http"
                    ? "HTTP MCP"
                    : "本地进程 · STDIO"}
                </dd>
              </div>
              <div>
                <dt>服务</dt>
                <dd>{dialog.connection.endpoint}</dd>
              </div>
              <div>
                <dt>账号</dt>
                <dd>{dialog.connection.accountLabel || "原生服务维护"}</dd>
              </div>
            </dl>
            {dialog.connection.lastError && (
              <div className="warning-banner">
                {dialog.connection.lastError}
              </div>
            )}
            <div className="split-buttons">
              <button
                className="button primary"
                disabled={!!busy}
                onClick={() => void connect([dialog.connection])}
              >
                <ArrowLeftRight size={15} />
                接给 Agent
              </button>
              <button
                className="button"
                disabled={!!busy}
                onClick={() => void probe(dialog.connection)}
              >
                <RefreshCw size={15} />
                检查工具
              </button>
              <button
                className="button"
                disabled={!!busy}
                onClick={async () => {
                  const managed = await manage(dialog.connection);
                  if (managed)
                    setDialog({ kind: "editor", connection: managed });
                }}
              >
                编辑
              </button>
            </div>
            <h3 className="section-label">Agent 接入</h3>
            {ownBindings(dialog.connection).map((binding) => (
              <div
                className="binding-detail"
                key={`${binding.sourceId}:${binding.name}`}
              >
                <div>
                  <strong>{agentLabel(binding.agent)}</strong>
                  <small>
                    {
                      data?.sources.find(
                        (source) => source.id === binding.sourceId,
                      )?.label
                    }{" "}
                    ·{" "}
                    {binding.mode === "shared"
                      ? "通过 mcplink 共享账号"
                      : "直连"}{" "}
                    · {binding.enabled ? "启用" : "停用"}
                    {binding.missing
                      ? " · 来源已移除，可清理绑定"
                      : binding.changed
                        ? " · 外部配置有变化"
                        : binding.needsUpdate
                          ? " · 待同步更新"
                          : ""}
                    {binding.changed &&
                      !binding.missing &&
                      binding.mode === "direct" && (
                        <button
                          className="button small"
                          onClick={async () => {
                            const result = await action(
                              "采用来源更新",
                              "connections.adopt_source",
                              {
                                sourceId: binding.sourceId,
                                name: binding.name,
                              },
                            );
                            if (result) setDialog(undefined);
                          }}
                        >
                          采用来源更新
                        </button>
                      )}
                    {!!binding.overriddenIn?.length && (
                      <span>
                        {" "}
                        · 在 {binding.overriddenIn.length}{" "}
                        个已登记项目来源中被更高作用域覆盖
                      </span>
                    )}
                  </small>
                </div>
                <button
                  className="text-button"
                  disabled={!!busy}
                  onClick={async () => {
                    const managed = await manage(dialog.connection);
                    if (managed)
                      setDialog({ kind: "connect", connections: [managed] });
                  }}
                >
                  管理
                </button>
              </div>
            ))}
            <h3 className="section-label">
              实际工具{" "}
              {tools[dialog.connection.id]
                ? `· ${tools[dialog.connection.id].length}`
                : ""}
            </h3>
            {tools[dialog.connection.id]?.map((tool) => (
              <details className="tool" key={tool.name}>
                <summary>
                  <code>{tool.name}</code>
                  <ChevronDown size={14} />
                </summary>
                <p>{tool.description || "上游未提供描述"}</p>
                <pre>{JSON.stringify(tool.inputSchema, null, 2)}</pre>
              </details>
            ))}
            {!tools[dialog.connection.id] && (
              <p className="muted">
                点击“检查工具”读取当前服务暴露的工具与参数。
              </p>
            )}
            {!dialog.connection.detected &&
              ownBindings(dialog.connection).length === 0 && (
                <button
                  className="text-button danger"
                  disabled={!!busy}
                  onClick={() =>
                    setDialog({
                      kind: "confirm",
                      title: "删除本地连接",
                      message: `删除 ${dialog.connection.title} 的本地记录？已接入的 Agent 需先移除绑定。`,
                      method: "connections.delete",
                      params: { id: dialog.connection.id },
                    })
                  }
                >
                  删除本地连接
                </button>
              )}
          </div>
        </Modal>
      )}
      {dialog?.kind === "confirm" && (
        <Modal title={dialog.title} onClose={() => setDialog(undefined)}>
          <div className="modal-body">
            <p>{dialog.message}</p>
            <footer className="modal-footer">
              <button className="button" onClick={() => setDialog(undefined)}>
                取消
              </button>
              <button
                className="button primary"
                disabled={!!busy}
                onClick={async () => {
                  const result = await action(
                    dialog.title,
                    dialog.method,
                    dialog.params,
                  );
                  if (result) setDialog(undefined);
                }}
              >
                确认
              </button>
            </footer>
          </div>
        </Modal>
      )}
      {dialog?.kind === "connect" && (
        <ConnectDialog
          connections={dialog.connections}
          targetAgent={dialog.targetAgent}
          onManage={manage}
          sources={data?.sources || []}
          clients={localAgents}
          bindings={data?.bindings || []}
          onClose={() => setDialog(undefined)}
          onDone={async () => {
            setSelected([]);
            await load();
          }}
          onNotice={setNotice}
        />
      )}
      {dialog?.kind === "editor" && (
        <Editor
          connection={dialog.connection}
          onClose={() => setDialog(undefined)}
          onDone={async () => {
            await load();
            setDialog(undefined);
          }}
        />
      )}
      {dialog?.kind === "credential" && (
        <CredentialDialog
          connection={dialog.connection}
          onClose={() => setDialog(undefined)}
          onDone={async () => {
            await load();
            setDialog(undefined);
          }}
        />
      )}
    </div>
  );
}

function SourceDialog({
  agent,
  onClose,
  onSaved,
}: {
  agent: Agent;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [scope, setScope] = useState("user");
  const [path, setPath] = useState("");
  const [project, setProject] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const scoped = scope === "local" || scope === "project";
  return (
    <Modal title={`添加 ${agentLabel(agent)} 来源`} onClose={onClose}>
      <form
        className="modal-body"
        onSubmit={async (event) => {
          event.preventDefault();
          setPending(true);
          setError("");
          try {
            await rpc("sources.add", {
              agent,
              scope: scope === "readonly" ? null : scope,
              path: scoped && agent === "claude" ? null : path || null,
              project: scoped ? project : null,
            });
            await onSaved();
          } catch (error) {
            setError((error as Error).message);
          } finally {
            setPending(false);
          }
        }}
      >
        <label className="field">
          <span>配置作用域</span>
          <select
            value={scope}
            onChange={(event) => setScope(event.target.value)}
          >
            <option value="user">User · 所有项目</option>
            {agent === "claude" && (
              <option value="local">Local · 个人项目配置</option>
            )}
            {["claude", "opencode", "zcode", "kimicode"].includes(agent) && (
              <option value="project">Project · 项目配置</option>
            )}
            {agent === "claude" && (
              <option value="readonly">其他 JSON · 只读盘点</option>
            )}
          </select>
        </label>
        {scoped && (
          <label className="field">
            <span>项目目录</span>
            <input
              value={project}
              onChange={(event) => setProject(event.target.value)}
              required
              placeholder="完整目录路径"
            />
            <button
              type="button"
              className="button"
              onClick={async () => {
                const selected = await open({
                  directory: true,
                  multiple: false,
                  title: "选择项目目录",
                });
                if (typeof selected === "string") setProject(selected);
              }}
            >
              选择项目目录
            </button>
          </label>
        )}
        {(!scoped || agent === "opencode") && (
          <label className="field">
            <span>配置文件</span>
            <input
              value={path}
              onChange={(event) => setPath(event.target.value)}
              required={scope === "readonly"}
              placeholder={
                agent === "opencode"
                  ? "留空自动选择 opencode.json(c)"
                  : scope === "user"
                    ? agent === "claude"
                      ? "留空使用当前用户 .claude.json"
                      : agent === "deepseek"
                        ? "留空使用 desktop profile 的 cordis.patch.yml"
                        : "留空使用当前用户的原生 MCP 配置"
                    : "完整 JSON 文件路径"
              }
            />
            <button
              type="button"
              className="button"
              onClick={async () => {
                const selected = await open({
                  multiple: false,
                  filters: [
                    {
                      name: "原生 MCP 配置",
                      extensions:
                        agent === "deepseek"
                          ? ["yml", "yaml"]
                          : ["json", "jsonc"],
                    },
                  ],
                });
                if (typeof selected === "string") setPath(selected);
              }}
            >
              选择配置文件
            </button>
          </label>
        )}
        <p className="field-help">
          {agent === "opencode"
            ? "使用当前 OpenCode 配置格式，支持 JSON/JSONC 及空 MCP 配置，保留注释和其他设置。"
            : agent === "claude"
              ? "User 与 Local 使用 .claude.json 的不同位置；Local 按 Git 项目根目录定位。Project 使用所选目录的 .mcp.json，客户端审批及信任由 Claude Code 维护。插件与托管来源保持只读。"
              : agent === "deepseek"
                ? "按 profile 管理 cordis.patch.yml 中的 MCP 插件，可选择 desktop、web 或其他实际 profile。共享代理写入原生 stdio 入口。"
                : "使用当前原生 MCP 格式，支持配置管理及共享代理。文件仅在应用预览后的变更时写入。"}
        </p>
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
        <footer className="modal-footer">
          <button type="button" className="button" onClick={onClose}>
            取消
          </button>
          <button className="button primary" disabled={pending}>
            {pending && <Loader2 className="spin" size={15} />}添加来源
          </button>
        </footer>
      </form>
    </Modal>
  );
}

function ConnectDialog({
  connections,
  targetAgent,
  onManage,
  sources,
  clients,
  bindings,
  onClose,
  onDone,
  onNotice,
}: {
  connections: Connection[];
  targetAgent?: Agent;
  onManage: (connection: Connection) => Promise<Connection | undefined>;
  sources: Source[];
  clients: LocalAgent[];
  bindings: Binding[];
  onClose: () => void;
  onDone: () => Promise<void>;
  onNotice: (notice: { text: string; error?: boolean }) => void;
}) {
  const [targets, setTargets] = useState<string[]>(() =>
    targetAgent
      ? sources
          .filter(
            (source) =>
              source.agent === targetAgent &&
              !source.readOnly &&
              clients.some(
                (client) => client.id === targetAgent && client.configurable,
              ),
          )
          .map((source) => source.id)
      : [],
  );
  const [mode, setMode] = useState("direct");
  const [operation, setOperation] = useState("upsert");
  const [plans, setPlans] = useState<Plan[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [results, setResults] =
    useState<{ sourceId: string; status: string; message: string }[]>();
  const isAvailable = (source: Source) =>
    !source.readOnly &&
    clients.some((client) => client.id === source.agent && client.configurable);
  useEffect(() => {
    const available = new Set(
      sources
        .filter(
          (source) =>
            !source.readOnly &&
            clients.some(
              (client) => client.id === source.agent && client.configurable,
            ),
        )
        .map((source) => source.id),
    );
    setTargets((previous) => previous.filter((id) => available.has(id)));
    setPlans((previous) =>
      previous.some((plan) =>
        plan.changes.some((change) => !available.has(change.sourceId)),
      )
        ? []
        : previous,
    );
  }, [clients, sources]);
  const preview = async () => {
    setPending(true);
    setError("");
    try {
      const managed: Connection[] = [];
      for (const connection of connections) {
        const result = await onManage(connection);
        if (!result) throw new Error("服务暂时无法纳入管理，请核对后重试");
        managed.push(result);
      }
      const plan = await rpc<Plan>("changes.preview", {
        connectionIds: managed.map((connection) => connection.id),
        targetSourceIds: targets,
        mode,
        action: operation,
      });
      setPlans([plan]);
    } catch (error) {
      setError((error as Error).message);
    } finally {
      setPending(false);
    }
  };
  const apply = async () => {
    setPending(true);
    setError("");
    try {
      const all = [];
      for (const plan of plans) {
        const result = await rpc<{
          results: { sourceId: string; status: string; message: string }[];
        }>("changes.apply", { planId: plan.planId });
        all.push(...result.results);
      }
      setResults(all);
      await onDone();
      onNotice({
        text: all.every((result) => result.status === "applied")
          ? "配置已写入，请在对应 Agent 重新加载 MCP"
          : "部分目标未完成，请核对每项结果",
        error: all.some((result) => result.status !== "applied"),
      });
    } catch (error) {
      setError((error as Error).message);
    } finally {
      setPending(false);
    }
  };
  return (
    <Modal
      title={
        results ? "接入结果" : plans.length ? "核对配置变更" : "接给 Agent"
      }
      wide
      onClose={onClose}
    >
      <div className="modal-body">
        <div className="connection-summary">
          <Server size={20} />
          <div>
            <strong>
              {connections.length === 1
                ? connections[0].title
                : `${connections.length} 项服务`}
            </strong>
            <p>{connections.map((connection) => connection.name).join("、")}</p>
          </div>
        </div>
        {!plans.length && !results && (
          <>
            <label className="field">
              <span>操作</span>
              <select
                value={operation}
                onChange={(event) => setOperation(event.target.value)}
              >
                <option value="upsert">接入 / 更新配置</option>
                <option value="disable">停用所选 Agent 的服务</option>
                <option value="enable">启用所选 Agent 的服务</option>
                <option value="remove">移除所选 Agent 的配置</option>
              </select>
            </label>
            {operation === "upsert" && (
              <div className="mode-choices">
                <label className={mode === "direct" ? "chosen" : ""}>
                  <input
                    type="radio"
                    name="mode"
                    checked={mode === "direct"}
                    onChange={() => setMode("direct")}
                  />
                  <strong>直连</strong>
                  <small>各 Agent 直接启动或连接服务</small>
                </label>
                <label className={mode === "shared" ? "chosen" : ""}>
                  <input
                    type="radio"
                    name="mode"
                    checked={mode === "shared"}
                    onChange={() => setMode("shared")}
                  />
                  <strong>通过 mcplink 共享</strong>
                  <small>共用托管账号与上游连接</small>
                </label>
              </div>
            )}
            <h3 className="section-label">目标 Agent 与配置位置</h3>
            {sources.map((source) => (
              <label className="target" key={source.id}>
                <input
                  type="checkbox"
                  checked={targets.includes(source.id)}
                  disabled={!isAvailable(source)}
                  onChange={(event) =>
                    setTargets((previous) =>
                      event.target.checked
                        ? [...previous, source.id]
                        : previous.filter((id) => id !== source.id),
                    )
                  }
                />
                <span className={`agent-avatar ${source.agent}`}>
                  {agentLabel(source.agent)[0]}
                </span>
                <div>
                  <strong>
                    {agentLabel(source.agent)} · {source.label}
                  </strong>
                  <small>
                    {source.path}
                    {!clients.some(
                      (client) =>
                        client.id === source.agent && client.installed,
                    )
                      ? " · 未安装，不可配置"
                      : source.readOnly
                        ? " · 只读，请在原客户端维护"
                        : ""}
                  </small>
                </div>
                {bindings.some(
                  (binding) =>
                    binding.sourceId === source.id &&
                    connections.some(
                      (connection) => connection.id === binding.connectionId,
                    ),
                ) && <span className="muted">已有接入</span>}
              </label>
            ))}
          </>
        )}
        {plans.map((plan) => (
          <div key={plan.planId}>
            <h3>{plan.connection.title}</h3>
            {plan.changes.map((change) => (
              <div className="change-preview" key={change.id}>
                <div>
                  <span className={`agent-avatar ${change.agent}`}>
                    {agentLabel(change.agent)[0]}
                  </span>
                  <strong>
                    {agentLabel(change.agent)} · {change.sourceLabel}
                  </strong>
                  <span className="pill">
                    {change.action === "remove"
                      ? "移除配置"
                      : change.action === "disable"
                        ? "停用"
                        : change.action === "enable"
                          ? "启用"
                          : change.mode === "shared"
                            ? "共享账号"
                            : "直连"}
                  </span>
                </div>
                <p>{change.reload}</p>
                <details>
                  <summary>查看脱敏配置变化</summary>
                  <pre>
                    {JSON.stringify(
                      { before: change.before, after: change.after },
                      null,
                      2,
                    )}
                  </pre>
                </details>
                <details>
                  <summary>配置位置与接入名称</summary>
                  <code>{change.path}</code>
                  <code>{change.name}</code>
                </details>
              </div>
            ))}
          </div>
        ))}
        {results?.map((result, index) => (
          <div className="result-row" key={index}>
            <Status value={result.status} />
            <p>
              {sources.find((source) => source.id === result.sourceId)?.agent} ·{" "}
              {result.message}
            </p>
          </div>
        ))}
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
        <footer className="modal-footer">
          {plans.length > 0 && !results && (
            <button
              className="button"
              disabled={pending}
              onClick={() => setPlans([])}
            >
              返回调整
            </button>
          )}
          <button className="button" onClick={onClose}>
            {results ? "完成" : "取消"}
          </button>
          {!results && (
            <button
              className="button primary"
              disabled={pending || !targets.length}
              onClick={() => void (plans.length ? apply() : preview())}
            >
              {pending && <Loader2 size={15} className="spin" />}
              {plans.length ? "应用配置" : "预览变更"}
            </button>
          )}
        </footer>
      </div>
    </Modal>
  );
}

function Editor({
  connection,
  onClose,
  onDone,
}: {
  connection?: Connection;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const [name, setName] = useState(connection?.name || "");
  const [title, setTitle] = useState(connection?.title || "");
  const [type, setType] = useState("http");
  const [url, setUrl] = useState("");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [account, setAccount] = useState(connection?.accountLabel || "");
  const [token, setToken] = useState("");
  const [environment, setEnvironment] = useState("{}");
  const [customHeaders, setCustomHeaders] = useState("{}");
  const [cwd, setCwd] = useState("");
  const [definition, setDefinition] = useState<Definition>();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (connection) {
      setPending(true);
      rpc<{ definition: Definition }>("connections.detail", {
        id: connection.id,
      })
        .then((result) => {
          setDefinition(result.definition);
          setType(result.definition.url ? "http" : "stdio");
          setUrl(result.definition.url || "");
          setCommand(result.definition.command || "");
          setArgs(result.definition.args.join("\n"));
          setEnvironment(JSON.stringify(result.definition.env, null, 2));
          setCustomHeaders(JSON.stringify(result.definition.headers, null, 2));
          setCwd(result.definition.cwd || "");
        })
        .catch((error) => setError(error.message))
        .finally(() => setPending(false));
    }
  }, [connection]);
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setPending(true);
    setError("");
    try {
      const next: Definition = {
        args: [],
        env: {},
        headers: {},
        enabled: true,
        oauth: false,
        ...definition,
        command: type === "stdio" ? command : undefined,
        url: type === "http" ? url : undefined,
      };
      if (type === "stdio") {
        next.args = args.split("\n").filter(Boolean);
        next.headers = {};
        next.oauth = false;
        next.clientId = undefined;
        next.clientSecret = undefined;
        next.oauthScopes = [];
        next.oauthCallbackPort = undefined;
        next.cwd = cwd || undefined;
        next.env = JSON.parse(environment);
      } else {
        next.args = [];
        next.env = {};
        next.cwd = undefined;
        next.headers = JSON.parse(customHeaders);
      }
      for (const map of [next.env, next.headers]) {
        if (
          !map ||
          Array.isArray(map) ||
          typeof map !== "object" ||
          Object.values(map).some((v) => typeof v !== "string")
        )
          throw new Error("环境变量与请求头需要键值均为字符串的 JSON 对象");
      }
      if (token && type === "http") {
        next.headers = Object.fromEntries(
          Object.entries(next.headers).filter(
            ([key]) => key.toLowerCase() !== "authorization",
          ),
        );
        next.headers.Authorization = token;
        next.oauth = false;
      }
      await rpc("connections.save", {
        id: connection?.id,
        name,
        title: title || name,
        accountLabel: account || "用户标签",
        definition: next,
      });
      await onDone();
    } catch (error) {
      setError((error as Error).message);
    } finally {
      setPending(false);
    }
  };
  return (
    <Modal
      title={connection ? "编辑服务连接" : "添加 MCP 服务"}
      onClose={onClose}
    >
      <form className="modal-body" onSubmit={(event) => void save(event)}>
        <div className="form-grid">
          <label className="field">
            <span>显示名称</span>
            <input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              placeholder="例如 腾讯文档 · 工作账号"
            />
          </label>
          <label className="field">
            <span>接入名称</span>
            <input
              required
              pattern="[A-Za-z0-9_-]{1,80}"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="tencent_docs"
            />
          </label>
        </div>
        <label className="field">
          <span>服务类型</span>
          <select
            value={type}
            onChange={(event) => setType(event.target.value)}
          >
            <option value="http">HTTP MCP</option>
            <option value="stdio">本地进程 · STDIO</option>
          </select>
        </label>
        {type === "http" ? (
          <>
            <label className="field">
              <span>MCP 地址</span>
              <input
                required
                value={url}
                onChange={(event) => setUrl(event.target.value)}
                placeholder="https://example.com/mcp"
              />
            </label>
            <label className="field">
              <span>Authorization 值</span>
              <input
                type="password"
                autoComplete="off"
                value={token}
                onChange={(event) => setToken(event.target.value)}
                placeholder={
                  connection
                    ? "留空保留现有认证"
                    : "免登录服务可留空，按服务要求填写完整值"
                }
              />
            </label>
            <label className="field">
              <span>账号标签</span>
              <input
                value={account}
                onChange={(event) => setAccount(event.target.value)}
                placeholder="工作账号（用于本机识别）"
              />
            </label>
          </>
        ) : (
          <>
            <label className="field">
              <span>启动命令</span>
              <input
                required
                value={command}
                onChange={(event) => setCommand(event.target.value)}
                placeholder="npx 或可执行文件完整路径"
              />
            </label>
            <label className="field">
              <span>启动参数，每行一个</span>
              <textarea
                value={args}
                onChange={(event) => setArgs(event.target.value)}
                placeholder={
                  "-y\n@modelcontextprotocol/server-filesystem\n/path/to/folder"
                }
                rows={4}
              />
            </label>
          </>
        )}
        <details className="oauth-option">
          <summary>更多连接设置</summary>
          {type === "stdio" ? (
            <>
              <label className="field">
                <span>工作目录</span>
                <input
                  value={cwd}
                  onChange={(e) => setCwd(e.target.value)}
                  placeholder="可留空"
                />
              </label>
              <label className="field">
                <span>环境变量 · JSON</span>
                <textarea
                  rows={4}
                  value={environment}
                  onChange={(e) => setEnvironment(e.target.value)}
                  spellCheck={false}
                />
              </label>
            </>
          ) : (
            <label className="field">
              <span>自定义请求头 · JSON</span>
              <textarea
                rows={4}
                value={customHeaders}
                onChange={(e) => setCustomHeaders(e.target.value)}
                spellCheck={false}
              />
            </label>
          )}
          <p className="field-help">
            •••••••• 表示保留已存值；变量引用可使用 $&#123;env:NAME&#125;。
          </p>
        </details>
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
        <footer className="modal-footer">
          <button type="button" className="button" onClick={onClose}>
            取消
          </button>
          <button className="button primary" disabled={pending}>
            {pending && <Loader2 className="spin" size={15} />}保存连接
          </button>
        </footer>
      </form>
    </Modal>
  );
}

function CredentialDialog({
  connection,
  onClose,
  onDone,
}: {
  connection: Connection;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const [token, setToken] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [oauth, setOauth] = useState(false);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [scopes, setScopes] = useState("");
  const [callbackPort, setCallbackPort] = useState("");
  const [oauthLoaded, setOauthLoaded] = useState(false);
  const [oauthSaved, setOauthSaved] = useState(false);
  useEffect(() => {
    let active = true;
    void rpc<{ definition: Definition }>("connections.detail", {
      id: connection.id,
    })
      .then(({ definition }) => {
        if (!active) return;
        setClientId(definition.clientId || "");
        setClientSecret(definition.clientSecret || "");
        setScopes((definition.oauthScopes || []).join(" "));
        setCallbackPort(definition.oauthCallbackPort?.toString() || "");
        setOauthLoaded(true);
      })
      .catch((error) => {
        if (active) setError(error.message);
      });
    return () => {
      active = false;
    };
  }, [connection.id]);
  const saveOauth = async () => {
    const result = await rpc<{ definition: Definition }>("oauth.configure", {
      id: connection.id,
      settings: {
        clientId: clientId.trim() || null,
        clientSecret: clientSecret || null,
        scopes: scopes.trim() ? scopes.trim().split(/\s+/) : [],
        callbackPort: callbackPort ? Number(callbackPort) : null,
      },
    });
    setClientSecret(result.definition.clientSecret || "");
    setOauthSaved(true);
  };
  const update = async (event: React.FormEvent) => {
    event.preventDefault();
    setPending(true);
    setError("");
    try {
      await rpc("credentials.update", {
        id: connection.id,
        header: "Authorization",
        value: token,
      });
      await onDone();
    } catch (error) {
      setError((error as Error).message);
    } finally {
      setPending(false);
    }
  };
  return (
    <Modal title={`${connection.title} · 认证`} onClose={onClose}>
      <form className="modal-body" onSubmit={(event) => void update(event)}>
        <p className="muted">
          认证保存在系统凭据库，使用这项共享连接的 Agent 会共用更新后的值。
        </p>
        {connection.endpoint.includes("docs.qq.com") && (
          <button
            className="button full"
            type="button"
            onClick={() =>
              void rpc("browser.open", {
                url: "https://docs.qq.com/open/auth/mcp.html",
              }).catch((error) => setError(error.message))
            }
          >
            打开腾讯文档授权页面
            <ArrowRight size={14} />
          </button>
        )}
        <label className="field">
          <span>Authorization 完整值</span>
          <input
            required
            type="password"
            autoComplete="off"
            value={token}
            onChange={(event) => setToken(event.target.value)}
            placeholder="粘贴服务提供的令牌或完整 Bearer 值"
          />
        </label>
        <p className="field-help">
          按上游要求原样保存，更新后可检查连接并查看实际工具。
        </p>
        <details className="oauth-option">
          <summary>服务使用 OAuth 浏览器授权</summary>
          <p className="muted">
            适用于提供标准 MCP OAuth
            元数据的服务；可使用动态注册或你自己的预注册应用。
          </p>
          <label className="field">
            <span>Client ID · 预注册应用</span>
            <input
              value={clientId}
              onChange={(event) => {
                setClientId(event.target.value);
                setOauthSaved(false);
              }}
              placeholder="支持动态注册时可留空"
              autoComplete="off"
            />
          </label>
          <label className="field">
            <span>客户端机密 · 按注册应用要求填写</span>
            <input
              type="password"
              value={clientSecret}
              onChange={(event) => {
                setClientSecret(event.target.value);
                setOauthSaved(false);
              }}
              autoComplete="off"
              placeholder="保存在系统凭据库；清空可删除已存值"
            />
          </label>
          <label className="field">
            <span>授权 scope · 用空格分隔</span>
            <input
              value={scopes}
              onChange={(event) => {
                setScopes(event.target.value);
                setOauthSaved(false);
              }}
              placeholder="留空使用服务建议的 scope"
            />
          </label>
          <label className="field">
            <span>本地回调端口 · 可选</span>
            <input
              type="number"
              min={1}
              max={65535}
              value={callbackPort}
              onChange={(event) => {
                setCallbackPort(event.target.value);
                setOauthSaved(false);
              }}
              placeholder="留空自动分配，路径为 /callback"
            />
          </label>
          <p className="field-help">
            •••••••• 表示保留已存机密。更换 OAuth
            应用或权限后需要重新授权；授权准备失败会保留原有认证。
          </p>
          <button
            className="button"
            type="button"
            disabled={pending || !oauthLoaded}
            onClick={async () => {
              setPending(true);
              setError("");
              try {
                await saveOauth();
                await onDone();
              } catch (error) {
                setError((error as Error).message);
              } finally {
                setPending(false);
              }
            }}
          >
            保存 OAuth 设置
          </button>
          {oauthSaved && <p className="muted">OAuth 客户端设置已保存。</p>}
          <button
            className="button"
            type="button"
            disabled={pending || !oauthLoaded}
            onClick={async () => {
              setPending(true);
              setError("");
              try {
                await saveOauth();
                const result = await rpc<{ url: string }>("oauth.start", {
                  id: connection.id,
                });
                await rpc("browser.open", { url: result.url });
                setOauth(true);
              } catch (error) {
                setError((error as Error).message);
              } finally {
                setPending(false);
              }
            }}
          >
            <KeyRound size={15} />
            在浏览器授权
          </button>
          {oauth && (
            <p className="muted">
              已打开浏览器，完成授权后返回并刷新账号状态。
            </p>
          )}
        </details>
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
        <footer className="modal-footer">
          <button type="button" className="button" onClick={onClose}>
            关闭
          </button>
          <button className="button primary" disabled={pending || !token}>
            {pending && <Loader2 className="spin" size={15} />}更新认证
          </button>
        </footer>
      </form>
    </Modal>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
