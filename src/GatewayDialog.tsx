import { useState } from "react";
import { Loader2 } from "lucide-react";
import { Connection, rpc } from "./contracts";
import { Modal, Status } from "./Modal";

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
  const [error, setError] = useState("");
  const available = connections.filter((c) => !c.detected && !c.members);
  const save = async () => {
    setPending(true);
    setError("");
    try {
      const entry = await rpc<Connection>("gateway.save", {
        id: connection?.id,
        editVersion: connection?.editVersion,
        title,
        connectionIds: selected,
      });
      await onDone(entry);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setPending(false);
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
          {pending && <Loader2 size={15} className="spin" />}保存服务选择
        </button>
      </footer>
    </Modal>
  );
}
