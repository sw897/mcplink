import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Loader2 } from "lucide-react";
import { Agent, agentLabel, rpc } from "./contracts";
import { Modal } from "./Modal";

export function SourceDialog({
  agent,
  onClose,
  onSaved,
}: {
  agent: Agent;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [scope, setScope] = useState("user");
  const [path, setPath] = useState("");
  const [project, setProject] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const scoped = scope === "local" || scope === "project";
  return (
    <Modal title={`添加 ${agentLabel(agent)} 来源`} onClose={onClose}>
      <form
        className="modal-body"
        onSubmit={async (event) => {
          event.preventDefault();
          setPending(true);
          setError("");
          try {
            await rpc("sources.add", {
              agent,
              scope: scope === "readonly" ? null : scope,
              path: scoped && agent !== "opencode" ? null : path || null,
              project: scoped ? project : null,
            });
            await onSaved();
          } catch (error) {
            setError((error as Error).message);
          } finally {
            setPending(false);
          }
        }}
      >
        <label className="field">
          <span>配置作用域</span>
          <select
            value={scope}
            onChange={(event) => setScope(event.target.value)}
          >
            <option value="user">User · 所有项目</option>
            {agent === "claude" && (
              <option value="local">Local · 个人项目配置</option>
            )}
            {[
              "claude",
              "opencode",
              "zcode",
              "kimicode",
              "workbuddy",
              "pi",
            ].includes(agent) && (
              <option value="project">Project · 项目配置</option>
            )}
            {agent === "claude" && (
              <option value="readonly">其他 JSON · 只读盘点</option>
            )}
          </select>
        </label>
        {scoped && (
          <label className="field">
            <span>项目目录</span>
            <input
              value={project}
              onChange={(event) => setProject(event.target.value)}
              required
              placeholder="完整目录路径"
            />
            <button
              type="button"
              className="button"
              onClick={async () => {
                const selected = await open({
                  directory: true,
                  multiple: false,
                  title: "选择项目目录",
                });
                if (typeof selected === "string") setProject(selected);
              }}
            >
              选择项目目录
            </button>
          </label>
        )}
        {!["qwenwork", "pi"].includes(agent) &&
          (!scoped || agent === "opencode") && (
            <label className="field">
              <span>配置文件</span>
              <input
                value={path}
                onChange={(event) => setPath(event.target.value)}
                required={scope === "readonly"}
                placeholder={
                  agent === "opencode"
                    ? "留空自动选择 opencode.json(c)"
                    : scope === "user"
                      ? agent === "claude"
                        ? "留空使用当前用户 .claude.json"
                        : agent === "deepseek"
                          ? "留空使用 desktop profile 的 cordis.patch.yml"
                          : "留空使用当前用户的原生 MCP 配置"
                      : "完整 JSON 文件路径"
                }
              />
              <button
                type="button"
                className="button"
                onClick={async () => {
                  const selected = await open({
                    multiple: false,
                    filters: [
                      {
                        name: "原生 MCP 配置",
                        extensions:
                          agent === "deepseek"
                            ? ["yml", "yaml"]
                            : ["json", "jsonc"],
                      },
                    ],
                  });
                  if (typeof selected === "string") setPath(selected);
                }}
              >
                选择配置文件
              </button>
            </label>
          )}
        <p className="field-help">
          {agent === "opencode"
            ? "使用当前 OpenCode 配置格式，支持 JSON/JSONC 及空 MCP 配置，保留注释和其他设置。"
            : agent === "claude"
              ? "User 与 Local 使用 .claude.json 的不同位置；Local 按 Git 项目根目录定位。Project 使用所选目录的 .mcp.json，客户端审批及信任由 Claude Code 维护。插件与托管来源保持只读。"
              : agent === "deepseek"
                ? "按 profile 管理 cordis.patch.yml 中的 MCP 插件，可选择 desktop、web 或其他实际 profile。共享代理写入原生 stdio 入口。"
                : agent === "pi"
                  ? "使用 Pi 1.x 内置 MCP：用户 ~/.pi/agent/mcp.json（或 PI_CODING_AGENT_DIR），项目 .pi/mcp.json，均为严格 JSON。项目须由你在 Pi 中信任；写入后 /reload。第三方 MCP 扩展不在此适配范围。"
                  : agent === "qwenwork"
                    ? "使用当前用户 .qwenworkcn/mcp.json，支持预览、应用、启停与恢复；写入后重新启动千问办公。身份隔离或迁移存储须使用客户端导入。"
                    : agent === "workbuddy"
                      ? "用户与项目配置均为严格 JSON 的 .workbuddy/mcp.json。可预览写入、移除及恢复；启停、信任与工具策略请在 WorkBuddy 中维护。"
                      : "使用当前原生 MCP 格式，支持配置管理及共享代理。文件仅在应用预览后的变更时写入。"}
        </p>
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
        <footer className="modal-footer">
          <button type="button" className="button" onClick={onClose}>
            取消
          </button>
          <button className="button primary" disabled={pending}>
            {pending && <Loader2 className="spin" size={15} />}添加来源
          </button>
        </footer>
      </form>
    </Modal>
  );
}
