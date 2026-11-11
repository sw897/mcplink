import { useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { Connection, rpc } from "./contracts";
import { Modal, Status } from "./Modal";

function ToolRange({
  connection,
  value,
  onChange,
}: {
  connection: Connection;
  value: string[] | null;
  onChange: (value: string[] | null) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [tools, setTools] = useState<
    { name: string; description?: string }[] | null
  >(null);
  const [error, setError] = useState("");
  const [search, setSearch] = useState("");
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!expanded) return;
    let active = true;
    setError("");
    void rpc<{
      tools: { name: string; description?: string }[];
      error?: unknown;
    }>("connections.tools", { id: connection.id })
      .then((result) => {
        if (result.error)
          throw new Error(
            typeof result.error === "string"
              ? result.error
              : "无法获取工具，请检查服务与账号授权；已保存的选择将保留。",
          );
        if (active) setTools(result.tools);
      })
      .catch((e: Error) => {
        if (active) setError(e.message);
      });
    return () => {
      active = false;
    };
  }, [expanded, connection.id, attempt]);
  const names = [
    ...new Set([...(tools || []).map((t) => t.name), ...(value || [])]),
  ];
  return (
    <div className="tool-range">
      <button
        type="button"
        className="button small"
        aria-expanded={expanded}
        onClick={() => setExpanded(!expanded)}
      >
        工具范围：{value === null ? "沿用服务范围" : `已选 ${value.length} 项`}
      </button>
      {expanded && (
        <div className="tool-range-body">
          <label>
            <input
              type="radio"
              name={`range-${connection.id}`}
              checked={value === null}
              onChange={() => onChange(null)}
            />
            沿用服务范围
          </label>
          <label>
            <input
              type="radio"
              name={`range-${connection.id}`}
              checked={value !== null}
              onChange={() => onChange([])}
            />
            仅开放所选工具
          </label>
          <p className="field-help">
            入口范围只能收窄服务的工具范围。选择 0
            项时不开放工具，资源和提示词不受影响。
          </p>
          {error ? (
            <div role="alert" className="warning-banner">
              {error}{" "}
              <button
                className="button small"
                onClick={() => setAttempt(attempt + 1)}
              >
                重新获取
              </button>
            </div>
          ) : tools === null ? (
            <p className="muted">正在获取工具…</p>
          ) : null}
          {value !== null && (
            <>
              <input
                aria-label={`${connection.title} 搜索工具`}
                placeholder="搜索工具名称"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
              <div className="toolbar compact">
                <button
                  className="button small"
                  disabled={!tools}
                  onClick={() =>
                    onChange([
                      ...new Set([
                        ...(value || []),
                        ...(tools || []).map((t) => t.name),
                      ]),
                    ])
                  }
                >
                  选择当前全部工具
                </button>
                <button className="button small" onClick={() => onChange([])}>
                  清空选择
                </button>
              </div>
              <div className="tool-range-list">
                {names
                  .filter((name) =>
                    name.toLowerCase().includes(search.toLowerCase()),
                  )
                  .map((name) => (
                    <label key={name}>
                      <input
                        type="checkbox"
                        checked={value.includes(name)}
                        onChange={(e) =>
                          onChange(
                            e.target.checked
                              ? [...value, name]
                              : value.filter((n) => n !== name),
                          )
                        }
                      />
                      <span>
                        {name}
                        {tools && !tools.some((t) => t.name === name) && (
                          <small className="muted">
                            当前不可用，保留已保存选择
                          </small>
                        )}
                      </span>
                    </label>
                  ))}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

export function GatewayDialog({
  connection,
  connections,
  onClose,
  onDone,
}: {
  connection?: Connection;
  connections: Connection[];
  onClose: () => void;
  onDone: (entry: Connection) => Promise<void>;
}) {
  const [title, setTitle] = useState(connection?.title || "mcplink");
  const [selected, setSelected] = useState(
    connection?.members?.map((m) => m.connectionId) || [],
  );
  const [pending, setPending] = useState(false);
  const saving = useRef(false);
  const [selections, setSelections] = useState<Record<string, string[] | null>>(
    () =>
      Object.fromEntries(
        connection?.members?.map((m) => [
          m.connectionId,
          m.includeTools ?? null,
        ]) || [],
      ),
  );
  const [error, setError] = useState("");
  const available = connections.filter((c) => !c.detected && !c.members);
  const save = async () => {
    if (saving.current) return;
    saving.current = true;
    setPending(true);
    setError("");
    try {
      const entry = await rpc<Connection>("gateway.save", {
        id: connection?.id,
        editVersion: connection?.editVersion,
        title,
        connectionIds: selected,
        toolSelections: Object.fromEntries(
          selected.map((id) => [id, selections[id] ?? null]),
        ),
      });
      await onDone(entry);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
      saving.current = false;
    }
  };
  return (
    <Modal
      title={connection ? "编辑统一入口" : "创建统一入口"}
      wide
      onClose={onClose}
    >
      <div className="modal-body">
        <label className="field">
          <span>入口名称</span>
          <input
            value={title}
            maxLength={120}
            onChange={(e) => setTitle(e.target.value)}
          />
        </label>
        <p className="field-help">
          Agent 中的配置名称为
          mcplink。每个入口可以选择不同的服务，账号授权与单服务入口、CLI 共用。
        </p>
        <h3 className="section-label">
          提供给 Agent 的服务 · 已选 {selected.length} 项
        </h3>
        {available.map((c) => (
          <div className="gateway-member" key={c.id}>
            <label className="target" key={c.id}>
              <input
                type="checkbox"
                checked={selected.includes(c.id)}
                onChange={(e) =>
                  setSelected((previous) =>
                    e.target.checked
                      ? [...previous, c.id]
                      : previous.filter((id) => id !== c.id),
                  )
                }
              />
              <div>
                <strong>{c.title}</strong>
                <small>
                  {c.name} · {c.accountLabel || c.endpoint}
                </small>
              </div>
              <Status value={c.status} />
            </label>
            {selected.includes(c.id) && (
              <ToolRange
                connection={c}
                value={selections[c.id] ?? null}
                onChange={(value) =>
                  setSelections((previous) => ({ ...previous, [c.id]: value }))
                }
              />
            )}
          </div>
        ))}
        {!available.length && (
          <p className="muted">请先添加服务，或将已有服务纳入托管。</p>
        )}
        <p className="field-help">
          保存后服务选择立即生效，无需重写 Agent 配置。请在 Agent
          中刷新工具列表，必要时重新加载 MCP
          或开启新会话。未选服务时入口不提供任何工具。
        </p>
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
      </div>
      <footer className="modal-footer">
        <button className="button" disabled={pending} onClick={onClose}>
          取消
        </button>
        <button
          className="button primary"
          disabled={pending || !title.trim()}
          onClick={() => void save()}
        >
          {pending && <Loader2 size={15} className="spin" />}保存入口
        </button>
      </footer>
    </Modal>
  );
}
