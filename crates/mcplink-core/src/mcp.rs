use crate::{Engine, config, model::Connection};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use rmcp::{
    RoleClient, ServiceExt,
    model::*,
    service::{ClientInitializeError, Peer, RunningService},
    transport::{
        StreamableHttpClientTransport, TokioChildProcess,
        auth::{
            AuthClient, AuthError, AuthorizationManager, AuthorizationRequest,
            CredentialRefreshGuard, CredentialStore, OAuthClientConfig, OAuthState,
            StoredCredentials,
        },
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Mutex as AsyncMutex};

type Session = RunningService<RoleClient, ClientConfig>;

fn client_config() -> ClientConfig {
    // ServiceExt::serve performs legacy initialize, not 2026 server/discover.
    // Advertise the implemented legacy version rather than the SDK's newest
    // default, which produces invalid sessions on the GitHub remote server.
    let mut config = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("mcplink", env!("CARGO_PKG_VERSION")),
    );
    config.protocol_version = ProtocolVersion::V_2025_11_25;
    config
}

fn http_endpoint(raw: &str, project: Option<&std::path::Path>) -> Result<String> {
    let endpoint = config::resolve(raw, project)?;
    let url = url::Url::parse(&endpoint).context("MCP 地址无效")?;
    let loopback = match url.host() {
        Some(url::Host::Domain(host)) => host == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        bail!("共享 HTTP 连接需要 HTTPS，或本机 loopback 地址");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("请通过认证字段配置凭据");
    }
    Ok(endpoint)
}
type SessionSlot = Arc<AsyncMutex<Option<(u64, Session)>>>;
#[derive(Default)]
pub struct McpPool {
    sessions: Mutex<HashMap<String, SessionSlot>>,
    refresh: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    shutdown_guard: AsyncMutex<()>,
}

impl McpPool {
    pub fn refresh_lock(&self, id: &str) -> Arc<AsyncMutex<()>> {
        self.refresh
            .lock()
            .unwrap()
            .entry(id.into())
            .or_default()
            .clone()
    }

    pub fn count(&self) -> usize {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .filter(|slot| {
                slot.try_lock()
                    .is_ok_and(|s| s.as_ref().is_some_and(|(_, session)| !session.is_closed()))
            })
            .count()
    }

    pub async fn invalidate(&self, id: &str) {
        let slot = self.sessions.lock().unwrap().remove(id);
        if let Some(slot) = slot
            && let Some((_, session)) = slot.lock().await.take()
        {
            session.cancellation_token().cancel();
        }
    }

    pub async fn clear(&self) {
        let ids: Vec<_> = self.sessions.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.invalidate(&id).await;
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        let _cleanup = self.shutdown_guard.lock().await;
        let slots: Vec<_> = self
            .sessions
            .lock()
            .unwrap()
            .drain()
            .map(|(_, slot)| slot)
            .collect();
        let mut tasks = tokio::task::JoinSet::new();
        for slot in slots {
            tasks.spawn(async move {
                if let Some((_, session)) = slot.lock().await.take() {
                    tokio::time::timeout(Duration::from_secs(20), session.cancel())
                        .await
                        .context("上游服务退出清理超时")?
                        .context("上游服务退出失败")?;
                }
                Ok::<_, anyhow::Error>(())
            });
        }
        let mut failure = None;
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result.context("上游退出任务失败").and_then(|value| value) {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub async fn peer(&self, engine: Arc<Engine>, id: &str) -> Result<Peer<RoleClient>> {
        let connection = engine.store.connection(id)?;
        if connection.members.is_some() {
            bail!("聚合入口通过托管服务路由，不能作为上游建立会话");
        }
        if !connection.active {
            bail!("连接已断开，请在账号页重新连接");
        }
        let slot = {
            let mut sessions = self.sessions.lock().unwrap();
            if std::ptr::eq(self, &engine.pool)
                && engine.paused.load(std::sync::atomic::Ordering::SeqCst)
            {
                bail!("共享后台已暂停，请在 mcplink 恢复");
            }
            sessions.entry(id.into()).or_default().clone()
        };
        let mut shutdown = engine.shutdown.subscribe();
        let mut slot = tokio::select! {
            _ = shutdown.wait_for(|stopping| *stopping) => bail!("共享后台正在退出"),
            slot = slot.lock() => slot,
        };
        if let Some((revision, session)) = slot.as_ref()
            && *revision == connection.revision
            && !session.is_closed()
        {
            return Ok(session.peer().clone());
        }
        let definition = engine.definition(&connection)?;
        if definition.oauth && connection.status == "needs_auth" {
            bail!("需要 OAuth 授权，请在账号页登录");
        }
        let started = tokio::time::timeout(
            Duration::from_secs(definition.startup_timeout.unwrap_or(40).clamp(5, 300)),
            async {
                if let Some(endpoint) = &definition.url {
                    let endpoint = http_endpoint(endpoint, definition.origin_project.as_deref())?;
                    let client = reqwest::Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .connect_timeout(Duration::from_secs(15))
                        .build()
                        .context("无法建立 HTTP 客户端")?;
                    let mut transport =
                        StreamableHttpClientTransportConfig::with_uri(endpoint.clone());
                    transport.reinit_on_expired_session = false;
                    for (key, value) in &definition.headers {
                        let key = reqwest::header::HeaderName::from_bytes(key.as_bytes())
                            .context("HTTP 头名称无效")?;
                        let value = reqwest::header::HeaderValue::from_str(&config::resolve(
                            value,
                            definition.origin_project.as_deref(),
                        )?)
                        .context("HTTP 认证字段无效")?;
                        transport.custom_headers.insert(key, value);
                    }
                    if definition.oauth {
                        // A refresh can immediately revoke the access token on an
                        // outstanding POST. Queue ordinary POSTs on this OAuth session.
                        transport.max_concurrent_requests = 1;
                        let mut manager = AuthorizationManager::new(endpoint)
                            .await
                            .context("无法发现 OAuth 认证服务")?;
                        manager.set_credential_store(VaultCredentials::new(
                            engine.clone(),
                            &connection,
                        ));
                        if !manager
                            .initialize_from_store()
                            .await
                            .context("无法读取 OAuth 授权，请重新登录")?
                        {
                            bail!("需要 OAuth 授权，请在账号页登录");
                        }
                        let (stored_client, _) = manager
                            .get_credentials()
                            .await
                            .context("无法读取 OAuth 客户端身份，请重新登录")?;
                        if definition
                            .client_id
                            .as_ref()
                            .is_some_and(|id| id != &stored_client)
                        {
                            bail!("OAuth 应用身份已变化，请重新登录");
                        }
                        if let Some(secret) = &definition.client_secret {
                            let redirect = format!(
                                "http://127.0.0.1:{}/callback",
                                definition.oauth_callback_port.unwrap_or(0)
                            );
                            manager
                                .configure_client(
                                    OAuthClientConfig::new(stored_client, redirect)
                                        .with_client_secret(secret.clone())
                                        .with_scopes(definition.oauth_scopes.clone()),
                                )
                                .context("无法恢复 OAuth 客户端认证，请重新登录")?;
                        }
                        let transport = StreamableHttpClientTransport::with_client(
                            AuthClient::new(client, manager),
                            transport,
                        );
                        Ok(client_config()
                            .serve(transport)
                            .await
                            .map_err(connection_error)?)
                    } else {
                        let transport =
                            StreamableHttpClientTransport::with_client(client, transport);
                        Ok(client_config()
                            .serve(transport)
                            .await
                            .map_err(connection_error)?)
                    }
                } else {
                    let executable = definition.command.as_deref().context("未配置启动命令")?;
                    let mut command = tokio::process::Command::new(config::resolve(
                        executable,
                        definition.origin_project.as_deref(),
                    )?);
                    #[cfg(target_os = "macos")]
                    command.env("PATH", crate::process_group::search_path()?);
                    for argument in &definition.args {
                        command.arg(config::resolve(
                            argument,
                            definition.origin_project.as_deref(),
                        )?);
                    }
                    for (key, value) in &definition.env {
                        command.env(
                            key,
                            config::resolve(value, definition.origin_project.as_deref())?,
                        );
                    }
                    if let Some(cwd) = &definition.cwd {
                        let cwd = std::path::PathBuf::from(config::resolve(
                            cwd,
                            definition.origin_project.as_deref(),
                        )?);
                        command.current_dir(if cwd.is_absolute() {
                            cwd
                        } else {
                            definition
                                .origin_project
                                .as_ref()
                                .map(|project| project.join(&cwd))
                                .unwrap_or(cwd)
                        });
                    } else if let Some(project) = &definition.origin_project {
                        command.current_dir(project);
                    }
                    command.stderr(Stdio::null()).kill_on_drop(true);
                    #[cfg(windows)]
                    let command = {
                        use process_wrap::tokio::{
                            CommandWrap, CreationFlags, JobObject, KillOnDrop,
                        };
                        // Keep the complete upstream process tree hidden and owned by the daemon.
                        // Closing the job on a daemon crash also prevents orphaned service processes.
                        let mut wrapped = CommandWrap::from(command);
                        wrapped.wrap(CreationFlags(
                            windows::Win32::System::Threading::CREATE_NO_WINDOW,
                        ));
                        wrapped
                            .wrap(KillOnDrop)
                            .wrap(JobObject)
                            .wrap(crate::process_group::KillGroupOnDrop);
                        wrapped
                    };
                    #[cfg(target_os = "macos")]
                    let command = {
                        use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessGroup};
                        let mut wrapped = CommandWrap::from(command);
                        wrapped
                            .wrap(KillOnDrop)
                            .wrap(ProcessGroup::leader())
                            .wrap(crate::process_group::KillGroupOnDrop);
                        wrapped
                    };
                    let transport = TokioChildProcess::new(command)
                        .context("无法启动 MCP 服务，请检查命令和运行环境")?;
                    Ok(client_config()
                        .serve(transport)
                        .await
                        .map_err(connection_error)?)
                }
            },
        );
        let started = tokio::select! {
            _ = shutdown.wait_for(|stopping| *stopping) => bail!("共享后台正在退出"),
            result = started => result.context("MCP 连接超时，请检查服务状态")??,
        };
        let current = engine.store.connection(id)?;
        if !current.active || current.revision != connection.revision {
            started.cancellation_token().cancel();
            bail!("连接已更新，请重新连接");
        }
        let peer = started.peer().clone();
        *slot = Some((connection.revision, started));
        Ok(peer)
    }

    pub async fn request(&self, engine: Arc<Engine>, id: &str, request: Value) -> Result<Value> {
        self.request_at_revision(engine, id, request, None).await
    }

    pub async fn request_at_revision(
        &self,
        engine: Arc<Engine>,
        id: &str,
        request: Value,
        revision: Option<u64>,
    ) -> Result<Value> {
        if revision.is_some() && engine.paused.load(std::sync::atomic::Ordering::SeqCst) {
            bail!("共享后台已暂停，请在 mcplink 恢复");
        }
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        if !matches!(
            method,
            "tools/list"
                | "tools/call"
                | "resources/list"
                | "resources/templates/list"
                | "resources/read"
                | "prompts/list"
                | "prompts/get"
        ) {
            bail!("此 MCP 能力暂未开放转发");
        }
        let connection = engine.store.connection(id)?;
        if revision.is_some_and(|expected| expected != connection.revision) {
            bail!(crate::invocation::ToolFailure {
                code: crate::invocation::ToolErrorCode::ConnectionChanged,
                details: Value::Null
            });
        }
        if connection.members.is_some() {
            return crate::gateway::request(engine, &connection, request).await;
        }
        let definition = engine.definition(&connection)?;
        if method.starts_with("resources/") && definition.resources_disabled
            || method.starts_with("prompts/") && definition.prompts_disabled
        {
            bail!("此连接未启用该 MCP 能力");
        }
        if method == "tools/call"
            && !tool_allowed(
                &definition,
                request
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
        {
            bail!("此工具不在连接启用清单中");
        }
        let list = method == "tools/list";
        let invoking = method == "tools/call";
        let peer = self.peer(engine.clone(), id).await?;
        if let Some(expected) = revision {
            let current = engine.store.connection(id)?;
            if current.revision != expected || !current.active {
                bail!(crate::invocation::ToolFailure {
                    code: crate::invocation::ToolErrorCode::ConnectionChanged,
                    details: Value::Null
                });
            }
        }
        let request =
            serde_json::from_value::<ClientRequest>(request).context("MCP 请求参数格式不正确")?;
        let limit = definition.timeout.unwrap_or(120).clamp(5, 600);
        let mut shutdown = engine.shutdown.subscribe();
        let outcome = tokio::select! {
            _ = shutdown.wait_for(|stopping| *stopping) => {
                bail!("后台正在退出，调用已中断；执行结果需核对，不要自动重试");
            },
            outcome = tokio::time::timeout(Duration::from_secs(limit), peer.send_request(request)) => outcome,
        };
        match outcome {
            Ok(Ok(result)) => {
                let mut value = serde_json::to_value(result)?;
                if list && let Some(tools) = value.get_mut("tools").and_then(Value::as_array_mut) {
                    tools.retain(|tool| {
                        tool_allowed(
                            &definition,
                            tool.get("name").and_then(Value::as_str).unwrap_or(""),
                        )
                    });
                }
                Ok(value)
            }
            Ok(Err(rmcp::ServiceError::McpError(error))) => Ok(json!({"rpcError":error})),
            Ok(Err(error)) => {
                let diagnostic = connection_error(error);
                if invoking {
                    Err(anyhow::anyhow!(
                        "工具调用中断，执行结果需核对；不要自动重试。{diagnostic}"
                    ))
                } else {
                    Err(diagnostic)
                }
            }
            Err(_) => bail!("工具请求超时，执行结果待核对，请确认结果后再决定是否重试"),
        }
    }
}

pub fn tool_allowed(definition: &crate::model::Definition, name: &str) -> bool {
    let hermes = definition.original_agent == Some(crate::model::Agent::Hermes);
    let matches = |pattern: &String| {
        if hermes {
            glob::Pattern::new(pattern).is_ok_and(|pattern| pattern.matches(name))
        } else {
            pattern == name
        }
    };
    let include = definition.include_specified || !definition.include_tools.is_empty();
    if include && !definition.include_tools.iter().any(matches) {
        return false;
    }
    if hermes && include {
        return true;
    }
    !definition.exclude_tools.iter().any(matches)
}

fn requires_authentication(error: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(initialization) = error.downcast_ref::<ClientInitializeError>() {
        if initialization.is_authorization_required() {
            return true;
        }
        if let ClientInitializeError::LegacyFallbackFailed { discover, fallback } = initialization {
            return requires_authentication(discover.as_ref())
                || requires_authentication(fallback.as_ref());
        }
    }
    let detail = error.to_string().to_ascii_lowercase();
    // SDK wrappers can omit the HTTP status or both startup failures in Display.
    [
        "401",
        "unauthorized",
        "auth required",
        "authorization required",
        "invalid_token",
        "400006",
    ]
    .iter()
    .any(|message| detail.contains(message))
        || error.source().is_some_and(requires_authentication)
}

fn connection_error(error: impl std::error::Error + 'static) -> anyhow::Error {
    if requires_authentication(&error) {
        anyhow::anyhow!("服务要求认证，请在账号页设置凭据或重新登录")
    } else {
        anyhow::anyhow!("MCP 连接中断或协议不兼容，请检查端点与服务状态")
    }
}

struct VaultCredentials {
    engine: Arc<Engine>,
    id: String,
    revision: u64,
    lock: Arc<AsyncMutex<()>>,
}

impl VaultCredentials {
    fn new(engine: Arc<Engine>, connection: &Connection) -> Self {
        let lock = engine.pool.refresh_lock(&connection.id);
        Self {
            engine,
            id: connection.id.clone(),
            revision: connection.revision,
            lock,
        }
    }

    fn check(&self) -> std::result::Result<(), AuthError> {
        let current = self
            .engine
            .store
            .connection(&self.id)
            .map_err(|_| AuthError::CredentialStoreError("连接不存在".into()))?;
        if !current.active || current.revision != self.revision {
            return Err(AuthError::CredentialStoreError(
                "授权会话已变化，请重新登录".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl CredentialStore for VaultCredentials {
    async fn load(&self) -> std::result::Result<Option<StoredCredentials>, AuthError> {
        self.check()?;
        self.engine
            .vault
            .get(&format!("oauth/{}", self.id))
            .map_err(|_| AuthError::CredentialStoreError("系统凭据库不可用".into()))?
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|_| AuthError::CredentialStoreError("授权数据格式有误".into()))
            })
            .transpose()
    }
    async fn save(&self, value: StoredCredentials) -> std::result::Result<(), AuthError> {
        self.check()?;
        let value = serde_json::to_string(&value)
            .map_err(|_| AuthError::CredentialStoreError("授权数据无法编码".into()))?;
        self.engine
            .vault
            .set(&format!("oauth/{}", self.id), &value)
            .map_err(|_| AuthError::CredentialStoreError("系统凭据库不可用".into()))
    }
    async fn clear(&self) -> std::result::Result<(), AuthError> {
        self.check()?;
        self.engine
            .vault
            .delete(&format!("oauth/{}", self.id))
            .map_err(|_| AuthError::CredentialStoreError("系统凭据库不可用".into()))
    }
    async fn acquire_refresh_guard(
        &self,
    ) -> std::result::Result<Option<CredentialRefreshGuard>, AuthError> {
        Ok(Some(CredentialRefreshGuard::new(
            self.lock.clone().lock_owned().await,
        )))
    }
}

pub async fn configure_oauth(engine: Arc<Engine>, params: &Value) -> Result<Value> {
    let _mutation = engine.mutation.lock().await;
    let id = params["id"].as_str().context("缺少连接 ID")?;
    let mut connection = engine.store.connection(id)?;
    let mut definition = engine.definition(&connection)?;
    if definition.url.is_none() {
        bail!("OAuth 客户端配置只适用于 HTTP 连接");
    }
    let settings = params["settings"]
        .as_object()
        .context("OAuth 设置须为 JSON 对象")?;
    if settings
        .keys()
        .any(|key| !["clientId", "clientSecret", "scopes", "callbackPort"].contains(&key.as_str()))
    {
        bail!("OAuth 设置含不支持的字段");
    }
    let previous = serde_json::to_value(&definition)?;
    if let Some(value) = settings.get("clientId") {
        definition.client_id = if value.is_null() {
            None
        } else {
            Some(
                value
                    .as_str()
                    .context("clientId 须为字符串或 null")?
                    .to_owned(),
            )
        };
    }
    if let Some(value) = settings.get("clientSecret") {
        if value.is_null() {
            definition.client_secret = None;
        } else {
            let secret = value.as_str().context("clientSecret 须为字符串或 null")?;
            if secret == "••••••••" {
                if definition.client_secret.is_none() {
                    bail!("当前未保存 OAuth 客户端机密，请重新填写");
                }
            } else {
                definition.client_secret = Some(secret.to_owned());
            }
        }
    }
    if let Some(value) = settings.get("scopes") {
        definition.oauth_scopes = value
            .as_array()
            .context("scopes 须为字符串数组")?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("scopes 须为字符串数组")
            })
            .collect::<Result<Vec<_>>>()?;
        let mut seen = std::collections::HashSet::new();
        definition
            .oauth_scopes
            .retain(|scope| seen.insert(scope.clone()));
    }
    if let Some(value) = settings.get("callbackPort") {
        definition.oauth_callback_port = if value.is_null() {
            None
        } else {
            Some(
                value
                    .as_u64()
                    .and_then(|port| u16::try_from(port).ok())
                    .context("callbackPort 须为 1–65535 或 null")?,
            )
        };
    }
    config::validate(&definition)?;
    let changed = previous != serde_json::to_value(&definition)?;
    if changed {
        engine.save_definition(&mut connection, &definition)?;
        engine.pool.invalidate(id).await;
    }
    Ok(
        json!({"status":if changed {"configured"}else{"unchanged"},"connection":connection.public_value(),"definition":connection.editable_definition(&definition)}),
    )
}

pub async fn start_oauth(engine: Arc<Engine>, id: &str) -> Result<Value> {
    let _mutation = engine.mutation.lock().await;
    let mut connection = engine.store.connection(id)?;
    let mut definition = engine.definition(&connection)?;
    let endpoint = http_endpoint(
        definition
            .url
            .as_deref()
            .context("OAuth 需要 HTTP MCP 服务")?,
        definition.origin_project.as_deref(),
    )?;
    config::validate(&definition)?;
    let refresh = engine.pool.refresh_lock(id);
    let _refresh = refresh.lock().await;
    let listener = TcpListener::bind((
        std::net::Ipv4Addr::LOCALHOST,
        definition.oauth_callback_port.unwrap_or(0),
    ))
    .await
    .context("无法建立本地授权回调")?;
    let address = listener.local_addr()?;
    let redirect = format!("http://127.0.0.1:{}/callback", address.port());
    let (mut state, auth_url) = tokio::time::timeout(Duration::from_secs(45), async {
        let mut manager = AuthorizationManager::new(endpoint)
            .await
            .context("无法发现上游 OAuth 元数据")?;
        let metadata = manager
            .resolve_metadata()
            .await
            .context("无法发现上游 OAuth 元数据")?
            .metadata;
        if definition.client_id.is_none() && metadata.registration_endpoint.is_none() {
            bail!("上游不支持动态客户端注册；请先配置你注册应用的 clientId 及其要求的客户端机密");
        }
        let github = url::Url::parse(&metadata.authorization_endpoint).is_ok_and(|url| {
            url.host_str() == Some("github.com") && url.path() == "/login/oauth/authorize"
        });
        if github && definition.client_secret.is_none() {
            bail!("GitHub 授权码流程需要自有注册应用的客户端机密，请先配置 OAuth 客户端");
        }
        manager.set_metadata(metadata);
        // Prepare with the SDK's in-memory store. Discovery/registration failures
        // must not delete an existing Token or grant before a URL can be returned.
        let mut state = OAuthState::Unauthorized(manager);
        let mut request = AuthorizationRequest::new(&redirect)
            .with_client_name("mcplink")
            .with_initial_scope_selection();
        if let Some(client) = &definition.client_id {
            request = request.with_preregistered_client(client.clone());
        }
        if let Some(secret) = &definition.client_secret {
            request = request.with_client_secret(secret.clone());
        }
        if !definition.oauth_scopes.is_empty() {
            request = request.with_scopes(definition.oauth_scopes.clone());
        }
        state
            .start_authorization(request)
            .await
            .context("无法准备 OAuth 授权，请核对注册应用、回调与客户端认证")?;
        let url = state
            .get_authorization_url()
            .await
            .context("无法创建浏览器授权地址")?;
        Ok::<_, anyhow::Error>((state, url))
    })
    .await
    .context("OAuth 授权准备超时，原有认证保持不变")??;
    if !matches!(state, OAuthState::Session(_)) {
        bail!("OAuth 授权会话未就绪");
    }
    definition.oauth = true;
    definition
        .headers
        .retain(|key, _| !key.eq_ignore_ascii_case("authorization"));
    connection.active = true;
    connection.auth_kind = "oauth".into();
    connection.status = "authorizing".into();
    engine.save_definition(&mut connection, &definition)?;
    engine.vault.delete(&format!("oauth/{id}"))?;
    engine.pool.invalidate(id).await;
    if let OAuthState::Session(session) = &mut state {
        session
            .auth_manager
            .set_credential_store(VaultCredentials::new(engine.clone(), &connection));
    }
    let state = Arc::new(AsyncMutex::new(state));
    let (finished, mut completed) = tokio::sync::mpsc::channel::<()>(1);
    let callback_engine = engine.clone();
    let callback_id = id.to_owned();
    let revision = connection.revision;
    let expiry_engine = engine.clone();
    let expiry_id = id.to_owned();
    let expected_host = format!("127.0.0.1:{}", address.port());
    let router = axum::Router::new().route(
        "/callback",
        axum::routing::get(
            move |headers: axum::http::HeaderMap, uri: axum::http::Uri| {
                let state = state.clone();
                let engine = callback_engine.clone();
                let id = callback_id.clone();
                let done = finished.clone();
                let host = expected_host.clone();
                async move {
                    if headers.get("host").and_then(|v| v.to_str().ok()) != Some(host.as_str()) {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::response::Html("授权回调来源无效"),
                        );
                    }
                    let _mutation = engine.mutation.lock().await;
                    let refresh = engine.pool.refresh_lock(&id);
                    let _refresh = refresh.lock().await;
                    if !engine
                        .store
                        .connection(&id)
                        .is_ok_and(|c| c.active && c.revision == revision)
                    {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::response::Html("授权会话已失效，请重新登录"),
                        );
                    }
                    let callback = format!("http://{host}{uri}");
                    let result = state.lock().await.handle_callback_url(&callback).await;
                    if result.is_err() {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::response::Html("授权校验未通过，请返回 mcplink 重新登录"),
                        );
                    }
                    if let Ok(mut connection) = engine.store.connection(&id) {
                        connection.status = "configured".into();
                        connection.last_error = None;
                        let _ = engine.store.save_connection(&connection);
                    }
                    let _ = done.send(()).await;
                    (
                        axum::http::StatusCode::OK,
                        axum::response::Html("授权已完成，可以返回 mcplink 检查连接。"),
                    )
                }
            },
        ),
    );
    tokio::spawn(async move {
        let shutdown = async move {
            tokio::select! {_=tokio::time::sleep(Duration::from_secs(300))=>{
                let _mutation=expiry_engine.mutation.lock().await;
                if let Ok(mut current)=expiry_engine.store.connection(&expiry_id)&& current.revision==revision&&current.status=="authorizing"{current.status="needs_auth".into();current.last_error=Some("浏览器授权已过期，请重新登录".into());let _=expiry_engine.store.save_connection(&current);}
            },_=completed.recv()=>{}}
        };
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown)
            .await;
    });
    Ok(json!({"url":auth_url,"expiresAt":crate::model::now()+300}))
}
