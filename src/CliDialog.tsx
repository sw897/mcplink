import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Copy, FolderOpen, Loader2 } from "lucide-react";
import { Connection, Json, Tool, rpc } from "./contracts";
import { Modal } from "./Modal";

type Descriptor = { tool: Tool; schemaHash: string };
type Generated = {
  name: string;
  directory: string;
  manifest: string;
  tools: string[];
};
type ContractCheck = {
  compatible: boolean;
  datasetMatches: boolean;
  tools: { name: string; status: string }[];
};
type Manifest = {
  name: string;
  connectionId: string;
  tools: { name: string }[];
};

const accountLabel = (connection: Connection) =>
  `${connection.title} · ${connection.members ? "统一入口" : connection.accountLabel || connection.name}`;

const objectValue = (value: unknown): Json | undefined =>
  value && typeof value === "object" && !Array.isArray(value)
    ? (value as Json)
    : undefined;

export function CliDialog({
  connection,
  connections,
  onClose,
}: {
  connection: Connection;
  connections: Connection[];
  onClose: () => void;
}) {
  const [source, setSource] = useState(connection.id);
  const [tools, setTools] = useState<Tool[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [active, setActive] = useState("");
  const [descriptor, setDescriptor] = useState<Descriptor>();
  const [search, setSearch] = useState("");
  const [name, setName] = useState("my-tools");
  const [parent, setParent] = useState("");
  const [argumentsText, setArgumentsText] = useState("{}");
  const [generated, setGenerated] = useState<Generated>();
  const [check, setCheck] = useState<ContractCheck>();
  const [manifestPath, setManifestPath] = useState("");
  const [output, setOutput] = useState<Json>();
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState("");
  const [refresh, setRefresh] = useState(0);
  const lock = useRef(false);
  useEffect(() => {
    let current = true;
    setLoading(true);
    setError("");
    setTools([]);
    setDescriptor(undefined);
    setActive("");
    setOutput(undefined);
    void rpc<{ tools: Tool[]; error?: unknown; upstreamErrors?: unknown[] }>(
      "connections.tools",
      { id: source },
    )
      .then((result) => {
        if (!current) return;
        if (result.error)
          throw new Error(
            typeof result.error === "string"
              ? result.error
              : "工具发现失败，请检查服务与账号授权",
          );
        setTools(result.tools);
        if (result.upstreamErrors?.length)
          setError("部分上游暂不可用，请先检查服务；本次仅展示可用工具。");
      })
      .catch((e: Error) => {
        if (current) setError(e.message);
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [source, refresh]);
  useEffect(() => {
    setDescriptor(undefined);
    setOutput(undefined);
    setArgumentsText("{}");
    if (!active) return;
    let current = true;
    void rpc<Descriptor>("tools.describe", { id: source, tool: active })
      .then((result) => {
        if (current) setDescriptor(result);
      })
      .catch((e: Error) => {
        if (current) setError(e.message);
      });
    return () => {
      current = false;
    };
  }, [source, active, refresh]);
  const run = async (label: string, action: () => Promise<void>) => {
    if (lock.current) return;
    lock.current = true;
    setPending(label);
    setError("");
    setNotice("");
    try {
      await action();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      lock.current = false;
      setPending("");
    }
  };
  const chooseManifest = () =>
    run("检查契约", async () => {
      const path = await open({
        multiple: false,
        filters: [{ name: "mcplink CLI", extensions: ["json"] }],
      });
      if (typeof path !== "string") return;
      const manifest = await rpc<Manifest>("cli.inspect", { manifest: path });
      const result = await rpc<ContractCheck>("cli.check", { manifest: path });
      setManifestPath(path);
      setCheck(result);
      if (
        result.datasetMatches &&
        connections.some((c) => c.id === manifest.connectionId && !c.detected)
      ) {
        setSource(manifest.connectionId);
        setName(manifest.name);
        setSelected(manifest.tools.map((t) => t.name));
      }
    });
  const requiredValue = descriptor?.tool.inputSchema?.required;
  const required = Array.isArray(requiredValue)
    ? requiredValue.filter(
        (value): value is string => typeof value === "string",
      )
    : [];
  const properties = objectValue(descriptor?.tool.inputSchema?.properties);
  const connectionLabel = (connection?: Connection) => {
    if (!connection) return "服务已移除";
    const label = accountLabel(connection);
    return connections.filter((c) => !c.detected && accountLabel(c) === label)
      .length > 1
      ? `${label} · ${connection.id.slice(0, 8)}`
      : label;
  };
  const toolLabel = (tool: string) => {
    const member = connections
      .find((c) => c.id === source)
      ?.members?.find((m) => tool.startsWith(`${m.namespace}__`));
    return member
      ? {
          name: tool.slice(member.namespace.length + 2),
          service: connectionLabel(
            connections.find((c) => c.id === member.connectionId),
          ),
        }
      : { name: tool, service: undefined };
  };
  return (
    <Modal title="工具 CLI" wide className="cli-modal" onClose={onClose}>
      <div className="modal-body cli-workflow">
        <p className="field-help">
          选择工具，查看参数，再生成终端命令。CLI 与 Agent
          共用此连接的托管认证。
        </p>
        <div className="cli-source">
          <label className="field">
            <span>服务或统一入口</span>
            <select
              value={source}
              disabled={!!pending}
              onChange={(e) => {
                setSource(e.target.value);
                setSelected([]);
                setGenerated(undefined);
                setCheck(undefined);
                setManifestPath("");
              }}
            >
              {connections
                .filter((c) => !c.detected)
                .map((c) => (
                  <option key={c.id} value={c.id}>
                    {connectionLabel(c)}
                  </option>
                ))}
            </select>
          </label>
          <button
            className="button"
            disabled={!!pending || loading}
            onClick={() => setRefresh(refresh + 1)}
          >
            刷新工具
          </button>
          <button
            className="button"
            disabled={!!pending}
            onClick={() => void chooseManifest()}
          >
            检查已有 CLI
          </button>
        </div>
        {check && (
          <div
            role="status"
            className={check.compatible ? "cli-contract" : "warning-banner"}
          >
            <strong>
              {check.compatible
                ? "工具契约一致"
                : check.datasetMatches
                  ? "工具契约需要核对"
                  : "此 CLI 属于另一数据目录，请先用 cli rebind 绑定本机连接"}
            </strong>
            <ul>
              {check.tools.map((tool) => (
                <li key={tool.name}>
                  {tool.name}：
                  {{
                    current: "一致",
                    changed: "已变化",
                    unavailable: "当前不可用",
                  }[tool.status] || tool.status}
                </li>
              ))}
            </ul>
            <small>{manifestPath}</small>
            {!check.compatible && check.datasetMatches && (
              <p>核对当前参数后，选择新输出目录重新生成。原 CLI 文件将保留。</p>
            )}
          </div>
        )}
        <div className="cli-columns">
          <section className="cli-tool-selection" aria-label="选择工具">
            <label className="field">
              <span>搜索工具 · 已选 {selected.length} 项</span>
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder="工具名称"
              />
            </label>
            {loading ? (
              <p role="status">正在获取工具…</p>
            ) : !tools.length ? (
              <p className="muted">
                此连接暂无可用工具，请检查入口范围和服务状态。
              </p>
            ) : (
              <div className="cli-tools">
                {tools
                  .filter((t) =>
                    t.name.toLowerCase().includes(search.toLowerCase()),
                  )
                  .map((tool) => (
                    <div
                      key={tool.name}
                      className={active === tool.name ? "active" : ""}
                    >
                      <input
                        aria-label={`导出 ${tool.name}`}
                        type="checkbox"
                        disabled={!!pending}
                        checked={selected.includes(tool.name)}
                        onChange={(e) =>
                          setSelected(
                            e.target.checked
                              ? [...selected, tool.name]
                              : selected.filter((n) => n !== tool.name),
                          )
                        }
                      />
                      <button
                        className="cli-tool-name"
                        aria-label={`查看 ${tool.name} 参数`}
                        aria-pressed={active === tool.name}
                        disabled={!!pending}
                        onClick={() => setActive(tool.name)}
                      >
                        {toolLabel(tool.name).name}
                        {toolLabel(tool.name).service && (
                          <small>{toolLabel(tool.name).service}</small>
                        )}
                      </button>
                    </div>
                  ))}
              </div>
            )}
            {selected.some((n) => !tools.some((t) => t.name === n)) &&
              !loading && (
                <p className="warning-banner">
                  选择中含当前不可用的工具。
                  <button
                    className="button small"
                    onClick={() =>
                      setSelected(
                        selected.filter((n) => tools.some((t) => t.name === n)),
                      )
                    }
                  >
                    移除不可用项
                  </button>
                </p>
              )}
          </section>
          <section className="cli-tool-help" aria-label="工具参数与调用">
            {!active ? (
              <p className="muted">
                点击工具名称查看参数。勾选只用于生成 CLI，不会执行工具。
              </p>
            ) : !descriptor ? (
              <p role="status">正在读取参数…</p>
            ) : (
              <>
                <h3>{toolLabel(descriptor.tool.name).name}</h3>
                {toolLabel(descriptor.tool.name).service && (
                  <p className="muted">
                    {toolLabel(descriptor.tool.name).service}
                  </p>
                )}
                {descriptor.tool.description && (
                  <p className="tool-description">
                    {descriptor.tool.description}
                  </p>
                )}
                {properties && (
                  <dl className="cli-parameters">
                    {Object.entries(properties).map(([key, value]) => {
                      const property = objectValue(value) || {};
                      return (
                        <div key={key}>
                          <dt>
                            {key}
                            {required?.includes(key) ? "（必填）" : ""}
                          </dt>
                          <dd>
                            {typeof property.type === "string"
                              ? property.type
                              : "JSON"}
                            {typeof property.description === "string"
                              ? ` · ${property.description}`
                              : ""}
                          </dd>
                        </div>
                      );
                    })}
                  </dl>
                )}
                <details>
                  <summary>调用名称与输入 Schema</summary>
                  <pre>
                    {descriptor.tool.name}
                    {"\n"}
                    {JSON.stringify(descriptor.tool.inputSchema, null, 2)}
                  </pre>
                </details>
                <label className="field">
                  <span>调用参数（JSON 对象）</span>
                  <textarea
                    spellCheck={false}
                    rows={5}
                    value={argumentsText}
                    disabled={!!pending}
                    onChange={(e) => setArgumentsText(e.target.value)}
                  />
                </label>
                <p className="field-help">
                  实际调用可能修改上游数据。请核对工具和参数；失败或超时后先核对执行结果，再决定是否重试。
                </p>
                <button
                  className="button"
                  disabled={!!pending || loading}
                  onClick={() =>
                    void run("调用工具", async () => {
                      let argumentsValue: unknown;
                      try {
                        argumentsValue = JSON.parse(argumentsText);
                      } catch {
                        throw new Error(
                          "调用参数须为有效 JSON 对象；工具尚未派发",
                        );
                      }
                      if (
                        !argumentsValue ||
                        Array.isArray(argumentsValue) ||
                        typeof argumentsValue !== "object"
                      )
                        throw new Error("调用参数须为 JSON 对象；工具尚未派发");
                      setOutput(undefined);
                      const result = await rpc<Json>("tools.call", {
                        id: source,
                        tool: descriptor.tool.name,
                        schemaHash: descriptor.schemaHash,
                        arguments: argumentsValue,
                      });
                      setOutput(result);
                      const upstream = result.output as Json | undefined;
                      setNotice(
                        upstream?.isError || upstream?.rpcError
                          ? "上游返回失败，请核对结果。"
                          : "调用完成，结果仅显示在当前窗口。",
                      );
                    })
                  }
                >
                  实际调用一次
                </button>
                {output && (
                  <pre className="cli-result" aria-label="调用结果">
                    {JSON.stringify(output, null, 2)}
                  </pre>
                )}
              </>
            )}
          </section>
        </div>
        <div className="cli-generation">
          <label className="field">
            <span>命令名称</span>
            <input
              value={name}
              maxLength={64}
              disabled={!!pending}
              onChange={(e) => setName(e.target.value)}
              placeholder="my-tools"
            />
          </label>
          <label className="field">
            <span>输出父目录</span>
            <input
              value={parent}
              readOnly
              placeholder="选择目录，将在其中新建命令同名文件夹"
            />
          </label>
          <button
            className="button"
            disabled={!!pending}
            onClick={() =>
              void run("选择目录", async () => {
                const path = await open({ directory: true, multiple: false });
                if (typeof path === "string") setParent(path);
              })
            }
          >
            <FolderOpen size={15} />
            选择目录
          </button>
        </div>
        <p className="field-help">
          新目录包含 POSIX、PowerShell 入口和契约清单，不写入凭据。运行时需在
          PATH 中提供 mcplink，或设置 MCPLINK_BIN。
        </p>
        {generated && (
          <div role="status" className="cli-generated">
            <strong>CLI 已生成</strong>
            <p>{generated.directory}</p>
            <p>
              macOS/Linux：进入生成目录运行 ./{generated.name}{" "}
              --help；Windows：运行 .\{generated.name}.ps1 --help。
            </p>
            <button
              className="button small"
              onClick={() =>
                void run("复制位置", async () => {
                  await navigator.clipboard.writeText(generated.manifest);
                  setNotice("已复制清单位置");
                })
              }
            >
              <Copy size={14} />
              复制清单位置
            </button>
          </div>
        )}
        {notice && <p role="status">{notice}</p>}
        {error && (
          <div role="alert" className="warning-banner">
            {error}
          </div>
        )}
      </div>
      <footer className="modal-footer">
        <button className="button" onClick={onClose}>
          关闭
        </button>
        <button
          className="button primary"
          disabled={
            !!pending || loading || !parent || !selected.length || !name.trim()
          }
          onClick={() =>
            void run("生成 CLI", async () => {
              const result = await rpc<Generated>("cli.generate", {
                id: source,
                name,
                output: `${parent}/${name}`,
                tools: selected,
              });
              setGenerated(result);
            })
          }
        >
          {pending && <Loader2 size={15} className="spin" />}
          {pending || "生成 CLI"}
        </button>
      </footer>
    </Modal>
  );
}
