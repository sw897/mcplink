#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mcplink_core::runtime::Session;
use std::path::PathBuf;
use tauri::Manager;

#[cfg(any(windows, target_os = "macos"))]
mod background;
#[cfg(any(windows, target_os = "macos"))]
mod updater;

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
    retry_auth: Option<bool>,
) -> Result<serde_json::Value, String> {
    #[cfg(any(windows, target_os = "macos"))]
    let _activity = background::enter_rpc(&app, &method)?;
    let session = app.state::<Session>();
    if retry_auth == Some(true) {
        session.retry_authentication().await;
    }
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(660),
        session.call(&method, params),
    )
    .await
    .map_err(|_| "本地管理程序响应超时，执行结果需核对；不要自动重试")?;
    let result = match response {
        Ok(value) => serde_json::json!({"ok":value}),
        Err(error) => {
            if let Some(failure) = error.downcast_ref::<mcplink_core::invocation::ToolFailure>() {
                let mut value = serde_json::json!({"error":{"code":failure.code,"message":failure.to_string()}});
                if !failure.details.is_null() {
                    value["result"] = failure.details.clone();
                }
                value
            } else {
                serde_json::json!({"error":{"code":"operation_failed","message":error.to_string()}})
            }
        }
    };
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

#[tauri::command]
async fn desktop_update(
    app: tauri::AppHandle,
    action: String,
    preview: Option<bool>,
    candidate_id: Option<u64>,
    confirmed: Option<bool>,
) -> Result<serde_json::Value, String> {
    #[cfg(any(windows, target_os = "macos"))]
    return updater::command(
        app,
        action,
        preview,
        candidate_id,
        confirmed.unwrap_or(false),
    )
    .await;
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (preview, candidate_id, confirmed);
        if action == "open_release" {
            opener::open("https://github.com/sw897/mcplink/releases")
                .map_err(|_| "无法打开 GitHub 发布页面")?;
        } else if action != "status" {
            return Err("当前平台暂未提供应用内升级，请到 GitHub 下载".into());
        }
        Ok(
            serde_json::json!({"supported":false,"phase":"unsupported","currentVersion":app.package_info().version.to_string()}),
        )
    }
}

fn main() {
    if std::env::args().any(|arg| arg == "--no-auth-ui") {
        mcplink_core::vault::disable_interaction().expect("cannot disable Keychain consent UI");
    }
    let builder = tauri::Builder::default().setup(|app| {
        app.manage(Session::new(
            mcplink_core::runtime::data_dir()?,
            cli_path(app.handle())?,
        ));
        Ok(())
    });
    #[cfg(any(windows, target_os = "macos"))]
    let builder = builder
        .plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(mcplink_desktop::UPDATE_PUBLIC_KEY.trim())
                .build(),
        )
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
        .setup(|app| {
            app.manage(Session::new(
                mcplink_core::runtime::data_dir()?,
                cli_path(app.handle())?,
            ));
            updater::setup(app)?;
            background::setup(app)
        })
        .on_menu_event(background::menu_event)
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                #[cfg(target_os = "macos")]
                if window.hide().is_ok() {
                    let _ = window
                        .app_handle()
                        .set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
                #[cfg(not(target_os = "macos"))]
                let _ = window.hide();
            }
        });
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            rpc,
            desktop_status,
            desktop_autostart,
            desktop_menu,
            desktop_update
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
            tauri::RunEvent::Reopen { .. } if !background::is_quitting(app) => {
                background::show(app);
            }
            _ => {}
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = (app, event);
    });
}
