import React, { useRef, useState } from "react";
import { FileInput, Loader2 } from "lucide-react";
import { Definition, rpc } from "./contracts";

export type CredentialInput = { header: string; kind: string; hint: string };
type ImportOperation = {
  key: string;
  operationId?: string;
  name: string;
  method: string;
  path: string;
  summary: string;
  supported: boolean;
  issue?: string;
};
type Preview = {
  title: string;
  suggestedName: string;
  sourceVersion: string;
  servers: string[];
  serverUrl?: string;
  operations: ImportOperation[];
  credentialInputs: CredentialInput[];
  issues: { code: string; message: string }[];
  definition: Definition | null;
};

export function OpenApiImport({
  disabled,
  onApply,
}: {
  disabled: boolean;
  onApply: (preview: Preview & { definition: Definition }) => void;
}) {
  const [open, setOpen] = useState(false);
  const [source, setSource] = useState("");
  const [fileName, setFileName] = useState("");
  const [server, setServer] = useState("");
  const [preview, setPreview] = useState<Preview>();
  const [selection, setSelection] = useState<string[]>([]);
  const [names, setNames] = useState<Record<string, string>>({});
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const generation = useRef(0);
  const listId = React.useId();

  const inspect = async (text: string, address?: string) => {
    const current = ++generation.current;
    setPending(true);
    setError("");
    try {
      const result = await rpc<Preview>("httpApi.import", {
        source: text,
        ...(address ? { serverUrl: address } : {}),
      });
      if (generation.current !== current) return;
      setPreview(result);
      setServer(result.serverUrl || address || "");
      setSelection([]);
      setNames({});
    } catch (e) {
      if (generation.current === current) setError((e as Error).message);
    } finally {
      if (generation.current === current) setPending(false);
    }
  };

  const load = async (file?: File) => {
    const current = ++generation.current;
    setPending(false);
    setPreview(undefined);
    setSelection([]);
    setNames({});
    setSource("");
    setError("");
    setFileName(file?.name || "");
    if (!file) return;
    if (file.size > 4 * 1024 * 1024) {
      setError("请选择不超过 4 MiB 的 JSON 或 YAML 文件");
      return;
    }
    setPending(true);
    try {
      const text = await file.text();
      if (generation.current !== current) return;
      setSource(text);
      setServer("");
      await inspect(text);
    } catch {
      if (generation.current === current) {
        setError("无法读取此文件，请选择 UTF-8 JSON 或 YAML 文档");
        setPending(false);
      }
    }
  };

  const apply = async () => {
    const current = ++generation.current;
    setPending(true);
    setError("");
    try {
      const result = await rpc<Preview>("httpApi.import", {
        source,
        serverUrl: server,
        operations: selection,
        toolNames: names,
      });
      if (current !== generation.current) return;
      if (!result.definition) {
        setError(
          result.issues.map((issue) => issue.message).join("；") ||
            "请选择可导入的操作",
        );
        return;
      }
      onApply({ ...result, definition: result.definition });
      setOpen(false);
    } catch (e) {
      if (current === generation.current) setError((e as Error).message);
    } finally {
      if (current === generation.current) setPending(false);
    }
  };

  return (
    <details
      className="openapi-import"
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>
        <FileInput size={16} />从 OpenAPI / Swagger 文件导入
      </summary>
      <div className="openapi-content">
        <p className="field-help">
          选择需要开放的操作，再生成连接配置。读取和预览文档不会调用上游接口。
        </p>
        <label className="field openapi-file">
          <span>接口文档 · JSON / YAML，最大 4 MiB</span>
          <input
            type="file"
            accept=".json,.yaml,.yml,application/json,application/yaml,text/yaml"
            aria-label="OpenAPI 或 Swagger 文件"
            disabled={disabled || pending}
            onChange={(event) => void load(event.target.files?.[0])}
          />
        </label>
        {preview && (
          <>
            <div className="openapi-source">
              <strong>{preview.title}</strong>
              <span>规范 {preview.sourceVersion}</span>
            </div>
            <label className="field">
              <span>服务地址</span>
              <input
                aria-label="导入服务地址"
                type="url"
                value={server}
                list={listId}
                placeholder="https://api.example.com/v1"
                disabled={disabled || pending}
                onChange={(event) => {
                  setServer(event.target.value);
                  setSelection([]);
                }}
              />
              <datalist id={listId}>
                {preview.servers.map((url) => (
                  <option key={url} value={url} />
                ))}
              </datalist>
            </label>
            <button
              type="button"
              className="button"
              disabled={disabled || pending || !server}
              onClick={() => void inspect(source, server)}
            >
              更新地址并预览
            </button>
            <div className="openapi-selection-bar">
              <strong>已选 {selection.length} 项操作</strong>
              <button
                type="button"
                className="text-button"
                disabled={disabled || pending || selection.length === 0}
                onClick={() => setSelection([])}
              >
                清空选择
              </button>
            </div>
            <div className="openapi-operations" aria-label="可导入的 API 操作">
              {preview.operations.length === 0 && (
                <p className="field-help">文档中没有可识别的操作。</p>
              )}
              {preview.operations.map((op) => (
                <div
                  className={`openapi-operation${op.supported ? "" : " unsupported"}`}
                  key={op.key}
                >
                  <input
                    type="checkbox"
                    aria-label={`选择 ${op.key}`}
                    checked={selection.includes(op.key)}
                    disabled={disabled || pending || !op.supported || !server}
                    onChange={(event) =>
                      setSelection((current) =>
                        event.target.checked
                          ? [...current, op.key]
                          : current.filter((key) => key !== op.key),
                      )
                    }
                  />
                  <div className="openapi-operation-info">
                    <div className="openapi-route">
                      <strong>{op.method}</strong>
                      <span>{op.path}</span>
                    </div>
                    <span className="field-help">{op.issue || op.summary}</span>
                  </div>
                  <label className="openapi-tool-name">
                    <span>工具名</span>
                    <input
                      aria-label={`${op.key} 工具名`}
                      value={names[op.key] ?? op.name}
                      maxLength={80}
                      disabled={disabled || pending || !op.supported}
                      onChange={(event) =>
                        setNames((current) => ({
                          ...current,
                          [op.key]: event.target.value,
                        }))
                      }
                    />
                  </label>
                </div>
              ))}
            </div>
            {preview.issues.length > 0 && (
              <p className="field-help">
                {preview.issues.map((issue) => issue.message).join("；")}
              </p>
            )}
            <p className="field-help">
              输入参数按 path、query、body
              分组。工具开放范围以你的选择为准，HTTP 方法不代表业务权限。
            </p>
            <button
              type="button"
              className="button"
              disabled={
                disabled || pending || selection.length === 0 || !server
              }
              onClick={() => void apply()}
            >
              {pending && <Loader2 size={14} className="spin" />}生成{" "}
              {selection.length} 项工具配置
            </button>
          </>
        )}
        {pending && !preview && (
          <p className="field-help">
            <Loader2 size={14} className="spin" />
            正在读取 {fileName}
          </p>
        )}
        {error && (
          <div role="alert" className="warning-banner">
            {error}
          </div>
        )}
      </div>
    </details>
  );
}
