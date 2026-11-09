#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::Stdio;
use tauri::Manager;
use tokio::io::AsyncWriteExt;

#[cfg(any(windows, target_os = "macos"))]
mod background;

fn cli_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("MCPLINK_CLI") {
        return Ok(path.into());
    }
    let binary = if cfg!(windows) {
        "mcplink.exe"
    } else {
        "mcplink"
    };
    let executable = std::env::current_exe().map_err(|_| "无法定位应用目录")?;
    let sibling = executable.parent().ok_or("无法定位应用目录")?.join(binary);
    if sibling.is_file() {
        return Ok(sibling);
    }
    let resource = app
        .path()
        .resource_dir()
        .map_err(|_| "无法定位应用资源")?
        .join(binary);
    if resource.is_file() {
        return Ok(resource);
    }
    #[cfg(debug_assertions)]
    {
        let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/debug")
            .join(binary);
        if development.is_file() {
            return Ok(development);
        }
    }
    Err("未找到 mcplink 后台程序，请重新安装应用".into())
}

#[tauri::command]
async fn rpc(
    app: tauri::AppHandle,
    method: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    #[cfg(any(windows, target_os = "macos"))]
    let _activity = background::enter_rpc(&app, &method)?;
    let mut command = tokio::process::Command::new(cli_path(&app)?);
    command
        .args(["rpc", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // RPC helpers must not create a console window or steal the GUI's focus.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|_| "无法启动本地管理程序")?;
    let body = serde_json::to_vec(&serde_json::json!({"method":method,"params":params}))
        .map_err(|_| "请求格式错误")?;
    let mut input = child.stdin.take().ok_or("无法建立本地通信")?;
    input.write_all(&body).await.map_err(|_| "本地通信中断")?;
    drop(input);
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(660),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "本地管理程序响应超时，执行结果需核对")?
    .map_err(|_| "本地管理程序已退出")?;
    let result = serde_json::from_slice(&output.stdout)
        .map_err(|_| "本地管理程序没有返回有效结果".to_string())?;
    #[cfg(any(windows, target_os = "macos"))]
    background::observe(&app, &method, &result);
    Ok(result)
}

#[tauri::command]
fn desktop_status(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(any(windows, target_os = "macos"))]
    return background::status(&app);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = app;
        Ok(serde_json::json!({"supported":false}))
    }
}

#[tauri::command]
fn desktop_autostart(app: tauri::AppHandle, enabled: bool) -> Result<serde_json::Value, String> {
    #[cfg(any(windows, target_os = "macos"))]
    return background::set_autostart(&app, enabled);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (app, enabled);
        Err("当前平台尚未提供开机自启设置".into())
    }
}

#[tauri::command]
fn desktop_menu(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(any(windows, target_os = "macos"))]
    return background::popup(&app, &window);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (app, window);
        Err("当前平台尚未提供托盘菜单".into())
    }
}

fn main() {
    let builder = tauri::Builder::default();
    #[cfg(any(windows, target_os = "macos"))]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !args.iter().any(|arg| arg == "--background") {
                background::show(app);
            }
        }))
        .plugin({
            let plugin = tauri_plugin_autostart::Builder::new()
                .app_name("mcplink")
                .args(["--background"]);
            #[cfg(target_os = "macos")]
            let plugin = plugin.macos_launcher(tauri_plugin_autostart::MacosLauncher::LaunchAgent);
            plugin.build()
        })
        .setup(|app| background::setup(app))
        .on_menu_event(background::menu_event)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        });
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            rpc,
            desktop_status,
            desktop_autostart,
            desktop_menu
        ])
        .build(tauri::generate_context!())
        .expect("mcplink failed to start");
    app.run(|app, event| {
        #[cfg(any(windows, target_os = "macos"))]
        match event {
            tauri::RunEvent::ExitRequested { api, .. } if !background::may_exit(app) => {
                api.prevent_exit();
                background::quit(app);
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                if !background::is_quitting(app) {
                    background::show(app);
                }
            }
            _ => {}
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = (app, event);
    });
}
