use crate::{Engine, config, lifecycle, model};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    response::Html,
    routing::get,
};
use rmcp::transport::auth::{
    AuthError, AuthorizationCallback, AuthorizationRequest, CredentialStore,
    InMemoryCredentialStore, OAuthState,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;
use url::Url;

const CALLBACK_LIFETIME: Duration = Duration::from_secs(300);

#[derive(Default, Clone)]
pub(crate) struct Authorizations(Arc<Mutex<Registry>>);

#[derive(Default)]
struct Registry {
    attempts: HashMap<String, Arc<Attempt>>,
    ports: HashMap<String, Weak<AsyncMutex<()>>>,
}

struct Attempt {
    cancelled: CancellationToken,
    port: Arc<AsyncMutex<()>>,
}

struct Lease {
    authorizations: Authorizations,
    id: String,
    attempt: Arc<Attempt>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut registry = self.authorizations.0.lock().unwrap();
        if registry
            .attempts
            .get(&self.id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.attempt))
        {
            registry.attempts.remove(&self.id);
        }
    }
}

impl Authorizations {
    fn reserve(&self, id: &str) -> Lease {
        let mut registry = self.0.lock().unwrap();
        if let Some(previous) = registry.attempts.get(id) {
            previous.cancelled.cancel();
        }
        registry.ports.retain(|_, port| port.strong_count() != 0);
        let port = registry
            .ports
            .get(id)
            .and_then(Weak::upgrade)
            .unwrap_or_default();
        registry.ports.insert(id.to_owned(), Arc::downgrade(&port));
        let attempt = Arc::new(Attempt {
            cancelled: CancellationToken::new(),
            port,
        });
        registry.attempts.insert(id.to_owned(), attempt.clone());
        Lease {
            authorizations: self.clone(),
            id: id.to_owned(),
            attempt,
        }
    }

    pub(crate) fn cancel(&self, id: &str) {
        if let Some(attempt) = self.0.lock().unwrap().attempts.get(id) {
            attempt.cancelled.cancel();
        }
    }

    pub(crate) async fn shutdown(&self) -> Result<()> {
        let ports: Vec<_> = {
            let registry = self.0.lock().unwrap();
            for attempt in registry.attempts.values() {
                attempt.cancelled.cancel();
            }
            registry.ports.values().filter_map(Weak::upgrade).collect()
        };
        tokio::time::timeout(Duration::from_secs(4), async {
            for port in ports {
                let _closed = port.lock().await;
            }
        })
        .await
        .context("本地 OAuth 回调退出超时")?;
        Ok(())
    }
}

struct Callback {
    engine: Arc<Engine>,
    id: String,
    revision: u64,
    host: String,
    csrf: String,
    oauth: AsyncMutex<OAuthState>,
    grant: InMemoryCredentialStore,
    cancelled: CancellationToken,
    complete: CancellationToken,
}

impl Callback {
    async fn commit(&self) -> Result<()> {
        let grant = self
            .grant
            .load()
            .await?
            .context("OAuth 授权返回后缺少凭据")?;
        let _mutation = tokio::select! {
            biased;
            _ = self.cancelled.cancelled() => bail!("授权会话已取消"),
            guard = self.engine.mutation.lock() => guard,
        };
        let connection = self.engine.store.connection(&self.id)?;
        ensure!(
            !*self.engine.shutdown.borrow()
                && !self.cancelled.is_cancelled()
                && connection.active
                && connection.revision == self.revision,
            "授权会话已变化"
        );
        let edit = lifecycle::CredentialEdit::begin(&self.engine, &self.id).await;
        let result = async {
            ensure!(!*self.engine.shutdown.borrow(), "共享后台正在退出");
            lifecycle::VaultCredentials::new(self.engine.clone(), &connection)
                .save(grant)
                .await?;
            lifecycle::complete_oauth(&self.engine, &self.id, self.revision)
        }
        .await;
        edit.finish(result)
    }
}

type CallbackReply = (StatusCode, Html<&'static str>);

async fn callback(
    State(state): State<Arc<Callback>>,
    headers: HeaderMap,
    uri: Uri,
) -> CallbackReply {
    let invalid = (
        StatusCode::BAD_REQUEST,
        Html("授权校验未通过，请返回 mcplink 检查授权。"),
    );
    if headers.get("host").and_then(|value| value.to_str().ok()) != Some(state.host.as_str())
        || uri.query().is_none_or(|query| query.len() > 8192)
        || state.cancelled.is_cancelled()
        || state.complete.is_cancelled()
    {
        return invalid;
    }
    let address = format!("http://{}{}", state.host, uri);
    let Ok(url) = Url::parse(&address) else {
        return invalid;
    };
    let mut fields = HashSet::new();
    let mut matched = false;
    let mut denied = false;
    for (key, value) in url.query_pairs() {
        if !matches!(key.as_ref(), "code" | "state" | "iss" | "error") {
            continue;
        }
        if value.is_empty() || !fields.insert(key.clone()) {
            return invalid;
        }
        if key == "state" {
            matched = aws_lc_rs::constant_time::verify_slices_are_equal(
                value.as_bytes(),
                state.csrf.as_bytes(),
            )
            .is_ok();
        }
        denied |= key == "error";
    }
    if !matched {
        return invalid;
    }
    let Ok(mut oauth) = state.oauth.try_lock() else {
        return (
            StatusCode::CONFLICT,
            Html("正在处理本次授权，请返回 mcplink 查看。"),
        );
    };
    if denied {
        state.complete.cancel();
        return (
            StatusCode::OK,
            Html("本次授权未完成，可以返回 mcplink 重试。"),
        );
    }
    let Ok(response) = AuthorizationCallback::from_redirect_url(&address) else {
        return invalid;
    };
    let exchanged = tokio::select! {
        biased;
        _ = state.cancelled.cancelled() => return invalid,
        result = tokio::time::timeout(Duration::from_secs(30), oauth.handle_callback_with_issuer(
            &response.code, &response.csrf_token, response.issuer.as_deref(),
        )) => result,
    };
    if matches!(
        exchanged,
        Ok(Err(AuthError::AuthorizationServerMismatch { .. }
            | AuthError::AuthorizationServerMissingIssuer { .. }))
    ) {
        return invalid;
    }
    let reply = if !matches!(exchanged, Ok(Ok(()))) {
        (
            StatusCode::BAD_GATEWAY,
            Html("授权交换未完成，请返回 mcplink 重新登录。"),
        )
    } else if state.commit().await.is_err() {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Html("授权状态未能保存，请返回 mcplink 检查连接。"),
        )
    } else {
        (
            StatusCode::OK,
            Html("授权已完成，可以返回 mcplink 检查连接。"),
        )
    };
    state.complete.cancel();
    reply
}

pub(crate) async fn start(engine: Arc<Engine>, id: &str) -> Result<Value> {
    let (lease, mut definition, revision) = {
        let _mutation = engine.mutation.lock().await;
        ensure!(!*engine.shutdown.borrow(), "共享后台正在退出");
        let connection = engine.store.connection(id)?;
        let definition = engine.definition(&connection)?;
        ensure!(
            definition.http_api.is_none(),
            "HTTP API Adapter 使用托管请求头认证"
        );
        config::validate(&definition)?;
        (
            engine.authorizations.reserve(id),
            definition,
            connection.revision,
        )
    };
    let cancelled = lease.attempt.cancelled.clone();
    let port = tokio::select! {
        biased;
        _ = cancelled.cancelled() => bail!("授权会话已取消"),
        port = lease.attempt.port.clone().lock_owned() => port,
    };
    let (endpoint, target) = engine.outbound.target(&definition)?;
    let prepare = async {
        let listener = TcpListener::bind((
            Ipv4Addr::LOCALHOST,
            definition.oauth_callback_port.unwrap_or(0),
        ))
        .await
        .context("无法建立本地授权回调")?;
        let host = listener.local_addr()?.to_string();
        let redirect = format!("http://{host}/callback");
        let mut manager = target
            .clone()
            .oauth(endpoint, cancelled.child_token())
            .await
            .context("无法配置上游 OAuth 客户端")?;
        let metadata = manager
            .resolve_metadata()
            .await
            .context("无法发现上游 OAuth 元数据")?
            .metadata;
        let authorization = Url::parse(&metadata.authorization_endpoint)
            .map_err(|_| anyhow::anyhow!("上游 OAuth 授权地址无效"))?;
        target.client(&authorization)?;
        if definition.client_id.is_none() && metadata.registration_endpoint.is_none() {
            bail!("上游不支持动态客户端注册；请先配置你注册应用的 clientId 及其要求的客户端机密");
        }
        if authorization.host_str() == Some("github.com")
            && authorization.path() == "/login/oauth/authorize"
            && definition.client_secret.is_none()
        {
            bail!("GitHub 授权码流程需要自有注册应用的客户端机密，请先配置 OAuth 客户端");
        }
        let grant = InMemoryCredentialStore::new();
        manager.set_credential_store(grant.clone());
        manager.set_metadata(metadata);
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
        let parsed = Url::parse(&url).map_err(|_| anyhow::anyhow!("浏览器授权地址无效"))?;
        target.client(&parsed)?;
        let mut states = parsed.query_pairs().filter(|(key, _)| key == "state");
        let csrf = states
            .next()
            .map(|(_, value)| value.into_owned())
            .context("OAuth 授权缺少 state")?;
        ensure!(
            !csrf.is_empty() && states.next().is_none(),
            "OAuth 授权 state 无效"
        );
        Ok::<_, anyhow::Error>((listener, host, state, grant, url, csrf))
    };
    let (listener, host, state, grant, url, csrf) = tokio::select! {
        biased;
        _ = cancelled.cancelled() => bail!("授权会话已取消"),
        result = tokio::time::timeout(Duration::from_secs(45), prepare) => {
            result.context("OAuth 授权准备超时，原有认证保持不变")??
        },
    };
    let _mutation = tokio::select! {
        biased;
        _ = cancelled.cancelled() => bail!("授权会话已取消"),
        guard = engine.mutation.lock() => guard,
    };
    let mut connection = engine.store.connection(id)?;
    ensure!(
        !*engine.shutdown.borrow() && !cancelled.is_cancelled() && connection.revision == revision,
        "连接已变化，请重新发起授权"
    );
    definition.oauth = true;
    definition
        .headers
        .retain(|key, _| !key.eq_ignore_ascii_case("authorization"));
    lifecycle::replace_definition(
        &engine,
        &mut connection,
        &definition,
        None,
        lifecycle::Transition::Authorize,
    )
    .await?;
    let state = Arc::new(Callback {
        engine: engine.clone(),
        id: connection.id,
        revision: connection.revision,
        host,
        csrf,
        oauth: AsyncMutex::new(state),
        grant,
        cancelled,
        complete: CancellationToken::new(),
    });
    tokio::spawn(async move {
        serve(listener, &state).await;
        // Release the fixed port before attempting to update abandoned status.
        drop(port);
        drop(lease);
        let _mutation = state.engine.mutation.lock().await;
        if let Ok(mut current) = state.engine.store.connection(&state.id)
            && current.revision == state.revision
            && current.status == "authorizing"
        {
            current.status = "needs_auth".into();
            current.last_error = Some("浏览器授权未完成，请重新登录".into());
            if state.engine.store.save_connection(&current).is_err() {
                eprintln!("mcplink: 无法更新已结束的授权状态");
            }
        }
    });
    Ok(json!({"url": url, "expiresAt": model::now() + CALLBACK_LIFETIME.as_secs()}))
}

async fn serve(listener: TcpListener, state: &Arc<Callback>) {
    let router = Router::new()
        .route("/callback", get(callback))
        .with_state(state.clone());
    let close = CancellationToken::new();
    let closing = close.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(closing.cancelled_owned())
            .await
    });
    tokio::select! {
        biased;
        _ = state.cancelled.cancelled() => {},
        _ = state.complete.cancelled() => {},
        _ = tokio::time::sleep(CALLBACK_LIFETIME) => {},
        _ = &mut server => return,
    }
    state.cancelled.cancel();
    close.cancel();
    if tokio::time::timeout(Duration::from_secs(2), &mut server)
        .await
        .is_err()
    {
        server.abort();
        let _ = server.await;
    }
}
