import { useEffect, useMemo, useState } from "react";
import { Loader2, Server } from "lucide-react";
import {
  Agent,
  LocalAgent,
  Connection,
  Source,
  Binding,
  Plan,
  Change,
  changeLabel,
  agentLabel,
  rpc,
} from "./contracts";
import { Status, Modal } from "./Modal";

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
    const available = sources.filter(
      (s) =>
        s.agent === targetAgent &&
        !s.readOnly &&
        clients.some((c) => c.id === targetAgent && c.configurable),
    );
    return available.length === 1 ? [available[0].id] : [];
  });
  const [mode, setMode] = useState(binding?.mode || "direct");
  const [operation, setOperation] = useState(
    binding?.missing ? "remove" : "upsert",
  );
  const [plans, setPlans] = useState<Plan[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [results, setResults] =
    useState<
      { sourceId: string; name: string; status: string; message: string }[]
    >();
  const isAvailable = (source: Source) =>
    !source.readOnly &&
    clients.some((client) => client.id === source.agent && client.configurable);
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
                      source.agent !== "claude")),
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
    setTargets((previous) => previous.filter((id) => available.has(id)));
    setPlans((previous) =>
      previous.some((plan) =>
        plan.changes.some((change) => !available.has(change.sourceId)),
      )
        ? []
        : previous,
    );
  }, [clients, visibleSources]);
  const preview = async () => {
    setPending(true);
    setError("");
    try {
      const plan = await rpc<Plan>("changes.preview", {
        connectionIds: connections.map((connection) => connection.id),
        targetSourceIds: targets,
        bindingTarget: binding
          ? { sourceId: binding.sourceId, name: binding.name }
          : undefined,
        mode,
        action: operation,
      });
      if (plan.changes.length) setPlans([plan]);
      else setError("这些配置已经移除，无需写入。");
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
          : plans.length
            ? "核对配置变更"
            : binding
              ? "管理 Agent 绑定"
              : "接给 Agent"
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
                  <small>
                    连接设置写入所选配置文件，令牌和启动参数可能以明文保存
                  </small>
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
            {!visibleSources.length && (
              <p className="muted">没有符合此操作的绑定。</p>
            )}
            {visibleSources.map((source) => (
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
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
      </div>
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
    </Modal>
  );
}
