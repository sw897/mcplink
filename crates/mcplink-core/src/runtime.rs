use crate::{
    Engine,
    vault::{Vault, private_dir, private_write},
};
use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(not(windows))]
use std::process::Stdio;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::net::TcpListener;

#[derive(Clone, Serialize, Deserialize)]
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

#[derive(Clone)]
struct ServerState {
    engine: Arc<Engine>,
    key: String,
    info: RuntimeInfo,
}

fn valid_host(headers: &HeaderMap, port: u16) -> bool {
    let expected = format!("127.0.0.1:{port}");
    headers.get("host").and_then(|v| v.to_str().ok()) == Some(expected.as_str())
        && !headers.contains_key("origin")
}

async fn health(State(state): State<ServerState>, headers: HeaderMap) -> impl IntoResponse {
    if !valid_host(&headers, state.info.port) {
        return (StatusCode::FORBIDDEN, Json(json!({"error":"来源无效"})));
    }
    (
        StatusCode::OK,
        Json(
            json!({"application":"mcplink","instance":state.info.instance,"version":env!("CARGO_PKG_VERSION"),"managementApi":4}),
        ),
    )
}

async fn rpc(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> impl IntoResponse {
    let expected = format!("Bearer {}", state.key);
    if !valid_host(&headers, state.info.port)
        || headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(expected.as_str())
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":{"message":"本地调用身份或来源无效"}})),
        );
    }
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let response = match state.engine.rpc(method, params).await {
        Ok(value) => json!({"ok":value}),
        Err(error) => {
            if let Some(failure) = error.downcast_ref::<crate::invocation::ToolFailure>() {
                json!({"error":{"code":failure.code,"message":failure.to_string(),"details":failure.details}})
            } else {
                json!({"error":{"message":error.to_string()}})
            }
        }
    };
    (StatusCode::OK, Json(response))
}

pub async fn daemon(data: PathBuf, port: Option<u16>) -> Result<()> {
    private_dir(&data)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data.join("runtime.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock).context("mcplink 后台已在运行")?;
    let old = fs::read(data.join("runtime.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RuntimeInfo>(&bytes).ok());
    let port = port
        .or_else(|| old.as_ref().map(|info| info.port))
        .unwrap_or(0);
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .context("本地后台端口被占用，请关闭占用程序或用 daemon --port 指定端口")?;
    let info = RuntimeInfo {
        port: listener.local_addr()?.port(),
        instance: uuid::Uuid::new_v4().to_string(),
        pid: std::process::id(),
    };
    let engine = Engine::open(data.clone())?;
    let key = match engine.vault.get("runtime-key")? {
        Some(value) => value,
        None => {
            let key = format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            );
            engine.vault.set("runtime-key", &key)?;
            key
        }
    };
    private_write(&data.join("runtime.json"), &serde_json::to_vec(&info)?)?;
    let state = ServerState { engine, key, info };
    let router = Router::new()
        .route("/health", get(health))
        .route("/rpc", post(rpc))
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024 + 64 * 1024))
        .with_state(state);
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            if tokio::signal::ctrl_c().await.is_err() {
                // A Windows background process can have no console to receive Ctrl-C.
                std::future::pending::<()>().await;
            }
        })
        .await
        .context("本地后台已中断")?;
    drop(lock);
    Ok(())
}

#[derive(Clone)]
pub struct Client {
    pub data: PathBuf,
    info: RuntimeInfo,
    key: String,
    http: reqwest::Client,
    management_api: u64,
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
    async fn read(data: &Path) -> Result<Self> {
        let info: RuntimeInfo =
            serde_json::from_slice(&fs::read(data.join("runtime.json")).context("后台尚未启动")?)
                .context("后台状态文件无法读取")?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(650))
            .build()?;
        let response = http
            .get(format!("http://127.0.0.1:{}/health", info.port))
            .timeout(Duration::from_secs(1))
            .send()
            .await
            .context("后台未连接")?;
        let status: Value = response.json().await.context("本地端口未返回有效状态")?;
        if status["application"] != "mcplink" || status["instance"] != info.instance {
            bail!("本地端口身份校验未通过");
        }
        let key = Vault::new(data)?
            .get("runtime-key")?
            .context("无法取得本地通信凭据")?;
        Ok(Self {
            data: data.to_owned(),
            info,
            key,
            http,
            management_api: status["managementApi"].as_u64().unwrap_or(1),
        })
    }

    pub async fn connect(data: PathBuf) -> Result<Self> {
        if let Ok(client) = Self::read(&data).await {
            return Ok(client);
        }
        let executable = std::env::current_exe().context("无法定位后台程序")?;
        #[cfg(windows)]
        spawn_background(&executable, &data)?;
        #[cfg(not(windows))]
        {
            let child = tokio::process::Command::new(executable)
                .args(["daemon", "--data-dir"])
                .arg(&data)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("无法启动后台程序")?;
            drop(child);
        }
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if let Ok(client) = Self::read(&data).await {
                return Ok(client);
            }
        }
        bail!("后台未能启动，请检查系统凭据库与本地端口，或运行 mcplink daemon 查看原因")
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        if self.management_api < 4
            && (matches!(method, "oauth.configure" | "oauth.start")
                || (method == "connections.save"
                    && ["clientSecret", "oauthScopes", "oauthCallbackPort"]
                        .iter()
                        .any(|key| params["definition"].get(*key).is_some())))
        {
            bail!("旧后台不支持当前 OAuth 客户端配置；请退出桌面并停止旧后台，再使用同一构建重试");
        }
        if method == "workspace.identity" && self.management_api < 3 {
            bail!("旧后台不支持当前 CLI 生成命令；请退出桌面并停止旧后台，再使用同一构建重试");
        }
        if matches!(method, "tools.describe" | "tools.call") && self.management_api < 2 {
            bail!("旧后台不支持当前工具调用命令；请退出桌面并停止旧后台，再使用同一构建重试");
        }
        let response = self
            .http
            .post(format!("http://127.0.0.1:{}/rpc", self.info.port))
            .bearer_auth(&self.key)
            .json(&json!({"method":method,"params":params}))
            .send()
            .await
            .context("后台通信中断，执行结果需核对；不要自动重试")?;
        let body: Value = response
            .json()
            .await
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
