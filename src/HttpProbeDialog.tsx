import { useEffect, useRef, useState } from "react";
import { Connection, ToolInventory, rpc } from "./contracts";
import { Modal, Status } from "./Modal";

export function HttpProbeDialog({
  connection,
  onClose,
  onResult,
  onEdit,
  onCall,
}: {
  connection: Connection;
  onClose: () => void;
  onResult: (result: ToolInventory) => Promise<void>;
  onEdit: () => void;
  onCall: () => void;
}) {
  const [preview, setPreview] = useState<ToolInventory>();
  const [result, setResult] = useState<ToolInventory>();
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const running = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    void rpc<ToolInventory>("connections.tools", { id: connection.id })
      .then((value) => {
        if (value.verification?.probe === undefined)
          throw new Error("后台不支持探测确认，请使用同一构建的桌面与后台。");
        if (mounted.current) setPreview(value);
      })
      .catch((cause: Error) => {
        if (mounted.current) setError(cause.message);
      });
    return () => {
      mounted.current = false;
    };
  }, [connection.id]);
  const probe = preview?.verification?.probe;
  const run = async () => {
    if (running.current || !probe || !preview?.connection.active) return;
    const revision = preview.connection.revision;
    if (revision === undefined || !Number.isSafeInteger(revision)) {
      setError("无法核对连接修订，请重新打开探测窗口。");
      return;
    }
    running.current = true;
    setPending(true);
    setError("");
    try {
      const value = await rpc<ToolInventory>("connections.probe", {
        id: connection.id,
        expectedRevision: revision,
      });
      if (mounted.current) setResult(value);
      await onResult(value);
    } catch (cause) {
      if (mounted.current) setError((cause as Error).message);
    } finally {
      if (mounted.current) setPending(false);
      // Keep this confirmation single-use, including after an uncertain failure.
    }
  };
  return (
    <Modal title="HTTP API 探测" onClose={onClose}>
      <div className="modal-body">
        <p>{connection.title}</p>
        {error && (
          <p className="warning-banner" role="alert">
            {error}
          </p>
        )}
        {!preview && !error && <p role="status">正在读取已保存的探测配置…</p>}
        {preview && !probe && (
          <p role="status">
            未配置探测操作。工具定义仍可查看；可编辑服务的 probe，或在工具 CLI
            中选择工具并明确调用一次。
          </p>
        )}
        {probe && (
          <>
            <dl className="details">
              <div>
                <dt>探测工具</dt>
                <dd>
                  <code>{probe.tool}</code>
                </dd>
              </div>
              <div>
                <dt>HTTP 操作</dt>
                <dd>
                  <code>
                    {probe.method} {probe.path}
                  </code>
                </dd>
              </div>
            </dl>
            <p className="field-help">
              确认后将使用已保存的探测参数和托管认证发起一次真实 HTTP
              请求，可能产生费用或业务变更。查看此窗口和取消均不执行请求。
            </p>
          </>
        )}
        {preview && !preview.connection.active && (
          <p role="alert" className="warning-banner">
            连接已断开，请先恢复连接和认证。
          </p>
        )}
        {pending && (
          <p role="status">正在运行一次探测；关闭窗口不会撤回已发出的请求。</p>
        )}
        {result && (
          <div
            role="status"
            className={
              result.connection.status === "ready"
                ? "field-help"
                : "warning-banner"
            }
          >
            <Status value={result.connection.status} />
            <p>
              {result.connection.status === "ready"
                ? "真实探测调用已通过。"
                : result.connection.lastError ||
                  "探测未通过，请核对服务与认证。"}
            </p>
          </div>
        )}
        {error && running.current && (
          <p className="field-help">
            本次确认已使用。请先核对服务端执行结果，再重新打开窗口决定是否再次探测。
          </p>
        )}
        <footer className="modal-footer">
          <button className="button" onClick={onClose}>
            {result || running.current ? "关闭" : "取消"}
          </button>
          {preview && !probe && (
            <button className="button" onClick={onEdit}>
              编辑服务
            </button>
          )}
          {preview && !probe && (
            <button className="button" onClick={onCall}>
              工具 CLI
            </button>
          )}
          {probe && (
            <button
              className="button primary"
              disabled={
                pending || running.current || !preview?.connection.active
              }
              onClick={() => void run()}
            >
              确认并运行一次
            </button>
          )}
        </footer>
      </div>
    </Modal>
  );
}
