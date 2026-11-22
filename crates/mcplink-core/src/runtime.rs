use crate::{
    Engine,
    runtime_auth::{self, Root, WireSession},
    vault::{Vault, private_dir, private_write},
};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router, body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hyper::server::conn::http1;
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(not(windows))]
use std::process::Stdio;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinSet};
use uuid::Uuid;

pub const MANAGEMENT_API: u64 = 24;
const MAX_CONNECTIONS: usize = 256;
const MAX_REQUESTS: usize = 64;
const MAX_SESSIONS: usize = 4096;
const BODY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub port: u16,
    pub instance: String,
    pub pid: u32,
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("MCPLINK_DATA_DIR") {
        return Ok(path.into());
    }
    directories::ProjectDirs::from("io", "mcplink", "mcplink")
        .map(|dirs| dirs.data_local_dir().to_owned())
        .context("无法定位应用数据目录")
}

struct ServerState {
    engine: Arc<Engine>,
    root: Root,
    info: RuntimeInfo,
    instance: Uuid,
    sessions: Mutex<HashMap<Uuid, Arc<WireSession>>>,
    requests: Semaphore,
}

fn valid_host(headers: &HeaderMap, port: u16) -> bool {
    let expected = format!("127.0.0.1:{port}");
    headers.get("host").and_then(|v| v.to_str().ok()) == Some(expected.as_str())
        && !headers.contains_key("origin")
        && headers
            .get("sec-fetch-site")
            .is_none_or(|value| value != "cross-site")
}

fn lock_contended(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

async fn health(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> impl IntoResponse {
    if !valid_host(&headers, state.info.port) {
        return (StatusCode::FORBIDDEN, Json(json!({"error":"来源无效"})));
    }
    (
        StatusCode::OK,
        Json(
            json!({"application":"mcplink","instance":state.info.instance,"version":env!("CARGO_PKG_VERSION"),"managementApi":MANAGEMENT_API,"runtimeAuth":runtime_auth::VERSION}),
        ),
    )
}

fn auth_header(headers: &HeaderMap, port: u16) -> Option<&str> {
    if !valid_host(headers, port) {
        return None;
    }
    let mut all = headers.get_all(runtime_auth::HEADER).iter();
    let value = all.next()?.to_str().ok()?;
    (all.next().is_none() && value.len() <= 128).then_some(value)
}

#[derive(Deserialize, Serialize)]
struct ServerProof {
    id: Uuid,
    proof: String,
}

async fn authenticate(State(state): State<Arc<ServerState>>, headers: HeaderMap) -> Response {
    let Some(client) = auth_header(&headers, state.info.port)
        .and_then(|header| state.root.verify_hello(state.instance, header))
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Ok(_permit) = state.requests.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let id = Uuid::new_v4();
    let Ok(session) = state.root.session(state.instance, client, id) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut sessions = state.sessions.lock().unwrap();
    let mut used_nonce = false;
    sessions.retain(|_, session| {
        let active = session.created.elapsed() < runtime_auth::SESSION_LIFETIME;
        used_nonce |= active && session.hello_nonce == client;
        active
    });
    if used_nonce {
        return StatusCode::FORBIDDEN.into_response();
    }
    if sessions.len() >= MAX_SESSIONS {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    sessions.insert(id, Arc::new(session));
    Json(ServerProof {
        id,
        proof: state.root.server_proof(state.instance, client, id),
    })
    .into_response()
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RpcRequest {
    method: String,
    params: Value,
    // Only the native desktop caller places its confirmation in the
    // authenticated envelope. User-supplied management parameters are nested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirmed_transfer_origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bridge_executable: Option<PathBuf>,
}

fn request_session(state: &ServerState, headers: &HeaderMap) -> Option<(Arc<WireSession>, u64)> {
    let mut header = auth_header(headers, state.info.port)?.splitn(3, '.');
    let id = Uuid::parse_str(header.next()?).ok()?;
    let number = header.next()?.parse().ok()?;
    let mac = header.next()?;
    let session = state.sessions.lock().unwrap().get(&id)?.clone();
    (session.created.elapsed() < runtime_auth::SESSION_LIFETIME
        && session.verify_header(number, mac))
    .then_some((session, number))
}

async fn rpc(State(state): State<Arc<ServerState>>, request: Request) -> Response {
    let Some((session, number)) = request_session(&state, request.headers()) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Ok(_permit) = state.requests.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let body = tokio::time::timeout(
        BODY_TIMEOUT,
        body::to_bytes(request.into_body(), runtime_auth::MAX_BYTES),
    )
    .await;
    let mut bytes = match body {
        Ok(Ok(bytes)) => bytes.to_vec(),
        Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    let Ok(plaintext) = session.open(number, false, &mut bytes) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    if !session.accept(number) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(request) = serde_json::from_slice::<RpcRequest>(plaintext) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let result = tokio::time::timeout(
        Duration::from_secs(660),
        state.engine.rpc_confirmed(
            &request.method,
            request.params,
            request.confirmed_transfer_origin.as_deref(),
            request.bridge_executable.as_deref(),
        ),
    )
    .await;
    let result = result.unwrap_or_else(|_| {
        Err(anyhow::anyhow!(
            "本地操作超时，执行结果需核对；不要自动重试"
        ))
    });
    let response = match result {
        Ok(value) => json!({"ok":value}),
        Err(error) => {
            if let Some(failure) = error.downcast_ref::<crate::invocation::ToolFailure>() {
                json!({"error":{"code":failure.code,"message":failure.to_string(),"details":failure.details}})
            } else {
                json!({"error":{"message":error.to_string()}})
            }
        }
    };
    let Ok(mut bytes) = serde_json::to_vec(&response) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if bytes.len() > runtime_auth::MAX_REPLY_BYTES - 16
        || session.seal(number, true, &mut bytes).is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    bytes.into_response()
}

pub async fn daemon(data: PathBuf, port: Option<u16>) -> Result<()> {
    private_dir(&data)?;
    let lock_path = data.join("runtime.lock");
    crate::vault::prepare_private_file(&lock_path)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)?;
    fs2::FileExt::try_lock_exclusive(&lock).context("mcplink 后台已在运行")?;
    let old = fs::read(data.join("runtime.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RuntimeInfo>(&bytes).ok());
    let preferred = port
        .or_else(|| old.as_ref().map(|info| info.port))
        .unwrap_or(0);
    let listener = match TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, preferred)).await {
        Err(error)
            if port.is_none()
                && preferred != 0
                && error.kind() == std::io::ErrorKind::AddrInUse =>
        {
            // The data directory lock is already held. No other daemon can
            // publish a competing endpoint while this instance falls back.
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await
        }
        result => result,
    }
    .context("本地后台端口被占用，请关闭占用程序或用 daemon --port 指定端口")?;
    let instance = Uuid::new_v4();
    let info = RuntimeInfo {
        port: listener.local_addr()?.port(),
        instance: instance.to_string(),
        pid: std::process::id(),
    };
    let engine = Engine::open(data.clone(), info.port)?;
    let root = Root::load(&engine.vault, true)?;
    private_write(&data.join("runtime.json"), &serde_json::to_vec(&info)?)?;
    let own_instance = info.instance.clone();
    let state = Arc::new(ServerState {
        engine: engine.clone(),
        root,
        info,
        instance,
        sessions: Mutex::new(HashMap::new()),
        requests: Semaphore::new(MAX_REQUESTS),
    });
    let router = Router::new()
        .route("/health", get(health))
        .route("/auth", post(authenticate))
        .route("/rpc", post(rpc))
        .with_state(state);
    let maintenance = tokio::spawn(maintain_change_storage(engine.clone()));
    let outcome = serve_management(listener, router, &engine).await;
    maintenance.abort();
    if fs::read(data.join("runtime.json"))
        .ok()
        .and_then(|value| serde_json::from_slice::<RuntimeInfo>(&value).ok())
        .is_some_and(|current| current.instance == own_instance)
    {
        fs::remove_file(data.join("runtime.json"))?;
    }
    drop(lock);
    outcome
}

async fn maintain_change_storage(engine: Arc<Engine>) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let engine = engine.clone();
        if tokio::task::spawn_blocking(move || {
            let _guard = engine.mutation.blocking_lock();
            if !*engine.shutdown.borrow() {
                engine.maintain_change_storage();
            }
        })
        .await
        .is_err()
        {
            break;
        }
    }
}

async fn serve_management(listener: TcpListener, router: Router, engine: &Engine) -> Result<()> {
    let mut connections = JoinSet::new();
    let mut shutdown = engine.shutdown.subscribe();
    let signal = termination_signal();
    tokio::pin!(signal);
    let result = loop {
        tokio::select! {
            biased;
            _ = shutdown.wait_for(|stopping| *stopping) => break Ok(()),
            _ = &mut signal => break Ok(()),
            _ = connections.join_next(), if !connections.is_empty() => {}
            incoming = listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                let (stream, _) = match incoming.context("本地后台连接已中断") {
                    Ok(connection) => connection,
                    Err(error) => break Err(error),
                };
                let router = router.clone();
                let mut stopping = engine.shutdown.subscribe();
                connections.spawn(async move {
                    let connection = http1::Builder::new()
                        .timer(TokioTimer::new())
                        .header_read_timeout(BODY_TIMEOUT)
                        .max_buf_size(32 * 1024)
                        .serve_connection(TokioIo::new(stream), TowerToHyperService::new(router));
                    tokio::pin!(connection);
                    tokio::select! {
                        _ = async { let _ = stopping.wait_for(|stop| *stop).await; } => {
                            connection.as_mut().graceful_shutdown();
                            let _ = connection.await;
                        }
                        _ = &mut connection => {}
                    }
                });
            }
        }
    };
    drop(listener);
    engine.shutdown.send_replace(true);
    engine.paused.store(true, Ordering::SeqCst);
    // The daemon owns cleanup. All admitted stop requests observe this same
    // result before the HTTP connection drains and the process releases its lock.
    let cleaned = tokio::time::timeout(Duration::from_secs(22), async {
        let (oauth, pool) = tokio::join!(engine.authorizations.shutdown(), engine.pool.shutdown());
        oauth.and(pool)
    })
    .await
    .context("共享后台退出清理超时")
    .and_then(|result| result);
    engine.shutdown_result.send_replace(Some(
        cleaned.as_ref().map(|_| ()).map_err(ToString::to_string),
    ));
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    result?;
    cleaned?;
    drained.context("本地后台请求退出清理超时")?;
    Ok(())
}

async fn termination_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! { _ = signal.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
            return;
        }
    }
    if tokio::signal::ctrl_c().await.is_err() {
        // A Windows background process can have no console to receive Ctrl-C.
        std::future::pending::<()>().await;
    }
}

#[derive(Clone)]
pub struct Client {
    pub data: PathBuf,
    info: RuntimeInfo,
    session: Arc<WireSession>,
    http: reqwest::Client,
}

struct Running {
    info: RuntimeInfo,
    http: reqwest::Client,
    management_api: u64,
    auth_version: Option<u64>,
}

#[derive(Default)]
struct SessionState {
    client: Option<Client>,
    denied: Option<(RuntimeInfo, String)>,
    startup_error: Option<String>,
}

/// One process-local authentication session; secrets never leave memory.
#[derive(Clone)]
pub struct Session {
    data: PathBuf,
    executable: PathBuf,
    state: Arc<tokio::sync::Mutex<SessionState>>,
}

impl Session {
    pub fn new(data: PathBuf, executable: PathBuf) -> Self {
        Self {
            data,
            executable,
            state: Arc::new(tokio::sync::Mutex::new(SessionState::default())),
        }
    }

    /// Only an explicit foreground retry should clear an access denial.
    pub async fn retry_authentication(&self) {
        let mut state = self.state.lock().await;
        state.denied = None;
        state.startup_error = None;
    }

    async fn client(&self, start: bool) -> Result<Client> {
        let mut state = self.state.lock().await;
        let running = match Client::running(&self.data, state.client.as_ref()).await {
            Ok(running) => {
                // A slow but successful startup is still discovered passively.
                state.startup_error = None;
                running
            }
            Err(error) if !start => return Err(error),
            Err(_) => {
                if let Some(message) = &state.startup_error {
                    bail!("{message}；自动启动已暂停，请在前台明确重试或重新启动客户端");
                }
                match Client::running_or_start(&self.data, &self.executable, state.client.as_ref())
                    .await
                {
                    Ok(running) => running,
                    Err(error) => {
                        state.startup_error = Some(error.to_string());
                        return Err(error);
                    }
                }
            }
        };
        if let Some((info, message)) = &state.denied
            && *info == running.info
        {
            bail!("{message}；请在前台明确重试授权，或重新启动客户端");
        }
        let info = running.info.clone();
        match Client::authenticate(&self.data, running, state.client.as_ref()).await {
            Ok(client) => {
                state.client = Some(client.clone());
                state.denied = None;
                Ok(client)
            }
            Err(error) => {
                state.denied = Some((info, error.to_string()));
                Err(error)
            }
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        if method == "gateway.stop" {
            let cached = self.state.lock().await.client.clone();
            let result = Client::stop_cached(&self.data, cached.as_ref()).await;
            self.state.lock().await.client = None;
            return result;
        }
        // Never replay an accepted request when its response is lost.
        self.client(true)
            .await?
            .dispatch(method, params, None, Some(&self.executable))
            .await
    }

    pub async fn call_running(&self, method: &str, params: Value) -> Result<Value> {
        self.client(false)
            .await?
            .dispatch(method, params, None, Some(&self.executable))
            .await
    }

    /// Called by the desktop after a native confirmation of this exact save.
    pub async fn save_confirmed_transfer(&self, params: Value, origin: String) -> Result<Value> {
        self.client(true)
            .await?
            .dispatch("connections.save", params, Some(origin), None)
            .await
    }
}

#[cfg(windows)]
fn spawn_background(executable: &Path, data: &Path) -> Result<()> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{CREATE_NO_WINDOW, CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW},
    };

    // Quote Windows argv without converting Unicode paths through UTF-8.
    fn quoted(value: &OsStr) -> Result<Vec<u16>> {
        let mut output = vec![b'"' as u16];
        let mut slashes = 0;
        for character in value.encode_wide() {
            if character == 0 {
                bail!("后台路径包含无效字符");
            }
            if character == b'\\' as u16 {
                slashes += 1;
                continue;
            }
            if character == b'"' as u16 {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
            } else {
                output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
            }
            output.push(character);
            slashes = 0;
        }
        output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
        output.push(b'"' as u16);
        Ok(output)
    }

    let application: Vec<_> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut command = quoted(executable.as_os_str())?;
    if crate::vault::interaction_disabled() {
        command.extend(" --no-auth-ui".encode_utf16());
    }
    command.extend(" daemon --data-dir ".encode_utf16());
    command.extend(quoted(data.as_os_str())?);
    command.push(0);
    if command.len() > 32_767 {
        bail!("后台命令路径过长");
    }
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: All pointers live through CreateProcessW; command is writable,
    // NUL-terminated UTF-16. The daemon needs no inherited handles, including
    // extra capture-pipe copies left by PowerShell/.NET or other parent hosts.
    let started = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_NO_WINDOW,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut process,
        )
    };
    if started == 0 {
        return Err(std::io::Error::last_os_error()).context("无法启动独立后台程序");
    }
    // SAFETY: Success returned two owned handles; closing them does not stop
    // the process. No task waits on these handles after startup.
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

impl Client {
    pub async fn stop_existing(data: &Path) -> Result<Value> {
        Self::stop_cached(data, None).await
    }

    async fn stop_cached(data: &Path, cached: Option<&Client>) -> Result<Value> {
        let path = data.join("runtime.lock");
        if !path.exists() {
            return Ok(json!({"stopped":true,"wasRunning":false}));
        }
        let lock = fs::OpenOptions::new().read(true).write(true).open(path)?;
        match fs2::FileExt::try_lock_exclusive(&lock) {
            Ok(()) => return Ok(json!({"stopped":true,"wasRunning":false})),
            Err(error) if lock_contended(&error) => {}
            Err(error) => return Err(error.into()),
        }
        // Never call connect here: quitting an absent daemon must not start one.
        let client = Self::authenticate(data, Self::running(data, cached).await?, cached).await?;
        let request = client.call("gateway.stop", json!({})).await;
        let cleanup_forced = request.is_err();
        let stopped = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match fs2::FileExt::try_lock_exclusive(&lock) {
                    Ok(()) => return Ok(json!({"stopped":true,"wasRunning":true,"cleanupForced":cleanup_forced})),
                    Err(error) if lock_contended(&error) => {},
                    Err(error) => return Err(anyhow::Error::from(error)),
                }
                if fs::read(data.join("runtime.json")).ok()
                    .and_then(|bytes| serde_json::from_slice::<RuntimeInfo>(&bytes).ok())
                    .is_some_and(|info| info.instance != client.info.instance) {
                    return Ok(json!({"stopped":true,"wasRunning":true,"restartedByRequest":true}));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }).await.context("后台退出清理超时，请检查现有调用后重试")??;
        if let Some(error) = request
            .as_ref()
            .ok()
            .and_then(|value| value["cleanupError"].as_str())
        {
            bail!("后台已停止，退出清理未完成：{error}");
        }
        Ok(stopped)
    }

    async fn running(data: &Path, cached: Option<&Client>) -> Result<Running> {
        let info: RuntimeInfo =
            serde_json::from_slice(&fs::read(data.join("runtime.json")).context("后台尚未启动")?)
                .context("后台状态文件无法读取")?;
        let http = if let Some(client) = cached {
            client.http.clone()
        } else {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .pool_idle_timeout(Duration::from_secs(5))
                .pool_max_idle_per_host(2)
                .timeout(Duration::from_secs(650))
                .build()?
        };
        let response = http
            .get(format!("http://127.0.0.1:{}/health", info.port))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .context("后台未连接")?;
        let status: Value = serde_json::from_slice(&read_bounded(response, 4096).await?)
            .context("本地端口未返回有效状态")?;
        if status["application"] != "mcplink" || status["instance"] != info.instance {
            bail!("本地端口身份校验未通过");
        }
        Ok(Running {
            info,
            http,
            management_api: status["managementApi"].as_u64().unwrap_or(1),
            auth_version: status["runtimeAuth"].as_u64(),
        })
    }

    async fn authenticate(data: &Path, running: Running, cached: Option<&Client>) -> Result<Self> {
        if running.management_api < MANAGEMENT_API {
            bail!(
                "旧后台使用不兼容的管理协议，请先用原版本退出后台，再使用同一构建的桌面、CLI 和 Bridge"
            );
        }
        ensure!(
            running.management_api == MANAGEMENT_API
                && running.auth_version == Some(runtime_auth::VERSION),
            "后台管理协议不兼容，请使用同一构建的桌面、CLI 和 Bridge"
        );
        if let Some(client) = cached
            && client.data == data
            && client.info == running.info
            && client.session.created.elapsed()
                < runtime_auth::SESSION_LIFETIME - Duration::from_secs(10)
        {
            return Ok(client.clone());
        }
        let root = Root::load(&Vault::new(data)?, false)?;
        let instance = Uuid::parse_str(&running.info.instance).context("后台实例标识无效")?;
        let nonce = Uuid::new_v4();
        let response = running
            .http
            .post(format!("http://127.0.0.1:{}/auth", running.info.port))
            .header(runtime_auth::HEADER, root.hello(instance, nonce))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .context("后台身份验证失败")?;
        let proof: ServerProof = serde_json::from_slice(
            &read_bounded(response, 1024)
                .await
                .context("后台身份验证失败")?,
        )
        .context("后台身份验证失败")?;
        root.verify_server(instance, nonce, proof.id, &proof.proof)?;
        Ok(Self {
            data: data.to_owned(),
            info: running.info,
            session: Arc::new(root.session(instance, nonce, proof.id)?),
            http: running.http,
        })
    }

    /// Reuse the authenticated HTTP client while its daemon instance is current.
    /// Catalog observers must never start a daemon or read the Vault each tick.
    pub async fn refresh_running(&self) -> Result<Self> {
        let info: RuntimeInfo = serde_json::from_slice(
            &fs::read(self.data.join("runtime.json")).context("后台尚未启动")?,
        )?;
        if info == self.info
            && self.session.created.elapsed()
                < runtime_auth::SESSION_LIFETIME - Duration::from_secs(10)
        {
            return Ok(self.clone());
        }
        Self::authenticate(
            &self.data,
            Self::running(&self.data, Some(self)).await?,
            Some(self),
        )
        .await
    }

    pub async fn connect(data: PathBuf) -> Result<Self> {
        let executable = std::env::current_exe().context("无法定位后台程序")?;
        let running = Self::running_or_start(&data, &executable, None).await?;
        Self::authenticate(&data, running, None).await
    }

    async fn running_or_start(
        data: &Path,
        executable: &Path,
        cached: Option<&Client>,
    ) -> Result<Running> {
        if let Ok(running) = Self::running(data, cached).await {
            return Ok(running);
        }
        #[cfg(windows)]
        spawn_background(executable, data)?;
        #[cfg(not(windows))]
        {
            let mut command = tokio::process::Command::new(executable);
            if crate::vault::interaction_disabled() {
                command.arg("--no-auth-ui");
            }
            command
                .args(["daemon", "--data-dir"])
                .arg(data)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(unix)]
            command.process_group(0);
            let child = command.spawn().context("无法启动后台程序")?;
            drop(child);
        }
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if let Ok(running) = Self::running(data, cached).await {
                return Ok(running);
            }
        }
        bail!("后台未能启动，请检查系统凭据库与本地端口，或运行 mcplink daemon 查看原因")
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.dispatch(method, params, None, None).await
    }

    async fn dispatch(
        &self,
        method: &str,
        params: Value,
        confirmed_transfer_origin: Option<String>,
        executable: Option<&Path>,
    ) -> Result<Value> {
        let number = self.session.next()?;
        // Use the invoking CLI or the native desktop's sidecar. RPC parameters
        // supplied by the CLI or WebView cannot set this field.
        let bridge_executable = if matches!(method, "changes.preview" | "connections.export_config")
        {
            Some(match executable {
                Some(path) => path.to_owned(),
                None => std::env::current_exe().context("无法定位共享入口程序")?,
            })
        } else {
            None
        };
        let mut bytes = serde_json::to_vec(&RpcRequest {
            method: method.to_owned(),
            params,
            confirmed_transfer_origin,
            bridge_executable,
        })?;
        ensure!(
            bytes.len() <= runtime_auth::MAX_BYTES - 16,
            "本地管理请求过大"
        );
        self.session.seal(number, false, &mut bytes)?;
        let response = self
            .http
            .post(format!("http://127.0.0.1:{}/rpc", self.info.port))
            .header(runtime_auth::HEADER, self.session.format_header(number))
            .body(bytes)
            .send()
            .await
            .context("后台通信中断，执行结果需核对；不要自动重试")?;
        let mut bytes = read_bounded(response, runtime_auth::MAX_REPLY_BYTES)
            .await
            .context("后台响应无效，执行结果需核对；不要自动重试")?;
        let plaintext = self
            .session
            .open(number, true, &mut bytes)
            .context("后台响应验证失败，执行结果需核对；不要自动重试")?;
        let body: Value = serde_json::from_slice(plaintext)
            .context("后台响应无效，执行结果需核对；不要自动重试")?;
        if let Some(error) = body.get("error") {
            if let Ok(failure) =
                serde_json::from_value::<crate::invocation::ToolFailure>(error.clone())
            {
                return Err(failure.into());
            }
            bail!(
                "{}",
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("本地操作失败")
            );
        }
        body.get("ok").cloned().context("后台响应缺少结果")
    }
}

async fn read_bounded(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut response = response.error_for_status()?;
    ensure!(
        response
            .content_length()
            .is_none_or(|len| len <= limit as u64),
        "本地后台响应过大"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(chunk.len() <= limit - bytes.len(), "本地后台响应过大");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
