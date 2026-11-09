import React, { useEffect, useState } from "react";
import { ArrowRight, KeyRound, Loader2 } from "lucide-react";
import { Connection, Definition, rpc } from "./contracts";
import { Modal } from "./Modal";

export function Editor({
  connection,
  sharedAgents,
  onClose,
  onDone,
}: {
  connection?: Connection;
  sharedAgents: string[];
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const [name, setName] = useState(connection?.name || "");
  const [title, setTitle] = useState(connection?.title || "");
  const [type, setType] = useState("http");
  const [url, setUrl] = useState("");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [account, setAccount] = useState(connection?.accountLabel || "");
  const [token, setToken] = useState("");
  const [environment, setEnvironment] = useState("{}");
  const [customHeaders, setCustomHeaders] = useState("{}");
  const [cwd, setCwd] = useState("");
  const [definition, setDefinition] = useState<Definition>();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    if (connection) {
      setPending(true);
      rpc<{ connection?: Connection; definition: Definition }>(
        connection.detected
          ? "connections.source_detail"
          : "connections.detail",
        connection.detected
          ? { sourceId: connection.sourceId, sourceName: connection.name }
          : { id: connection.id },
      )
        .then((result) => {
          const current = result.connection || connection;
          setName(current.name);
          setTitle(current.title);
          setAccount(current.accountLabel);
          setDefinition(result.definition);
          setType(result.definition.url ? "http" : "stdio");
          setUrl(result.definition.url || "");
          setCommand(result.definition.command || "");
          setArgs(result.definition.args.join("\n"));
          setEnvironment(JSON.stringify(result.definition.env, null, 2));
          setCustomHeaders(JSON.stringify(result.definition.headers, null, 2));
          setCwd(result.definition.cwd || "");
        })
        .catch((error) => setError(error.message))
        .finally(() => setPending(false));
    }
  }, [connection]);
  const origin = (address: string) => {
    try {
      return new URL(address).origin;
    } catch {
      return address;
    }
  };
  const originChanged =
    type === "http" &&
    !!definition?.url &&
    !!url &&
    origin(definition.url) !== origin(url);
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    setPending(true);
    setError("");
    try {
      const next: Definition = {
        args: [],
        env: {},
        headers: {},
        enabled: true,
        oauth: false,
        ...definition,
        command: type === "stdio" ? command : undefined,
        url: type === "http" ? url : undefined,
      };
      if (type === "stdio") {
        next.args =
          definition && args === definition.args.join("\n")
            ? definition.args
            : args === ""
              ? []
              : args.split("\n");
        next.headers = {};
        next.oauth = false;
        next.clientId = undefined;
        next.clientSecret = undefined;
        next.oauthScopes = [];
        next.oauthCallbackPort = undefined;
        next.cwd = cwd || undefined;
        next.env = JSON.parse(environment);
      } else {
        if (definition?.command) {
          next.args = [];
          next.env = {};
          next.cwd = undefined;
        }
        next.headers = JSON.parse(customHeaders);
      }
      for (const map of [next.env, next.headers]) {
        if (
          !map ||
          Array.isArray(map) ||
          typeof map !== "object" ||
          Object.values(map).some((v) => typeof v !== "string")
        )
          throw new Error("环境变量与请求头需要键值均为字符串的 JSON 对象");
      }
      if (token && type === "http") {
        next.headers = Object.fromEntries(
          Object.entries(next.headers).filter(
            ([key]) => key.toLowerCase() !== "authorization",
          ),
        );
        next.headers.Authorization = token;
        next.oauth = false;
      }
      await rpc("connections.save", {
        ...(connection?.detected
          ? { sourceId: connection.sourceId, sourceName: connection.name }
          : { id: connection?.id }),
        name,
        title: title || name,
        accountLabel: account || "用户标签",
        definition: next,
      });
      await onDone();
    } catch (error) {
      setError((error as Error).message);
    } finally {
      setPending(false);
    }
  };
  return (
    <Modal
      title={
        connection?.detected
          ? "编辑并纳入托管"
          : connection
            ? "编辑服务连接"
            : "添加 MCP 服务"
      }
      onClose={onClose}
    >
      <form className="modal-body" onSubmit={(event) => void save(event)}>
        {connection?.detected && (
          <p className="field-help">
            保存后由 mcplink 托管此连接。原客户端配置保留在原处。
          </p>
        )}
        <fieldset
          className="editor-fields"
          disabled={pending || (!!connection && !definition)}
        >
          <div className="form-grid">
            <label className="field">
              <span>显示名称</span>
              <input
                value={title}
                onChange={(event) => setTitle(event.target.value)}
                placeholder="例如 腾讯文档 · 工作账号"
              />
            </label>
            <label className="field">
              <span>接入名称</span>
              <input
                required
                pattern={"[A-Za-z0-9_\\-]{1,80}"}
                value={name}
                onChange={(event) => setName(event.target.value)}
                placeholder="tencent_docs"
              />
            </label>
          </div>
          <label className="field">
            <span>服务类型</span>
            <select
              value={type}
              onChange={(event) => setType(event.target.value)}
            >
              <option value="http">HTTP MCP</option>
              <option value="stdio">本地进程 · STDIO</option>
            </select>
          </label>
          {type === "http" ? (
            <>
              <label className="field">
                <span>MCP 地址</span>
                <input
                  required
                  value={url}
                  onChange={(event) => setUrl(event.target.value)}
                  placeholder="https://example.com/mcp"
                />
              </label>
              {originChanged && (
                <div className="warning-banner" role="status">
                  保存后连接将暂停，旧地址的认证不会发送到新地址。请在账号页设置新凭据，或确认地址后重新连接。
                  {sharedAgents.length > 0 && (
                    <p>受影响的共享 Agent · {sharedAgents.join("、")}</p>
                  )}
                </div>
              )}
              <label className="field">
                <span>Authorization 值</span>
                <input
                  type="password"
                  autoComplete="off"
                  value={token}
                  onChange={(event) => setToken(event.target.value)}
                  placeholder={
                    connection
                      ? originChanged
                        ? "新地址的认证值"
                        : "留空保留现有认证"
                      : "免登录服务可留空，按服务要求填写完整值"
                  }
                />
              </label>
              <label className="field">
                <span>账号标签</span>
                <input
                  value={account}
                  onChange={(event) => setAccount(event.target.value)}
                  placeholder="工作账号（用于本机识别）"
                />
              </label>
            </>
          ) : (
            <>
              <label className="field">
                <span>启动命令</span>
                <input
                  required
                  value={command}
                  onChange={(event) => setCommand(event.target.value)}
                  placeholder="npx 或可执行文件完整路径"
                />
              </label>
              <label className="field">
                <span>启动参数，每行一个。编号标记保留对应的已存参数。</span>
                <textarea
                  value={args}
                  onChange={(event) => setArgs(event.target.value)}
                  placeholder={
                    "-y\n@modelcontextprotocol/server-filesystem\n/path/to/folder"
                  }
                  rows={4}
                />
              </label>
            </>
          )}
          <details className="oauth-option">
            <summary>更多连接设置</summary>
            {type === "stdio" ? (
              <>
                <label className="field">
                  <span>工作目录</span>
                  <input
                    value={cwd}
                    onChange={(e) => setCwd(e.target.value)}
                    placeholder="可留空"
                  />
                </label>
                <label className="field">
                  <span>环境变量 · JSON</span>
                  <textarea
                    rows={4}
                    value={environment}
                    onChange={(e) => setEnvironment(e.target.value)}
                    spellCheck={false}
                  />
                </label>
              </>
            ) : (
              <label className="field">
                <span>自定义请求头 · JSON</span>
                <textarea
                  rows={4}
                  value={customHeaders}
                  onChange={(e) => setCustomHeaders(e.target.value)}
                  spellCheck={false}
                />
              </label>
            )}
            <p className="field-help">
              •••••••• 表示保留已存值；变量引用可使用 $&#123;env:NAME&#125;。
            </p>
          </details>
        </fieldset>
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
        <footer className="modal-footer">
          <button type="button" className="button" onClick={onClose}>
            取消
          </button>
          <button
            className="button primary"
            disabled={pending || (!!connection && !definition)}
          >
            {pending && <Loader2 className="spin" size={15} />}
            {connection?.detected ? "保存并纳入托管" : "保存连接"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}

export function CredentialDialog({
  connection,
  onClose,
  onDone,
}: {
  connection: Connection;
  onClose: () => void;
  onDone: () => Promise<void>;
}) {
  const [token, setToken] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [oauth, setOauth] = useState(false);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [scopes, setScopes] = useState("");
  const [callbackPort, setCallbackPort] = useState("");
  const [oauthLoaded, setOauthLoaded] = useState(false);
  const [oauthSaved, setOauthSaved] = useState(false);
  useEffect(() => {
    let active = true;
    void rpc<{ definition: Definition }>("connections.detail", {
      id: connection.id,
    })
      .then(({ definition }) => {
        if (!active) return;
        setClientId(definition.clientId || "");
        setClientSecret(definition.clientSecret || "");
        setScopes((definition.oauthScopes || []).join(" "));
        setCallbackPort(definition.oauthCallbackPort?.toString() || "");
        setOauthLoaded(true);
      })
      .catch((error) => {
        if (active) setError(error.message);
      });
    return () => {
      active = false;
    };
  }, [connection.id]);
  const saveOauth = async () => {
    const result = await rpc<{ definition: Definition }>("oauth.configure", {
      id: connection.id,
      settings: {
        clientId: clientId.trim() || null,
        clientSecret: clientSecret || null,
        scopes: scopes.trim() ? scopes.trim().split(/\s+/) : [],
        callbackPort: callbackPort ? Number(callbackPort) : null,
      },
    });
    setClientSecret(result.definition.clientSecret || "");
    setOauthSaved(true);
  };
  const update = async (event: React.FormEvent) => {
    event.preventDefault();
    setPending(true);
    setError("");
    try {
      await rpc("credentials.update", {
        id: connection.id,
        header: "Authorization",
        value: token,
      });
      await onDone();
    } catch (error) {
      setError((error as Error).message);
    } finally {
      setPending(false);
    }
  };
  return (
    <Modal title={`${connection.title} · 认证`} onClose={onClose}>
      <form className="modal-body" onSubmit={(event) => void update(event)}>
        <p className="muted">
          认证保存在系统凭据库，使用这项共享连接的 Agent 会共用更新后的值。
        </p>
        {connection.endpoint.includes("docs.qq.com") && (
          <button
            className="button full"
            type="button"
            onClick={() =>
              void rpc("browser.open", {
                url: "https://docs.qq.com/open/auth/mcp.html",
              }).catch((error) => setError(error.message))
            }
          >
            打开腾讯文档授权页面
            <ArrowRight size={14} />
          </button>
        )}
        <label className="field">
          <span>Authorization 完整值</span>
          <input
            required
            type="password"
            autoComplete="off"
            value={token}
            onChange={(event) => setToken(event.target.value)}
            placeholder="粘贴服务提供的令牌或完整 Bearer 值"
          />
        </label>
        <p className="field-help">
          按上游要求原样保存，更新后可检查连接并查看实际工具。
        </p>
        <details className="oauth-option">
          <summary>服务使用 OAuth 浏览器授权</summary>
          <p className="muted">
            适用于提供标准 MCP OAuth
            元数据的服务；可使用动态注册或你自己的预注册应用。
          </p>
          <label className="field">
            <span>Client ID · 预注册应用</span>
            <input
              value={clientId}
              onChange={(event) => {
                setClientId(event.target.value);
                setOauthSaved(false);
              }}
              placeholder="支持动态注册时可留空"
              autoComplete="off"
            />
          </label>
          <label className="field">
            <span>客户端机密 · 按注册应用要求填写</span>
            <input
              type="password"
              value={clientSecret}
              onChange={(event) => {
                setClientSecret(event.target.value);
                setOauthSaved(false);
              }}
              autoComplete="off"
              placeholder="保存在系统凭据库；清空可删除已存值"
            />
          </label>
          <label className="field">
            <span>授权 scope · 用空格分隔</span>
            <input
              value={scopes}
              onChange={(event) => {
                setScopes(event.target.value);
                setOauthSaved(false);
              }}
              placeholder="留空使用服务建议的 scope"
            />
          </label>
          <label className="field">
            <span>本地回调端口 · 可选</span>
            <input
              type="number"
              min={1}
              max={65535}
              value={callbackPort}
              onChange={(event) => {
                setCallbackPort(event.target.value);
                setOauthSaved(false);
              }}
              placeholder="留空自动分配，路径为 /callback"
            />
          </label>
          <p className="field-help">
            •••••••• 表示保留已存机密。更换 OAuth
            应用或权限后需要重新授权；授权准备失败会保留原有认证。
          </p>
          <button
            className="button"
            type="button"
            disabled={pending || !oauthLoaded}
            onClick={async () => {
              setPending(true);
              setError("");
              try {
                await saveOauth();
                await onDone();
              } catch (error) {
                setError((error as Error).message);
              } finally {
                setPending(false);
              }
            }}
          >
            保存 OAuth 设置
          </button>
          {oauthSaved && <p className="muted">OAuth 客户端设置已保存。</p>}
          <button
            className="button"
            type="button"
            disabled={pending || !oauthLoaded}
            onClick={async () => {
              setPending(true);
              setError("");
              try {
                await saveOauth();
                const result = await rpc<{ url: string }>("oauth.start", {
                  id: connection.id,
                });
                await rpc("browser.open", { url: result.url });
                setOauth(true);
              } catch (error) {
                setError((error as Error).message);
              } finally {
                setPending(false);
              }
            }}
          >
            <KeyRound size={15} />
            在浏览器授权
          </button>
          {oauth && (
            <p className="muted">
              已打开浏览器，完成授权后返回并刷新账号状态。
            </p>
          )}
        </details>
        {error && (
          <div className="warning-banner" role="alert">
            {error}
          </div>
        )}
        <footer className="modal-footer">
          <button type="button" className="button" onClick={onClose}>
            关闭
          </button>
          <button className="button primary" disabled={pending || !token}>
            {pending && <Loader2 className="spin" size={15} />}更新认证
          </button>
        </footer>
      </form>
    </Modal>
  );
}
