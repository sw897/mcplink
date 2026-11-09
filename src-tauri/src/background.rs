use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
#[cfg(windows)]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_autostart::ManagerExt;

struct Background {
    menu: Menu<tauri::Wry>,
    pause: CheckMenuItem<tauri::Wry>,
    autostart: CheckMenuItem<tauri::Wry>,
    health: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
    keep: MenuItem<tauri::Wry>,
    state: AtomicU8,
    action: tokio::sync::Mutex<()>,
    quitting: AtomicBool,
    allow_exit: AtomicBool,
    monitor: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    monitor_stop: tokio::sync::watch::Sender<bool>,
    requests: Mutex<usize>,
    drained: tokio::sync::Notify,
}

pub struct RpcActivity(AppHandle);

impl Drop for RpcActivity {
    fn drop(&mut self) {
        let state = self.0.state::<Background>();
        let mut active = state.requests.lock().unwrap();
        *active -= 1;
        if *active == 0 {
            state.drained.notify_one();
        }
    }
}

pub fn enter_rpc(app: &AppHandle, method: &str) -> Result<Option<RpcActivity>, String> {
    if method == "gateway.stop" {
        return Ok(None);
    }
    let state = app.state::<Background>();
    let mut active = state.requests.lock().unwrap();
    if state.quitting.load(Ordering::SeqCst) {
        return Err("应用正在退出，请等待共享服务清理完成".into());
    }
    *active += 1;
    Ok(Some(RpcActivity(app.clone())))
}

async fn drain_requests(app: &AppHandle) {
    let state = app.state::<Background>();
    loop {
        let idle = state.drained.notified();
        if *state.requests.lock().unwrap() == 0 {
            return;
        }
        idle.await;
    }
}

pub fn is_quitting(app: &AppHandle) -> bool {
    app.try_state::<Background>()
        .is_some_and(|state| state.quitting.load(Ordering::SeqCst))
}
pub fn may_exit(app: &AppHandle) -> bool {
    app.try_state::<Background>()
        .is_none_or(|state| state.allow_exit.load(Ordering::SeqCst))
}
pub fn show(app: &AppHandle) {
    if is_quitting(app) {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        let _ = app.show();
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
pub fn popup(app: &AppHandle, window: &tauri::WebviewWindow) -> Result<(), String> {
    if is_quitting(app) {
        return Err("应用正在退出".into());
    }
    window
        .popup_menu(&app.state::<Background>().menu)
        .map_err(|error| error.to_string())
}
pub fn status(app: &AppHandle) -> Result<Value, String> {
    let enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    Ok(
        json!({"supported":true,"autostartEnabled":enabled,"closeToTray":true,"platform":std::env::consts::OS}),
    )
}
pub fn set_autostart(app: &AppHandle, enabled: bool) -> Result<Value, String> {
    if is_quitting(app) {
        return Err("应用正在退出".into());
    }
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(|e| e.to_string())?;
    let result = status(app)?;
    app.state::<Background>()
        .autostart
        .set_checked(result["autostartEnabled"] == true)
        .map_err(|e| e.to_string())?;
    let _ = app.emit("desktop-status-changed", &result);
    Ok(result)
}
pub fn observe(app: &AppHandle, method: &str, response: &Value) {
    let Some(state) = app.try_state::<Background>() else {
        return;
    };
    if is_quitting(app) {
        return;
    }
    let value = if method == "snapshot" {
        &response["ok"]["gateway"]
    } else {
        &response["ok"]
    };
    if !matches!(
        method,
        "snapshot" | "gateway.status" | "gateway.pause" | "gateway.resume"
    ) {
        return;
    }
    if let Some(paused) = value["paused"].as_bool() {
        let next = if paused { 2 } else { 1 };
        let _ = state.pause.set_checked(paused);
        let _ = state.pause.set_enabled(true);
        let label = if paused {
            "共享已暂停（重启后保持）"
        } else {
            "共享后台运行中"
        };
        let _ = state.health.set_text(label);
        if let Some(tray) = app.tray_by_id("mcplink") {
            let _ = tray.set_tooltip(Some(format!("mcplink · {label}")));
        }
        if state.state.swap(next, Ordering::SeqCst) != next {
            let _ = app.emit("gateway-changed", ());
        }
    }
}
async fn gateway(app: &AppHandle, method: &str) -> Result<Value, String> {
    let timeout = if method == "gateway.stop" { 50 } else { 20 };
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(timeout),
        crate::rpc(app.clone(), method.into(), json!({})),
    )
    .await
    .map_err(|_| "连接共享后台超时".to_string())??;
    response.get("ok").cloned().ok_or_else(|| {
        response["error"]["message"]
            .as_str()
            .unwrap_or("共享后台操作失败")
            .to_string()
    })
}
fn report_error(app: &AppHandle, message: String) {
    let _ = app
        .state::<Background>()
        .health
        .set_text("后台操作失败，请打开主窗口检查");
    let _ = app.emit("desktop-error", message);
}
fn stop_monitor(app: &AppHandle) -> Option<tauri::async_runtime::JoinHandle<()>> {
    let state = app.state::<Background>();
    state.monitor_stop.send_replace(true);
    state.monitor.lock().unwrap().take()
}
fn start_monitor(app: &AppHandle) {
    let state = app.state::<Background>();
    state.monitor_stop.send_replace(false);
    let mut stop = state.monitor_stop.subscribe();
    let handle = app.clone();
    let task = tauri::async_runtime::spawn(async move {
        loop {
            if is_quitting(&handle) {
                break;
            }
            if gateway(&handle, "gateway.status").await.is_err() {
                let state = handle.state::<Background>();
                state.state.store(0, Ordering::SeqCst);
                let _ = state.pause.set_enabled(false);
                let _ = state.health.set_text("后台未连接（正在自动重试）");
                if let Some(tray) = handle.tray_by_id("mcplink") {
                    let _ = tray.set_tooltip(Some("mcplink · 后台未连接，正在重试"));
                }
            }
            tokio::select! {
                _ = stop.wait_for(|stopped| *stopped) => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {},
            }
        }
    });
    *app.state::<Background>().monitor.lock().unwrap() = Some(task);
}
pub fn quit(app: &AppHandle) {
    let state = app.state::<Background>();
    {
        let _active = state.requests.lock().unwrap();
        if state.quitting.swap(true, Ordering::SeqCst) {
            return;
        }
    }
    let monitor = stop_monitor(app);
    let _ = state.health.set_text("正在退出并清理共享服务…");
    let _ = state.quit.set_enabled(false);
    let _ = state.keep.set_enabled(false);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let cleaned = async {
            // Let an in-progress startup finish before stopping its daemon.
            // Aborting its helper here could leave a newly spawned daemon behind.
            if let Some(monitor) = monitor {
                monitor
                    .await
                    .map_err(|_| "后台巡检未能正常结束".to_string())?;
            }
            gateway(&app, "gateway.stop").await?;
            tokio::time::timeout(std::time::Duration::from_secs(30), drain_requests(&app))
                .await
                .map_err(|_| "桌面管理请求尚未结束，请等待后重试".to_string())?;
            // A helper accepted before quitting may have just finished starting
            // a daemon. No new desktop requests can enter after the drain.
            gateway(&app, "gateway.stop").await?;
            Ok::<_, String>(())
        }
        .await;
        match cleaned {
            Ok(_) => {
                app.state::<Background>()
                    .allow_exit
                    .store(true, Ordering::SeqCst);
                app.exit(0);
            }
            Err(error) => {
                let state = app.state::<Background>();
                state.quitting.store(false, Ordering::SeqCst);
                let _ = state.quit.set_enabled(true);
                let _ = state.keep.set_enabled(true);
                start_monitor(&app);
                show(&app);
                report_error(&app, format!("退出清理未完成：{error}"));
            }
        }
    });
}
pub fn menu_event(app: &AppHandle, event: MenuEvent) {
    if is_quitting(app) {
        return;
    }
    match event.id.as_ref() {
        "tray.open" => show(app),
        "tray.quit_all" => quit(app),
        "tray.quit_keep" => {
            let state = app.state::<Background>();
            state.quitting.store(true, Ordering::SeqCst);
            state.allow_exit.store(true, Ordering::SeqCst);
            if let Some(monitor) = stop_monitor(app) {
                monitor.abort();
            }
            app.exit(0);
        }
        "tray.autostart" => {
            let result =
                status(app).and_then(|value| set_autostart(app, value["autostartEnabled"] != true));
            if let Err(error) = result {
                if let Ok(value) = status(app) {
                    let _ = app
                        .state::<Background>()
                        .autostart
                        .set_checked(value["autostartEnabled"] == true);
                }
                report_error(app, error);
            }
        }
        "tray.pause" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<Background>();
                let _guard = state.action.lock().await;
                let result = async {
                    let value = gateway(&app, "gateway.status").await?;
                    gateway(
                        &app,
                        if value["paused"] == true {
                            "gateway.resume"
                        } else {
                            "gateway.pause"
                        },
                    )
                    .await
                }
                .await;
                if let Err(error) = result {
                    let _ = state
                        .pause
                        .set_checked(state.state.load(Ordering::SeqCst) == 2);
                    report_error(&app, error);
                }
            });
        }
        _ => {}
    }
}
pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    let health = MenuItem::with_id(app, "tray.health", "正在连接共享后台…", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "tray.open", "显示主窗口", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "tray.pause", "暂停共享", false, false, None::<&str>)?;
    let startup_label = if cfg!(target_os = "macos") {
        "登录时启动（仅菜单栏）"
    } else {
        "开机启动（仅托盘）"
    };
    let autostart = CheckMenuItem::with_id(
        app,
        "tray.autostart",
        startup_label,
        true,
        app.autolaunch().is_enabled()?,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "tray.quit_all",
        "退出 mcplink（停止共享）",
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let keep = MenuItem::with_id(
        app,
        "tray.quit_keep",
        "只退出界面（代理继续运行）",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &health,
            &open,
            &pause,
            &separator,
            &autostart,
            &separator2,
            &keep,
            &quit,
        ],
    )?;
    app.manage(Background {
        menu: menu.clone(),
        pause,
        autostart,
        health,
        quit,
        keep,
        state: AtomicU8::new(0),
        action: tokio::sync::Mutex::new(()),
        quitting: AtomicBool::new(false),
        allow_exit: AtomicBool::new(false),
        monitor: Mutex::new(None),
        monitor_stop: tokio::sync::watch::channel(false).0,
        requests: Mutex::new(0),
        drained: tokio::sync::Notify::new(),
    });
    #[cfg(windows)]
    let icon = app.default_window_icon().ok_or("缺少托盘图标")?.clone();
    #[cfg(target_os = "macos")]
    let icon = tauri::include_image!("icons/tray-template.png");
    let tray = TrayIconBuilder::with_id("mcplink")
        .icon(icon)
        .tooltip("mcplink · 正在连接共享后台")
        .menu(&menu);
    #[cfg(windows)]
    let tray = tray
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show(tray.app_handle());
            }
        });
    #[cfg(target_os = "macos")]
    let tray = tray.icon_as_template(true).show_menu_on_left_click(true);
    tray.build(app)?;
    if !std::env::args_os().any(|arg| arg == "--background") {
        show(app.handle());
    }
    start_monitor(app.handle());
    #[cfg(target_os = "macos")]
    {
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut terminate) => {
                    tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
                    quit(&handle);
                }
                Err(error) => report_error(&handle, format!("无法监听应用退出信号：{error}")),
            }
        });
    }
    Ok(())
}
