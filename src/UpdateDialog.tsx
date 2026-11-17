import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowUpCircle,
  CheckCircle2,
  Download,
  ExternalLink,
  Loader2,
  RefreshCw,
  ShieldCheck,
  X,
} from "lucide-react";
import { Modal } from "./Modal";

type UpdateView = {
  revision: number;
  currentVersion: string;
  target: string;
  supported: boolean;
  phase: string;
  preview: boolean;
  candidateId: number;
  release?: {
    version: string;
    name: string;
    notes: string;
    publishedAt: string;
    prerelease: boolean;
    url: string;
  };
  downloaded: number;
  total: number;
  cliTargets: string[];
  canDownload: boolean;
  canInstall: boolean;
  error?: string;
  startupIssue?: string;
  message?: string;
};

const bytes = (value: number) =>
  value >= 1024 * 1024
    ? `${(value / (1024 * 1024)).toFixed(1)} MB`
    : `${Math.ceil(value / 1024)} KB`;

export function UpdateDialog({ onClose }: { onClose: () => void }) {
  const [view, setView] = useState<UpdateView>();
  const [preview, setPreview] = useState(
    () => localStorage.getItem("mcplink.updates.preview") !== "false",
  );
  const [confirmation, setConfirmation] = useState<number>();
  const [error, setError] = useState("");
  const [pending, setPending] = useState<string[]>([]);
  const operations = useRef(new Set<string>());
  const mounted = useRef(true);
  const apply = useCallback((next: UpdateView) => {
    if (mounted.current)
      setView((current) =>
        !current || (next.revision ?? 0) >= (current.revision ?? 0)
          ? next
          : current,
      );
  }, []);

  useEffect(() => {
    mounted.current = true;
    const events = listen<UpdateView>("software-update", (event) =>
      apply(event.payload),
    );
    void events.catch((e) => {
      if (mounted.current) setError(String(e));
    });
    void invoke<UpdateView>("desktop_update", { action: "status" })
      .then(apply)
      .catch((e) => setError(String(e)));
    return () => {
      mounted.current = false;
      void events.then((stop) => stop()).catch(() => {});
    };
  }, [apply]);
  useEffect(() => {
    if (!view?.canInstall || view.candidateId !== confirmation)
      setConfirmation(undefined);
  }, [view?.canInstall, view?.candidateId, confirmation]);

  const run = async (action: string, options: Record<string, unknown> = {}) => {
    if (operations.current.has(action)) return;
    operations.current.add(action);
    setPending([...operations.current]);
    setError("");
    try {
      const next = await invoke<UpdateView>("desktop_update", {
        action,
        preview,
        candidateId: view?.candidateId,
        ...options,
      });
      apply(next);
    } catch (e) {
      if (mounted.current) setError(String(e));
    } finally {
      operations.current.delete(action);
      if (mounted.current) setPending([...operations.current]);
    }
  };
  const installing = view?.phase === "installing";
  const downloading = view?.phase === "downloading";
  const checking = view?.phase === "checking";
  const busy =
    checking ||
    downloading ||
    installing ||
    pending.some((p) => p !== "cancel" && p !== "open_release");
  const close = () => {
    if (!installing) onClose();
  };
  const percent = view?.total
    ? Math.min(100, Math.floor((view.downloaded / view.total) * 100))
    : 0;
  const label = installing
    ? "正在清理共享后台并安装更新…"
    : checking
      ? "正在检查 GitHub 发布…"
      : downloading
        ? "正在下载更新包…"
        : view?.canInstall
          ? "更新包已通过签名、版本和 SHA-256 校验"
          : view?.phase === "current"
            ? "当前版本无需更新"
            : view?.phase === "no-release"
              ? "所选范围暂无已发布版本"
              : view?.phase === "unavailable"
                ? "发现新版本，尚未提供本平台的签名更新包"
                : view?.phase === "cancelled"
                  ? "下载已取消，可以重新下载"
                  : view?.canDownload
                    ? `发现新版本 v${view.release?.version}`
                    : "检查新版本，选择合适的时间升级";

  return (
    <Modal
      title="软件更新"
      onClose={close}
      closeDisabled={installing}
      className="update-dialog"
    >
      <div className="modal-body update-body">
        <div className="update-version">
          <div>
            <span className="muted">当前版本</span>
            <strong>v{view?.currentVersion || "…"}</strong>
          </div>
          <span className="muted">
            {view?.target === "darwin-aarch64"
              ? "macOS Apple Silicon"
              : view?.target === "windows-x86_64"
                ? "Windows x64"
                : "桌面应用"}
          </span>
        </div>
        <p className="muted">
          从 GitHub Releases
          获取更新。检查和下载由你发起，安装前会清理共享后台。
        </p>
        <label className="update-channel">
          <input
            type="checkbox"
            checked={preview}
            disabled={busy}
            onChange={(e) => {
              const next = e.target.checked;
              setPreview(next);
              localStorage.setItem("mcplink.updates.preview", String(next));
              setConfirmation(undefined);
              if (view && view.phase !== "idle")
                void run("check", { preview: next });
            }}
          />
          包含预览版
        </label>
        {(error || view?.error) && (
          <div className="update-error" role="alert">
            {error || view?.error}
          </div>
        )}
        {view?.startupIssue && (
          <div className="update-error" role="alert">
            <p>{view.startupIssue}</p>
            <button
              className="button small"
              disabled={busy}
              onClick={() => void run("retry_cli")}
            >
              重试同步 CLI
            </button>
          </div>
        )}
        {view?.message && (
          <p className="update-message" role="status">
            {view.message}
          </p>
        )}
        {view && !view.supported && (
          <p className="update-error">
            当前平台暂未提供应用内升级，请到 GitHub 发布页面下载安装包。
          </p>
        )}
        <section
          className="update-status"
          aria-live="polite"
          aria-atomic="true"
        >
          {checking || downloading || installing ? (
            <Loader2 size={19} className="spin" />
          ) : view?.canInstall ? (
            <ShieldCheck size={19} />
          ) : view?.phase === "current" ? (
            <CheckCircle2 size={19} />
          ) : (
            <ArrowUpCircle size={19} />
          )}
          <strong>{label}</strong>
        </section>
        {(downloading || view?.canInstall || installing) && (
          <div className="update-download">
            <progress aria-label="更新包下载进度" max={100} value={percent} />
            <div>
              <span>
                {bytes(view?.downloaded || 0)} / {bytes(view?.total || 0)}
              </span>
              <span>{percent}%</span>
            </div>
            {downloading && (
              <p className="muted">
                关闭面板后会继续下载；取消下载不会修改已安装应用。
              </p>
            )}
          </div>
        )}
        {view?.release && (
          <section className="update-release">
            <header>
              <h3>{view.release.name}</h3>
              {view.release.prerelease && <span className="tag">预览版</span>}
            </header>
            <p className="muted">
              {view.release.publishedAt
                ? new Date(view.release.publishedAt).toLocaleDateString()
                : ""}
              {view.total > 0 ? ` · 更新包 ${bytes(view.total)}` : ""}
            </p>
            <details>
              <summary>查看更新说明</summary>
              <p className="update-notes">
                {view.release.notes || "本版本未提供更新说明。"}
              </p>
            </details>
          </section>
        )}
        {confirmation !== undefined && view?.canInstall && (
          <section className="update-confirm" aria-label="确认升级">
            <strong>确认升级至 v{view.release?.version}？</strong>
            <p>
              共享调用会短暂停止，应用安装后将重新启动。已有账号和 Agent
              配置会保留；请让 Agent 重新加载 MCP 入口。
            </p>
            {view.cliTargets.length > 0 ? (
              <>
                <p>以下独立 CLI 会同步更新：</p>
                <ul>
                  {view.cliTargets.map((path) => (
                    <li key={path}>
                      <code>{path}</code>
                    </li>
                  ))}
                </ul>
              </>
            ) : (
              <p>
                未识别到可自动同步的独立 CLI；如有其他位置的 CLI，请一并更新。
              </p>
            )}
            {view.target === "windows-x86_64" && (
              <p>若 Agent 仍占用程序，安装会停止并提示先关闭相关 MCP 连接。</p>
            )}
            <div className="update-actions">
              <button
                className="button"
                onClick={() => setConfirmation(undefined)}
              >
                暂不安装
              </button>
              <button
                className="button primary"
                onClick={() =>
                  void run("install", {
                    confirmed: true,
                    candidateId: confirmation,
                  })
                }
              >
                确认安装并重启
              </button>
            </div>
          </section>
        )}
        <div className="update-actions">
          <button
            className="text-button"
            disabled={installing}
            onClick={() => void run("open_release")}
          >
            <ExternalLink size={14} />
            GitHub 发布页面
          </button>
          <span className="update-actions-spacer" />
          {downloading ? (
            <button
              className="button"
              disabled={pending.includes("cancel")}
              onClick={() => void run("cancel")}
            >
              <X size={14} />
              取消下载
            </button>
          ) : confirmation !== undefined || installing ? null : (
            <>
              <button
                className={`button ${!view?.canDownload && !view?.canInstall ? "primary" : ""}`}
                disabled={!view || !view.supported || busy}
                onClick={() => void run("check")}
              >
                <RefreshCw size={14} className={checking ? "spin" : ""} />
                {checking ? "检查中…" : "检查更新"}
              </button>
              {view?.canDownload && (
                <button
                  className="button primary"
                  disabled={busy}
                  onClick={() => void run("download")}
                >
                  <Download size={14} />
                  下载更新包
                </button>
              )}
              {view?.canInstall && (
                <button
                  className="button primary"
                  disabled={busy}
                  onClick={() => setConfirmation(view.candidateId)}
                >
                  <ArrowUpCircle size={14} />
                  {view.error ? "重试安装" : "安装并重新启动"}
                </button>
              )}
            </>
          )}
        </div>
      </div>
    </Modal>
  );
}
