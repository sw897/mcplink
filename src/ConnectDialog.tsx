import { useEffect, useMemo, useRef, useState } from "react";
import { Loader2, Server } from "lucide-react";
import {
  Agent,
  LocalAgent,
  Connection,
  Source,
  Binding,
  Plan,
  Change,
  ImportConfig,
  changeLabel,
  agentLabel,
  rpc,
} from "./contracts";
import { Status, Modal } from "./Modal";
import { ImportConfigCard } from "./ImportConfigCard";

export function ChangeDiff({ change }: { change: Change }) {
  const names: Record<string, string> = {
    url: "服务地址",
    args: "启动参数",
    command: "启动命令",
    cwd: "工作目录",
    clientSecret: "OAuth 客户端机密",
    enabled: "启用状态",
    native: "客户端专属设置",
    oauth: "OAuth 认证",
    includeTools: "启用的工具",
    excludeTools: "排除的工具",
  };
  return (
    <>
      <ul className="field-changes">
        {change.fields.map((item) => (
          <li key={JSON.stringify([item.field, item.key])}>
            {item.key
              ? (item.field === "env" ? "环境变量 " : "请求头 ") + item.key
              : names[item.field] || item.field}
            {" · "}
            {item.kind === "update" ? "已更换" : changeLabel(item.kind)}
          </li>
        ))}
      </ul>
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
    </>
  );
}

export function ConnectDialog({
  connections,
  targetAgent,
  binding,
  sources,
  clients,
  bindings,
  onClose,
  onDone,
  onNotice,
}: {
  connections: Connection[];
  targetAgent?: Agent;
  binding?: Binding;
  sources: Source[];
  clients: LocalAgent[];
  bindings: Binding[];
  onClose: () => void;
  onDone: () => Promise<void>;
  onNotice: (notice: { text: string; error?: boolean }) => void;
}) {
  const [targets, setTargets] = useState<string[]>(() => {
    if (binding) return [binding.sourceId];
    if (
      clients.some(
        (c) =>
          c.id === targetAgent &&
          c.supportsImport &&
          (c.management === "manual" || !c.installed),
      )
    )
      return [`import:${targetAgent}`];
    const available = sources.filter(
      (s) =>
        s.agent === targetAgent &&
        !s.readOnly &&
        clients.some((c) => c.id === targetAgent && c.configurable),
    );
    return available.length === 1 ? [available[0].id] : [];
  });
  const aggregate = connections.some((c) => c.members);
  const [mode, setMode] = useState(
    aggregate ? "shared" : binding?.mode || "shared",
  );
  const [replaceShared, setReplaceShared] = useState(false);
  const [operation, setOperation] = useState("upsert");
  const [plans, setPlans] = useState<Plan[]>([]);
  const [imports, setImports] = useState<ImportConfig[]>([]);
  const hasPreview = plans.length > 0 || imports.length > 0;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [results, setResults] =
    useState<
      { sourceId: string; name: string; status: string; message: string }[]
    >();
  const body = useRef<HTMLDivElement>(null);
  useEffect(() => {
    body.current?.scrollTo({ top: 0 });
  }, [hasPreview, results]);
  const isAvailable = (source: Source) =>
    !source.readOnly &&
    clients.some((client) => client.id === source.agent && client.configurable);
  const targetStatus = (source: Source) => {
    const existing = bindings.filter(
      (item) =>
        item.sourceId === source.id &&
        connections.some((connection) => connection.id === item.connectionId) &&
        (!binding || item.name === binding.name),
    );
    if (existing.some((item) => item.unavailable)) return "来源无法读取";
    if (existing.some((item) => item.missing))
      return existing.some((item) => !item.missing)
        ? "部分配置缺失"
        : "配置缺失";
    return existing.length ? "已有接入" : "";
  };
  const visibleSources = useMemo(
    () =>
      sources
        .filter((source) => !binding || source.id === binding.sourceId)
        .filter(
          (source) =>
            operation === "upsert" ||
            connections.every((connection) =>
              bindings.some(
                (b) =>
                  b.sourceId === source.id &&
                  b.connectionId === connection.id &&
                  (operation === "remove" ||
                    (!b.missing &&
                      (operation === "enable" ? !b.enabled : b.enabled) &&
                      !["claude", "workbuddy"].includes(source.agent))),
              ),
            ),
        ),
    [sources, connections, bindings, operation, binding],
  );
  useEffect(() => {
    const available = new Set(
      visibleSources
        .filter(
          (source) =>
            !source.readOnly &&
            clients.some(
              (client) => client.id === source.agent && client.configurable,
            ),
        )
        .map((source) => source.id),
    );
    if (operation === "upsert" && !binding)
      clients
        .filter((c) => c.supportsImport)
        .forEach((c) => available.add(`import:${c.id}`));
    setTargets((previous) => previous.filter((id) => available.has(id)));
    setPlans((previous) =>
      previous.some((plan) =>
        plan.changes.some((change) => !available.has(change.sourceId)),
      )
        ? []
        : previous,
    );
  }, [clients, visibleSources, operation, binding]);
  const preview = async () => {
    setPending(true);
    setError("");
    try {
      const generated = [];
      for (const client of clients.filter(
        (c) => c.supportsImport && targets.includes(`import:${c.id}`),
      )) {
        generated.push(
          await rpc<ImportConfig>("connections.export_config", {
            connectionIds: connections.map((c) => c.id),
            targetAgent: client.id,
            mode,
          }),
        );
      }
      const nativeTargets = targets.filter((id) => !id.startsWith("import:"));
      const plan = nativeTargets.length
        ? await rpc<Plan>("changes.preview", {
            connectionIds: connections.map((connection) => connection.id),
            targetSourceIds: nativeTargets,
            bindingTarget: binding
              ? { sourceId: binding.sourceId, name: binding.name }
              : undefined,
            mode,
            action: operation,
            replaceShared: aggregate && replaceShared && operation === "upsert",
          })
        : undefined;
      if (plan?.changes.length) setPlans([plan]);
      setImports(generated);
      if (!plan?.changes.length && !generated.length)
        setError("这些配置已经移除，无需写入。");
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
          results: {
            sourceId: string;
            name: string;
            status: string;
            message: string;
          }[];
        }>("changes.apply", { planId: plan.planId });
        all.push(...result.results);
      }
      setResults(all);
      await onDone();
      onNotice({
        text: all.every((result) => result.status === "applied")
          ? plans.every((plan) =>
              plan.changes.every((change) => change.kind === "cleanup"),
            )
            ? "已清理失效绑定"
            : "配置已写入，请在对应 Agent 重新加载 MCP"
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
        results
          ? "接入结果"
          : hasPreview
            ? plans.length
              ? "核对配置变更"
              : "客户端导入配置"
            : binding
              ? "管理 Agent 绑定"
              : "接给 Agent"
      }
      wide
      onClose={onClose}
    >
      <div className="modal-body" ref={body}>
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
        {!hasPreview && !results && (
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
            {["disable", "enable"].includes(operation) && (
              <p className="field-help">
                Claude Code 和 WorkBuddy 的启停请在原客户端维护。
              </p>
            )}
            {operation === "upsert" && (
              <div className="mode-choices">
                {!aggregate && (
                  <label className={mode === "direct" ? "chosen" : ""}>
                    <input
                      type="radio"
                      name="mode"
                      checked={mode === "direct"}
                      onChange={() => setMode("direct")}
                    />
                    <strong>原生直连</strong>
                    <small>
                      写入配置文件或生成导入内容，令牌和启动参数可能以明文保存
                    </small>
                  </label>
                )}
                <label className={mode === "shared" ? "chosen" : ""}>
                  <input
                    type="radio"
                    name="mode"
                    checked={mode === "shared"}
                    onChange={() => setMode("shared")}
                  />
                  <strong>{aggregate ? "统一入口" : "单服务入口"}</strong>
                  <small>
                    {aggregate
                      ? "一个 mcplink 配置提供所选服务，复用各服务托管认证"
                      : "每项服务各有一个入口，与统一入口和 CLI 共用托管认证"}
                  </small>
                </label>
              </div>
            )}
            {aggregate && operation === "upsert" && (
              <label className="target">
                <input
                  type="checkbox"
                  checked={replaceShared}
                  onChange={(event) => setReplaceShared(event.target.checked)}
                />
                <div>
                  <strong>替换目标配置中的单服务入口</strong>
                  <small>
                    仅移除同一来源中所选服务的托管共享条目；保留原生直连，并在预览中逐项列出。
                  </small>
                </div>
              </label>
            )}
            <h3 className="section-label">目标 Agent 与配置位置</h3>
            {operation === "upsert" &&
              bindings.some(
                (item) =>
                  item.missing &&
                  targets.includes(item.sourceId) &&
                  connections.some(
                    (connection) => connection.id === item.connectionId,
                  ),
              ) && (
                <p className="field-help">
                  应用后将重新创建缺失的配置条目，也可选择“移除”清理绑定记录。
                </p>
              )}
            {!visibleSources.length && (
              <p className="muted">没有符合此操作的绑定。</p>
            )}
            {visibleSources.map((source) => (
              <label className="target" key={source.id}>
                <input
                  type="checkbox"
                  value={source.id}
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
                {targetStatus(source) && (
                  <span className="muted">{targetStatus(source)}</span>
                )}
              </label>
            ))}
            {operation === "upsert" &&
              !binding &&
              clients
                .filter((client) => client.supportsImport)
                .map((client) => (
                  <label className="target" key={client.id}>
                    <input
                      type="checkbox"
                      checked={targets.includes(`import:${client.id}`)}
                      onChange={(event) =>
                        setTargets((previous) =>
                          event.target.checked
                            ? [...previous, `import:${client.id}`]
                            : previous.filter(
                                (id) => id !== `import:${client.id}`,
                              ),
                        )
                      }
                    />
                    <span className={`agent-avatar ${client.id}`}>
                      {client.label[0]}
                    </span>
                    <div>
                      <strong>{client.label} · 客户端导入</strong>
                      <small>
                        {client.id === "qwenwork"
                          ? "生成可粘贴的 JSON 配置"
                          : "生成自定义连接器的填写字段"}
                        ，可离线准备
                      </small>
                    </div>
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
                <p>
                  {change.name} · {change.path}
                </p>
                {change.action === "upsert" && change.mode === "direct" && (
                  <p>
                    直连会将连接设置写入上述文件，令牌会以明文保存。请核对项目文件的共享范围。
                  </p>
                )}
                <p>
                  {change.kind === "cleanup"
                    ? "仅清理托管记录。"
                    : change.reload}
                </p>
                <ChangeDiff change={change} />
              </div>
            ))}
          </div>
        ))}
        {results?.map((result, index) => (
          <div className="result-row" key={index}>
            <Status value={result.status} />
            <p>
              {result.name} ·{" "}
              {sources.find((source) => source.id === result.sourceId)?.path ||
                result.sourceId}{" "}
              · {result.message}
            </p>
          </div>
        ))}
        {imports.map((value) => (
          <ImportConfigCard key={value.agent} value={value} />
        ))}
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
      </div>
      <footer className="modal-footer">
        {hasPreview && !results && (
          <button
            className="button"
            disabled={pending}
            onClick={() => {
              setPlans([]);
              setImports([]);
            }}
          >
            返回调整
          </button>
        )}
        <button className="button" onClick={onClose}>
          {results || (imports.length > 0 && !plans.length) ? "完成" : "取消"}
        </button>
        {!results && (!hasPreview || plans.length > 0) && (
          <button
            className="button primary"
            disabled={pending || !targets.length}
            onClick={() => void (plans.length ? apply() : preview())}
          >
            {pending && <Loader2 size={15} className="spin" />}
            {plans.length
              ? "应用配置"
              : targets.length > 0 &&
                  targets.every((id) => id.startsWith("import:"))
                ? "生成导入配置"
                : "预览变更"}
          </button>
        )}
      </footer>
    </Modal>
  );
}
