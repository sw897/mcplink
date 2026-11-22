use mcplink_desktop::{
    UPDATE_PUBLIC_KEY,
    cli_sync::{self, CliTarget},
    update_catalog::{Catalog, DOWNLOAD_TIMEOUT, Package, RELEASES_URL, Release, StagedPackage},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::watch;

#[derive(Clone)]
struct Settings {
    catalog: Catalog,
    home: PathBuf,
    receipt: PathBuf,
    sidecar: PathBuf,
    #[cfg(target_os = "macos")]
    data: PathBuf,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    revision: u64,
    current_version: String,
    target: String,
    supported: bool,
    phase: String,
    preview: bool,
    candidate_id: u64,
    release: Option<Release>,
    downloaded: u64,
    total: u64,
    cli_targets: Vec<String>,
    can_download: bool,
    can_install: bool,
    can_archive_stale_receipt: bool,
    error: Option<String>,
    startup_issue: Option<String>,
    message: Option<String>,
}

#[derive(Clone)]
struct Candidate {
    update: Update,
    package: Package,
}

struct Inner {
    view: View,
    candidate: Option<Candidate>,
    bytes: Option<Arc<StagedPackage>>,
    targets: Vec<CliTarget>,
    cancel: Option<watch::Sender<bool>>,
    reset_backend: bool,
    cli_synced: bool,
}

struct State {
    settings: Settings,
    inner: Mutex<Inner>,
}

impl Inner {
    fn busy(&self) -> bool {
        matches!(
            self.view.phase.as_str(),
            "checking" | "downloading" | "installing"
        )
    }
    fn snapshot(&self) -> View {
        let mut view = self.view.clone();
        view.can_download = !self.busy() && self.candidate.is_some() && self.bytes.is_none();
        view.can_install = !self.busy() && self.candidate.is_some() && self.bytes.is_some();
        view
    }
}

fn snapshot(app: &AppHandle) -> View {
    app.state::<State>().inner.lock().unwrap().snapshot()
}
fn publish(app: &AppHandle) -> View {
    let view = {
        let state = app.state::<State>();
        let mut inner = state.inner.lock().unwrap();
        inner.view.revision += 1;
        inner.snapshot()
    };
    let _ = app.emit("software-update", &view);
    view
}
fn fail(app: &AppHandle, error: String) -> View {
    let state = app.state::<State>();
    {
        let mut inner = state.inner.lock().unwrap();
        inner.view.phase = "error".into();
        inner.view.error = Some(error);
        inner.cancel = None;
    }
    publish(app)
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let version = app.package_info().version.clone();
    let target = tauri_plugin_updater::target().unwrap_or_else(|| "unsupported".into());
    let settings = Settings {
        catalog: Catalog::github(
            version.clone(),
            target.clone(),
            UPDATE_PUBLIC_KEY.trim().into(),
        ),
        home: directories::BaseDirs::new()
            .ok_or("无法定位用户目录")?
            .home_dir()
            .to_owned(),
        receipt: app.path().app_data_dir()?.join("updates/pending.json"),
        sidecar: crate::cli_path(app.handle())?,
        #[cfg(target_os = "macos")]
        data: mcplink_core::runtime::data_dir()?,
    };
    #[cfg(debug_assertions)]
    let settings = fixture_settings(app, settings)?;
    let receipt = cli_sync::read_receipt(&settings.receipt);
    let reset_backend = receipt
        .as_ref()
        .ok()
        .and_then(|r| r.as_ref())
        .is_some_and(|r| r.version == version);
    let can_archive_stale_receipt =
        matches!(receipt, Ok(Some(ref receipt)) if receipt.version != version);
    let synced = cli_sync::apply_receipt(
        &settings.receipt,
        &settings.home,
        &settings.sidecar,
        &version,
    );
    let startup_issue = synced.as_ref().err().cloned();
    app.manage(State {
        settings,
        inner: Mutex::new(Inner {
            view: View {
                revision: 0,
                current_version: version.to_string(),
                target: target.clone(),
                supported: matches!(target.as_str(), "darwin-aarch64" | "windows-x86_64"),
                phase: "idle".into(),
                preview: true,
                candidate_id: 0,
                release: None,
                downloaded: 0,
                total: 0,
                cli_targets: vec![],
                can_download: false,
                can_install: false,
                can_archive_stale_receipt,
                error: None,
                startup_issue,
                message: None,
            },
            candidate: None,
            bytes: None,
            targets: vec![],
            cancel: None,
            reset_backend,
            cli_synced: synced.unwrap_or(false),
        }),
    });
    Ok(())
}

#[cfg(debug_assertions)]
fn fixture_settings(
    app: &tauri::App,
    mut settings: Settings,
) -> Result<Settings, Box<dyn std::error::Error>> {
    let version = app.package_info().version.clone();
    let target = settings.catalog.target.clone();
    if let Some(path) = std::env::var_os("MCPLINK_UPDATE_FIXTURE") {
        if !app
            .config()
            .identifier
            .starts_with("io.mcplink.acceptance.")
        {
            return Err("更新夹具只能用于独立验收应用".into());
        }
        let home = PathBuf::from(std::env::var_os("MCPLINK_HOME").ok_or("更新夹具需要独立 home")?);
        let path = PathBuf::from(path).canonicalize()?;
        if !path.starts_with(home.canonicalize()?) {
            return Err("更新夹具必须位于验收 home".into());
        }
        let fixture: Value = serde_json::from_slice(&fs::read(path)?)?;
        let key =
            PathBuf::from(fixture["publicKey"].as_str().ok_or("缺少夹具公钥")?).canonicalize()?;
        if !key.starts_with(home.canonicalize()?) {
            return Err("夹具公钥必须位于验收 home".into());
        }
        settings.catalog = Catalog::fixture(
            version.clone(),
            target.clone(),
            fs::read_to_string(key)?.trim().into(),
            &home,
            fixture["api"].as_str().ok_or("缺少夹具 API")?.parse()?,
            fixture["downloads"]
                .as_str()
                .ok_or("缺少夹具下载地址")?
                .parse()?,
        )?;
        settings.home = home.clone();
        settings.receipt = home.join("desktop-updates/pending.json");
    }
    Ok(settings)
}

pub fn needs_backend_reset(app: &AppHandle) -> bool {
    app.try_state::<State>()
        .is_some_and(|s| s.inner.lock().unwrap().reset_backend)
}

pub fn finish_startup(app: &AppHandle, result: Result<(), String>) {
    let state = app.state::<State>();
    {
        let mut inner = state.inner.lock().unwrap();
        match result {
            Ok(()) => {
                inner.reset_backend = false;
                if inner.cli_synced {
                    if fs::remove_file(&state.settings.receipt).is_err() {
                        inner.view.startup_issue =
                            Some("升级已完成，但恢复记录未能清理；可在软件更新中重试".into());
                    } else {
                        inner.view.startup_issue = None;
                        inner.view.message =
                            Some("升级已完成。请让 Agent 重新加载 MCP，以使用新版本入口。".into());
                    }
                }
            }
            Err(error) => {
                inner.view.startup_issue = Some(format!("升级后的旧后台未能退出：{error}"))
            }
        }
    }
    publish(app);
}

async fn check(app: &AppHandle, preview: bool) -> View {
    let settings = app.state::<State>().settings.clone();
    let result = tokio::time::timeout(Duration::from_secs(45), async {
        let Some(release) = settings.catalog.latest(preview).await? else {
            return Ok::<_, String>(("no-release", None, None));
        };
        if !release
            .version
            .cmp_precedence(&settings.catalog.current)
            .is_gt()
        {
            return Ok(("current", Some(release), None));
        }
        let Some(package) = settings.catalog.package(&release).await? else {
            return Ok(("unavailable", Some(release), None));
        };
        // Tauri requires an endpoint even when restoring an already fetched manifest.
        // All network I/O remains in Catalog; restore_update never requests this URL.
        let manifest_name = if settings.catalog.target == "darwin-aarch64" {
            "macos-updater.json"
        } else {
            "windows-updater.json"
        };
        let update = app
            .updater_builder()
            .endpoints(vec![
                format!("{RELEASES_URL}/download/{}/{manifest_name}", release.tag)
                    .parse()
                    .map_err(|_| "更新清单地址无效")?,
            ])
            .map_err(|_| "无法准备更新清单地址")?
            .target(settings.catalog.target.clone())
            .pubkey(settings.catalog.public_key.clone())
            .version_comparator(|current, release| release.version.cmp_precedence(&current).is_gt())
            // Our background and receipts are flushed first; retain the UI if launching the
            // Windows installer fails instead of tearing down Tauri before ShellExecute.
            .on_before_exit(|| {})
            .build()
            .map_err(|_| "无法准备系统升级器")?
            .restore_update(package.manifest.clone())
            .map_err(|_| "签名更新清单无效")?
            .ok_or("所选发布不再是更新版本")?;
        if update.version != release.version.to_string()
            || update.download_url != package.url
            || update.signature != package.signature
        {
            return Err("系统升级器的版本或下载信息不一致".into());
        }
        Ok((
            "available",
            Some(release),
            Some(Candidate { update, package }),
        ))
    })
    .await;
    match result {
        Ok(Ok((phase, release, candidate))) => {
            let state = app.state::<State>();
            {
                let mut inner = state.inner.lock().unwrap();
                inner.view.phase = phase.into();
                inner.view.total = candidate.as_ref().map_or(0, |c| c.package.bytes);
                inner.view.release = release;
                inner.candidate = candidate;
            }
            publish(app)
        }
        Ok(Err(error)) => fail(app, error),
        Err(_) => fail(app, "检查 GitHub 更新超时，请稍后重试".into()),
    }
}

async fn download(app: &AppHandle) -> View {
    let state = app.state::<State>();
    let settings = state.settings.clone();
    let (candidate, receiver) = {
        let inner = state.inner.lock().unwrap();
        let candidate = inner.candidate.clone().unwrap();
        let receiver = inner.cancel.as_ref().unwrap().subscribe();
        (candidate, receiver)
    };
    let mut last_emit = Instant::now();
    let result = tokio::time::timeout(
        DOWNLOAD_TIMEOUT,
        settings
            .catalog
            .download(&candidate.package, receiver, |done, total| {
                {
                    let mut inner = state.inner.lock().unwrap();
                    inner.view.downloaded = done;
                    inner.view.total = total;
                }
                if last_emit.elapsed() >= Duration::from_millis(100) || done == total {
                    publish(app);
                    last_emit = Instant::now();
                }
            }),
    )
    .await;
    let bytes = match result {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(error)) if error == "下载已取消" => {
            {
                let mut inner = state.inner.lock().unwrap();
                inner.view.phase = "cancelled".into();
                inner.cancel = None;
            }
            return publish(app);
        }
        Ok(Err(error)) => return fail(app, error),
        Err(_) => return fail(app, "下载更新超时，请重新下载".into()),
    };
    let (bytes, targets) = match tauri::async_runtime::spawn_blocking(move || {
        candidate
            .update
            .verify(&bytes)
            .map_err(|error| format!("系统升级器拒绝了更新包：{error}"))?;
        let targets = cli_sync::discover(&settings.home, &settings.sidecar);
        Ok::<_, String>((bytes, targets))
    })
    .await
    {
        Ok(Ok(verified)) => verified,
        Ok(Err(error)) => return fail(app, error),
        Err(_) => return fail(app, "下载后的验证中断".into()),
    };
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.cancel.as_ref().is_some_and(|c| *c.borrow()) {
            inner.view.phase = "cancelled".into();
            inner.cancel = None;
            drop(inner);
            return publish(app);
        }
        inner.view.phase = "ready".into();
        inner.view.cli_targets = targets
            .iter()
            .map(|t| t.path.to_string_lossy().into_owned())
            .collect();
        inner.targets = targets;
        inner.bytes = Some(Arc::new(bytes));
        inner.cancel = None;
    }
    publish(app)
}

async fn install(app: &AppHandle) -> View {
    let state = app.state::<State>();
    let settings = state.settings.clone();
    let (candidate, bytes, targets) = {
        let inner = state.inner.lock().unwrap();
        (
            inner.candidate.clone().unwrap(),
            inner.bytes.clone().unwrap(),
            inner.targets.clone(),
        )
    };
    let validation = candidate
        .update
        .verify(&bytes)
        .map_err(|_| "更新签名或版本校验失败".into())
        .and_then(|_| cli_sync::validate_targets(&targets, &settings.home));
    if let Err(error) = validation {
        return fail(app, error);
    }
    // Validate native signatures before stopping any working backend or writing a receipt.
    // Keep the guard until restart so another update/helper migration cannot race installation.
    #[cfg(target_os = "macos")]
    let _helper_guard = {
        let data = settings.data.clone();
        let package = bytes.clone();
        match tauri::async_runtime::spawn_blocking(move || {
            mcplink_desktop::macos_update::validate_package(&data, &package)
        })
        .await
        {
            Ok(Ok(guard)) => guard,
            Ok(Err(error)) => return fail(app, error),
            Err(_) => return fail(app, "无法完成固定助手更新检查，当前程序保持不变".into()),
        }
    };
    if let Err(error) = crate::background::prepare_update(app).await {
        return fail(app, error);
    }
    let prepare = prepare_receipt(&settings, candidate.package.version.clone(), targets);
    if let Err(error) = prepare {
        crate::background::resume_after_update_failure(app);
        return fail(app, error);
    }
    let result =
        tauri::async_runtime::spawn_blocking(move || candidate.update.install(&bytes[..])).await;
    match result {
        Ok(Ok(())) => app.restart(),
        error => {
            let _ = fs::remove_file(&settings.receipt);
            crate::background::resume_after_update_failure(app);
            let message = match error {
                Ok(Err(e)) => format!("安装未完成：{e}。当前应用已恢复，可重试安装"),
                _ => "安装任务异常结束，当前应用已恢复".into(),
            };
            fail(app, message)
        }
    }
}

fn prepare_receipt(
    settings: &Settings,
    version: semver::Version,
    targets: Vec<CliTarget>,
) -> Result<(), String> {
    #[cfg(windows)]
    if fs::OpenOptions::new()
        .write(true)
        .open(&settings.sidecar)
        .is_err()
    {
        return Err("Agent 仍在使用 mcplink，请先在相关 Agent 中停止 MCP 连接，再重试安装".into());
    }
    cli_sync::write_receipt(&settings.receipt, version, targets, &settings.home)
}

pub async fn command(
    app: AppHandle,
    action: String,
    preview: Option<bool>,
    candidate_id: Option<u64>,
    confirmed: bool,
) -> Result<Value, String> {
    let state = app.state::<State>();
    match action.as_str() {
        "status" => return Ok(json!(snapshot(&app))),
        "open_release" => {
            let url = snapshot(&app)
                .release
                .map_or(RELEASES_URL.into(), |r| r.url);
            opener::open(url).map_err(|_| "无法打开 GitHub 发布页面")?;
            return Ok(json!(snapshot(&app)));
        }
        "cancel" => {
            let inner = state.inner.lock().unwrap();
            if let Some(cancel) = &inner.cancel {
                cancel.send_replace(true);
            }
            return Ok(json!(inner.snapshot()));
        }
        "retry_cli" => {
            if state.inner.lock().unwrap().busy() || crate::background::is_quitting(&app) {
                return Err("升级正在进行，请稍候".into());
            }
            let settings = &state.settings;
            let result = cli_sync::apply_receipt(
                &settings.receipt,
                &settings.home,
                &settings.sidecar,
                &settings.catalog.current,
            );
            {
                let mut inner = state.inner.lock().unwrap();
                inner.view.startup_issue = result.as_ref().err().cloned();
                inner.cli_synced = result.unwrap_or(false);
                if inner.cli_synced {
                    inner.reset_backend = true;
                }
            }
            return Ok(json!(publish(&app)));
        }
        "archive_stale_receipt" => {
            let mut inner = state.inner.lock().unwrap();
            if inner.busy() || crate::background::is_quitting(&app) {
                return Err("升级正在进行，请稍候".into());
            }
            cli_sync::archive_stale_receipt(
                &state.settings.receipt,
                &state.settings.catalog.current,
            )?;
            inner.view.can_archive_stale_receipt = false;
            inner.view.startup_issue = None;
            inner.view.message = Some("旧版升级记录已归档，可以重新检查更新。".into());
            drop(inner);
            return Ok(json!(publish(&app)));
        }
        "check" | "download" | "install" => {}
        _ => return Err("未知软件更新操作".into()),
    }
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.busy() || crate::background::is_quitting(&app) {
            return Err("软件更新或退出正在进行，请稍候".into());
        }
        if !inner.view.supported {
            return Err("当前平台暂未提供应用内升级，请到 GitHub 下载".into());
        }
        match action.as_str() {
            "check" => {
                inner.view.phase = "checking".into();
                inner.view.preview = preview.unwrap_or(inner.view.preview);
                inner.view.candidate_id += 1;
                inner.view.release = None;
                inner.view.downloaded = 0;
                inner.view.total = 0;
                inner.view.cli_targets.clear();
                inner.candidate = None;
                inner.bytes = None;
                inner.targets.clear();
            }
            "download" | "install" => {
                if candidate_id != Some(inner.view.candidate_id) || inner.candidate.is_none() {
                    return Err("所选更新已变化，请重新检查".into());
                }
                if action == "install" && (!confirmed || inner.bytes.is_none()) {
                    return Err("请下载并确认本次更新后再安装".into());
                }
                if action == "download" && inner.bytes.is_some() {
                    return Err("更新包已下载，请确认安装".into());
                }
                inner.view.phase = if action == "download" {
                    inner.view.downloaded = 0;
                    let (sender, _) = watch::channel(false);
                    inner.cancel = Some(sender);
                    "downloading".into()
                } else {
                    "installing".into()
                };
            }
            _ => unreachable!(),
        }
        inner.view.error = None;
    }
    publish(&app);
    let result = match action.as_str() {
        "check" => check(&app, snapshot(&app).preview).await,
        "download" => download(&app).await,
        _ => install(&app).await,
    };
    Ok(json!(result))
}
