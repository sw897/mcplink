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
import {
  Agent,
  agents,
  Binding,
  Change,
  changeLabel,
  restoreLabel,
  Json,
  Connection,
  Operation,
  Snapshot,
  DesktopStatus,
  Tool,
  Dialog,
  agentLabel,
  time,
  rpc,
} from "./contracts";
import { Status, ModalNotice, Modal } from "./Modal";
import { SourceDialog } from "./SourceDialog";
import { ConnectDialog, ChangeDiff } from "./ConnectDialog";
import { Editor, CredentialDialog } from "./ConnectionEditor";
import "./style.css";

const toolKey = (connection: Connection) =>
  JSON.stringify([connection.id, connection.revision, connection.checkedAt]);
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
  const [compact, setCompact] = useState(
    () => localStorage.getItem("mcplink.serviceLayout") !== "cards",
  );
  const [selected, setSelected] = useState<string[]>([]);
  const [dialog, setDialog] = useState<Dialog>();
  const [busy, setBusy] = useState("");
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [failure, setFailure] = useState("");
  const [olderHistory, setOlderHistory] = useState<Operation[]>([]);
  const [historyEnd, setHistoryEnd] = useState(false);
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
      const validKeys = new Set(
        next.connections
          .filter((c) => c.active && c.status === "ready")
          .map(toolKey),
      );
      setTools((previous) =>
        Object.fromEntries(
          Object.entries(previous).filter(([key]) => validKeys.has(key)),
        ),
      );
      void invoke<DesktopStatus>("desktop_status")
        .then(setDesktop)
        .catch((error) => {
          setNotice({ text: String(error), error: true });
        });
      setDialog((previous) => {
        if (previous?.kind === "detail") {
          const connection = next.connections.find(
            (c) => c.id === previous.connection.id,
          );
          return connection ? { ...previous, connection } : undefined;
        }
        return previous?.kind === "addSource" &&
          !next.agents.some(
            (client) => client.id === previous.agent && client.configurable,
          )
          ? undefined
          : previous;
      });
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
    setNotice(undefined);
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
  const connect = (
    connections: Connection[],
    targetAgent?: Agent,
    binding?: Binding,
  ) => {
    setDialog(
      connections.some((c) => c.detected)
        ? {
            kind: "adopt",
            intent: "connect",
            connections,
            targetAgent,
            binding,
          }
        : { kind: "connect", connections, targetAgent, binding },
    );
  };
  const probe = async (connection: Connection) => {
    if (connection.detected) {
      setDialog({ kind: "adopt", intent: "probe", connections: [connection] });
      return;
    }
    const result = await action<{
      tools: Tool[];
      connection: Connection;
      error?: unknown;
    }>("检查连接", "connections.probe", { id: connection.id });
    if (result) {
      setTools((previous) => ({
        ...previous,
        [toolKey(result.connection)]: result.tools || [],
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
  const visibleTools =
    dialog?.kind === "detail" &&
    dialog.connection.active &&
    dialog.connection.status === "ready"
      ? tools[toolKey(dialog.connection)]
      : undefined;
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

  const addSource = async (agent: Agent) => {
    if (agent !== "codex" && agent !== "hermes") {
      setDialog({ kind: "addSource", agent });
      return;
    }
    try {
      const path = await open({
        title: `选择 ${agentLabel(agent)} 配置`,
        multiple: false,
        filters: [
          {
            name: "配置文件",
            extensions: agent === "codex" ? ["toml"] : ["yaml", "yml"],
          },
        ],
      });
      if (typeof path === "string")
        await action("添加来源", "sources.add", { agent, path });
    } catch (error) {
      setNotice({ text: String(error), error: true });
    }
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
            missing ? void addSource(agent) : void connect([connection], agent)
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

  const toast = notice && (
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
  );
  const content = (
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
            onClick={async () => {
              setOlderHistory([]);
              setHistoryEnd(false);
              setDialog({ kind: "history" });
              if (data?.history.some((op) => !op.kind))
                await action("读取历史", "changes.history");
            }}
          >
            <History size={16} />
            变更与恢复
          </button>
          <div className="runtime">
            <i
              className={
                failure || !data || data.gateway.paused ? "paused" : ""
              }
            />
            <span>
              本机管理
              {failure
                ? "连接异常"
                : !data
                  ? "连接中"
                  : data.gateway.paused
                    ? " · 共享已暂停"
                    : "已连接"}
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
                  <button
                    className="button small density-toggle"
                    aria-pressed={compact}
                    onClick={() => {
                      localStorage.setItem(
                        "mcplink.serviceLayout",
                        compact ? "cards" : "compact",
                      );
                      setCompact(!compact);
                    }}
                  >
                    紧凑列表
                  </button>
                  <label className="search">
                    <Search size={16} />
                    <input
                      placeholder="搜索服务或账号"
                      aria-label="搜索服务或账号"
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
                <section
                  className={compact ? "service-list compact" : "service-list"}
                  aria-label="MCP 服务列表"
                >
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
                            role="group"
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
                                              binding.enabled &&
                                              !binding.missing &&
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
                        {query || filter !== "all"
                          ? "没有匹配的服务"
                          : "把已有的 MCP 接到一起"}
                      </h3>
                      <p>
                        {query || filter !== "all"
                          ? "尝试其他名称或筛选条件。"
                          : "支持的配置会自动显示，也可以添加服务或配置位置。"}
                      </p>
                      {!query && filter === "all" && (
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
                            {connection.authKind === "none"
                              ? "本机未配置独立凭据"
                              : connection.detected
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
                          onClick={() =>
                            setDialog(
                              connection.detected &&
                                connection.transport === "http"
                                ? {
                                    kind: "adopt",
                                    intent: "credential",
                                    connections: [connection],
                                  }
                                : {
                                    kind:
                                      connection.transport === "http"
                                        ? "credential"
                                        : "editor",
                                    connection,
                                  },
                            )
                          }
                        >
                          {connection.transport === "stdio"
                            ? "编辑启动配置"
                            : connection.authKind === "none"
                              ? "设置认证"
                              : "更新认证"}
                        </button>
                        {!connection.active && !connection.detected && (
                          <button
                            className="button small"
                            onClick={() =>
                              setDialog({
                                kind: "confirm",
                                title: "重新连接",
                                message: `重新启用 ${connection.title}（${connection.endpoint}）后，已共享的 Agent 可再次请求该服务。`,
                                method: "connections.reconnect",
                                params: { id: connection.id },
                              })
                            }
                          >
                            重新连接
                          </button>
                        )}
                        {!connection.detected &&
                          connection.active &&
                          connection.authKind !== "none" && (
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
                        onClick={() => void addSource(currentAgent.id)}
                      >
                        <FolderPlus size={14} />
                        添加来源
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
                          {currentAgent.detected ? "已找到配置文件。" : ""}
                        </p>
                        <button
                          className="button small"
                          disabled={!!busy}
                          onClick={async () => {
                            try {
                              const path = await open({
                                multiple: false,
                                title: `选择 ${currentAgent.label} 的 CLI 可执行文件`,
                              });
                              if (typeof path === "string")
                                await action(
                                  "核验客户端程序",
                                  "agents.launcher",
                                  { agent: currentAgent.id, path },
                                );
                            } catch (e) {
                              setNotice({ text: String(e), error: true });
                            }
                          }}
                        >
                          选择已安装的 CLI
                        </button>
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
      {!dialog && toast}
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
                  onClick={() => void addSource(agent)}
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
              [
                ...data.history,
                ...olderHistory.filter(
                  (op) => !data.history.some((recent) => recent.id === op.id),
                ),
              ].map((operation) => (
                <div className="history-row" key={operation.id}>
                  <Clock3 size={17} />
                  <div>
                    <strong>
                      {changeLabel(operation.kind)} · {operation.name}
                    </strong>
                    {data.sources
                      .filter((s) => s.id === operation.sourceId)
                      .map((s) => (
                        <p key={s.id}>
                          {agentLabel(s.agent)} · {s.label} ·{" "}
                          {time(operation.createdAt)}
                          <br />
                          <code className="source-path">{s.path}</code>
                        </p>
                      ))}
                    {operation.status === "applied" && (
                      <small>{restoreLabel(operation.restoreEffect)}</small>
                    )}
                    {operation.error && (
                      <small className="inline-error">{operation.error}</small>
                    )}
                  </div>
                  <Status
                    value={operation.status}
                    label={
                      operation.status === "applied" &&
                      operation.kind === "cleanup"
                        ? "已清理"
                        : undefined
                    }
                  />
                  {operation.status === "pending" && (
                    <button
                      className="button small"
                      disabled={!!busy}
                      onClick={async () => {
                        const result = await action<{ remaining: number }>(
                          "核对写入",
                          "changes.reconcile",
                        );
                        if (result)
                          setNotice({
                            text: result.remaining
                              ? "仍有待核对的写入，请查看记录中的原因"
                              : "已核对写入记录",
                            error: result.remaining > 0,
                          });
                      }}
                    >
                      重新核对
                    </button>
                  )}
                  {["applied", "failed"].includes(operation.status) &&
                    operation.restoreEffect !== "none" && (
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
                          const result = await action<{
                            change?: Change;
                            effect: string;
                          }>("预览恢复", "changes.preview_restore", {
                            operationId: operation.id,
                          });
                          if (result?.change)
                            setDialog({
                              kind: "restore",
                              operation,
                              change: result.change,
                              effect: result.effect,
                            });
                        }}
                      >
                        预览恢复
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
            {data?.history.length === 25 && !historyEnd && (
              <button
                className="button"
                disabled={!!busy}
                onClick={async () => {
                  setBusy("读取历史");
                  try {
                    const before = (olderHistory.at(-1) || data.history.at(-1))!
                      .cursor;
                    const next = await rpc<Operation[]>("changes.history", {
                      before,
                    });
                    setOlderHistory([...olderHistory, ...next]);
                    setHistoryEnd(next.length < 25);
                  } catch (e) {
                    setNotice({ text: String(e), error: true });
                  } finally {
                    setBusy("");
                  }
                }}
              >
                查看更早的记录
              </button>
            )}
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
                onClick={() =>
                  setDialog({ kind: "editor", connection: dialog.connection })
                }
              >
                {dialog.connection.detected ? "编辑并纳入托管" : "编辑"}
              </button>
              {dialog.connection.detected && (
                <button
                  className="button"
                  disabled={!!busy}
                  onClick={() =>
                    setDialog({
                      kind: "adopt",
                      intent: "detail",
                      connections: [dialog.connection],
                    })
                  }
                >
                  纳入托管
                </button>
              )}
              {!dialog.connection.active && (
                <button
                  className="button"
                  onClick={() =>
                    setDialog({
                      kind: "confirm",
                      title: "重新连接",
                      message: `重新启用 ${dialog.connection.title}（${dialog.connection.endpoint}）后，已共享的 Agent 可再次请求该服务。`,
                      method: "connections.reconnect",
                      params: { id: dialog.connection.id },
                    })
                  }
                >
                  重新连接
                </button>
              )}
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
                    {binding.unavailable
                      ? " · 来源不可读取，可解除绑定"
                      : binding.missing
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
                  disabled={
                    !!busy ||
                    binding.unavailable ||
                    data?.sources.find((s) => s.id === binding.sourceId)
                      ?.readOnly
                  }
                  onClick={() =>
                    connect([dialog.connection], binding.agent, binding)
                  }
                >
                  管理
                </button>
                {!dialog.connection.detected && (
                  <button
                    className="text-button"
                    disabled={!!busy}
                    onClick={() =>
                      setDialog({
                        kind: "confirm",
                        title: "解除托管绑定",
                        message: `保留 ${agentLabel(binding.agent)} 配置中的 ${binding.name}，仅解除 mcplink 的绑定记录。${binding.mode === "shared" ? "原配置仍引用此连接，删除连接后该入口将失效。" : ""}`,
                        method: "connections.detach",
                        params: {
                          id: dialog.connection.id,
                          sourceId: binding.sourceId,
                          name: binding.name,
                        },
                      })
                    }
                  >
                    解除绑定
                  </button>
                )}
              </div>
            ))}
            <h3 className="section-label">
              实际工具 {visibleTools ? `· ${visibleTools.length}` : ""}
            </h3>
            {visibleTools?.map((tool) => (
              <details className="tool" key={tool.name}>
                <summary>
                  <code>{tool.name}</code>
                  <ChevronDown size={14} />
                </summary>
                <p>{tool.description || "上游未提供描述"}</p>
                <pre>{JSON.stringify(tool.inputSchema, null, 2)}</pre>
              </details>
            ))}
            {!visibleTools && (
              <p className="muted">
                {dialog.connection.active
                  ? "点击“检查工具”读取当前服务暴露的工具与参数。"
                  : "重新连接并检查后，可查看当前账号的工具。"}
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
      {dialog?.kind === "adopt" && (
        <Modal title="纳入托管" onClose={() => setDialog(undefined)}>
          <div className="modal-body">
            <p>
              继续前，将下列来源的连接设置和认证保存到
              mcplink，并与原配置建立绑定。
            </p>
            <ul>
              {dialog.connections
                .filter((c) => c.detected)
                .map((c) => (
                  <li key={c.id}>
                    {c.title} ·{" "}
                    {data?.sources.find((s) => s.id === c.sourceId)?.label}
                  </li>
                ))}
            </ul>
            <footer className="modal-footer">
              <button
                className="button"
                disabled={!!busy}
                onClick={() => setDialog(undefined)}
              >
                取消
              </button>
              <button
                className="button primary"
                disabled={!!busy}
                onClick={async () => {
                  const managed: Connection[] = [];
                  for (const c of dialog.connections) {
                    const result = c.detected
                      ? await action<Connection>(
                          "纳入托管",
                          "connections.import",
                          { sourceId: c.sourceId, name: c.name },
                        )
                      : c;
                    if (!result) return;
                    managed.push(result);
                  }
                  if (dialog.intent === "probe") await probe(managed[0]);
                  else if (dialog.intent === "credential")
                    setDialog({ kind: "credential", connection: managed[0] });
                  else if (dialog.intent === "detail")
                    setDialog({ kind: "detail", connection: managed[0] });
                  else connect(managed, dialog.targetAgent, dialog.binding);
                }}
              >
                纳入托管并继续
              </button>
            </footer>
          </div>
        </Modal>
      )}
      {dialog?.kind === "restore" && (
        <Modal
          title="核对恢复内容"
          onClose={() => setDialog({ kind: "history" })}
        >
          <div className="modal-body">
            <h3>
              {restoreLabel(dialog.effect)} · {dialog.change.name}
            </h3>
            <p>
              {agentLabel(dialog.change.agent)} · {dialog.change.sourceLabel}
            </p>
            <code className="source-path">{dialog.change.path}</code>
            {dialog.change.after && (
              <p>
                {dialog.change.mode === "shared"
                  ? "恢复共享入口"
                  : "恢复直连配置，凭据可能以明文写入该文件"}
              </p>
            )}
            <ChangeDiff change={dialog.change} />
            <footer className="modal-footer">
              <button
                className="button"
                onClick={() => setDialog({ kind: "history" })}
              >
                返回记录
              </button>
              <button
                className="button primary"
                disabled={!!busy}
                onClick={async () => {
                  if (
                    await action("恢复配置", "changes.restore", {
                      operationId: dialog.operation.id,
                    })
                  ) {
                    setOlderHistory([]);
                    setHistoryEnd(false);
                    setDialog({ kind: "history" });
                    setNotice({ text: "配置已恢复，请在对应 Agent 重新加载" });
                  }
                }}
              >
                确认恢复
              </button>
            </footer>
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
          binding={dialog.binding}
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
          sharedAgents={
            dialog.connection
              ? [
                  ...new Set(
                    ownBindings(dialog.connection)
                      .filter((b) => b.mode === "shared" && !b.missing)
                      .map((b) => agentLabel(b.agent)),
                  ),
                ]
              : []
          }
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
  return <ModalNotice.Provider value={toast}>{content}</ModalNotice.Provider>;
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
