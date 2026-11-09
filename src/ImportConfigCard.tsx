import { useState } from "react";
import { Copy } from "lucide-react";
import { Agent, ImportConfig, agentLabel } from "./contracts";

export const importInstructions = (agent: Agent) =>
  agent === "qwenwork"
    ? "在千问办公打开「扩展 → 连接器 → +添加」，粘贴 JSON 配置并保存、启用，然后新建任务验证工具调用。"
    : "在豆包工作打开「插件·技能·伙伴 → 插件 → +添加 → 新建自定义连接器」，选择 HTTP 或 STDIO，按下方字段填写并保存，再在任务中验证工具调用。";

function CopyValue({
  label,
  value,
  multiline = false,
}: {
  label: string;
  value: string;
  multiline?: boolean;
}) {
  const [message, setMessage] = useState("");
  return (
    <div className="import-value">
      <label className="field">
        <span>{label}</span>
        {multiline ? (
          <textarea
            aria-label={label}
            value={value}
            readOnly
            rows={8}
            spellCheck={false}
            onFocus={(event) => event.currentTarget.select()}
          />
        ) : (
          <input
            aria-label={label}
            value={value}
            readOnly
            onFocus={(event) => event.currentTarget.select()}
          />
        )}
      </label>
      <button
        className="button small"
        onClick={async () => {
          try {
            if (navigator.clipboard) await navigator.clipboard.writeText(value);
            else {
              const field = document.createElement("textarea");
              field.value = value;
              field.style.position = "fixed";
              field.style.opacity = "0";
              // Keep the fallback inside the modal's focus boundary.
              const parent =
                document.querySelector("dialog[open]") || document.body;
              parent.appendChild(field);
              try {
                field.select();
                if (!document.execCommand("copy"))
                  throw new Error("copy failed");
              } finally {
                field.remove();
              }
            }
            setMessage("已复制，请在客户端粘贴");
          } catch {
            setMessage("复制失败，请选中上方内容手动复制");
          }
        }}
      >
        <Copy size={13} />
        复制{label}
      </button>
      {message && <small role="status">{message}</small>}
    </div>
  );
}

export function ImportConfigCard({ value }: { value: ImportConfig }) {
  return (
    <section
      className="import-config"
      aria-label={`${agentLabel(value.agent)} 导入配置`}
    >
      <h3>{agentLabel(value.agent)} · 待客户端导入</h3>
      <p>{importInstructions(value.agent)}</p>
      <p className="field-help">
        生成和复制不代表已接入，mcplink
        无法读取此客户端的导入结果。已有同名连接器请先在客户端核对并更新。
      </p>
      {value.mode === "direct" && (
        <p className="warning-banner">
          下方是可直接使用的完整配置，可能包含明文令牌、地址和启动参数。
        </p>
      )}
      {value.mode === "shared" && (
        <p className="field-help">
          请选择 STDIO。共享入口引用本机
          mcplink，请保留程序位置并保持共享连接可用。
        </p>
      )}
      {value.mode === "shared" && value.gatewayPaused && (
        <p className="warning-banner">共享代理当前已暂停，恢复后才能调用。</p>
      )}
      {value.config ? (
        <CopyValue
          label="JSON 配置"
          value={JSON.stringify(value.config, null, 2)}
          multiline
        />
      ) : (
        value.entries.map(({ name, definition }) => (
          <div className="import-entry" key={name}>
            <h4>
              {name} · {definition.type === "stdio" ? "STDIO" : "HTTP"}
            </h4>
            <CopyValue label="服务器名称" value={name} />
            {definition.command !== undefined && (
              <CopyValue label="运行命令" value={definition.command} />
            )}
            {!!definition.args?.length && (
              <p className="field-help">
                参数按顺序逐项添加，保留每项中的空格和引号。
              </p>
            )}
            {definition.args?.map((arg, index) => (
              <CopyValue key={index} label={`参数 ${index + 1}`} value={arg} />
            ))}
            {Object.entries(definition.env || {}).map(([key, content]) => (
              <CopyValue key={key} label={`环境变量 ${key}`} value={content} />
            ))}
            {definition.url !== undefined && (
              <CopyValue label="服务器 URL" value={definition.url} />
            )}
            {Object.entries(definition.headers || {}).map(([key, content]) => (
              <CopyValue key={key} label={`Header ${key}`} value={content} />
            ))}
          </div>
        ))
      )}
    </section>
  );
}
