#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use mcplink_core::runtime::Session;
use std::path::PathBuf;
use tauri::Manager;

#[cfg(any(windows, target_os = "macos"))]
mod background;
#[cfg(any(windows, target_os = "macos"))]
mod updater;

fn cli_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
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
    mcplink_desktop::desktop_rpc::validate(&method, &params)?;
    let transfer = method == "connections.save" && params.get("credentialMappings").is_some();
    rpc_internal(app, method, params, retry_auth, transfer).await
}

async fn rpc_internal(
    app: tauri::AppHandle,
    method: String,
    params: serde_json::Value,
    retry_auth: Option<bool>,
    transfer: bool,
) -> Result<serde_json::Value, String> {
    #[cfg(any(windows, target_os = "macos"))]
    let _activity = background::enter_rpc(&app)?;
    let session = app.state::<Session>();
    if retry_auth == Some(true) {
        session.retry_authentication().await;
    }
    let approved_origin = if transfer {
        Some(confirm_transfer(&app, &session, &params).await?)
    } else {
        None
    };
    let response = tokio::time::timeout(std::time::Duration::from_secs(660), async {
        match approved_origin {
            Some(origin) => session.save_confirmed_transfer(params, origin).await,
            None => session.call(&method, params).await,
        }
    })
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

async fn confirm_transfer(
    app: &tauri::AppHandle,
    session: &Session,
    params: &serde_json::Value,
) -> Result<String, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

    let preview = session
        .call("connections.transfer_preview", params.clone())
        .await
        .map_err(|error| error.to_string())?;
    let origin = preview["origin"].as_str().ok_or("凭据迁移缺少目标地址")?;
    let fields: Vec<(String, String)> =
        serde_json::from_value(preview["fields"].clone()).map_err(|_| "凭据迁移缺少字段清单")?;
    let fields = fields
        .into_iter()
        .map(|(from, to)| format!("{from} → {to}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (answer, confirmed) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(format!("允许将以下已存环境变量用作 {origin} 的请求头？\n\n{fields}\n\n保存后请在账号页重新连接。"))
        .title("确认迁移已存凭据")
        .buttons(MessageDialogButtons::OkCancelCustom("迁移".into(), "取消".into()))
        .show(move |accepted| { let _ = answer.send(accepted); });
    if confirmed.await != Ok(true) {
        return Err("已取消凭据迁移".into());
    }
    Ok(origin.to_owned())
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
        Ok(serde_json::json!({
            "supported": false, "phase": "unsupported", "revision": 0, "target": "unsupported",
            "currentVersion":app.package_info().version.to_string(), "preview": false,
            "candidateId": 0, "downloaded": 0, "total": 0, "cliTargets": [],
            "canDownload": false, "canInstall": false,
            "canArchiveStaleReceipt": false,
        }))
    }
}

fn main() {
    // Startup, polling and MCP recovery never open system password dialogs.
    // First-time helper grants use the explicit reviewed CLI authorization flow.
    mcplink_core::vault::disable_interaction().expect("cannot disable Keychain consent UI");
    let builder = tauri::Builder::default();
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
        .setup(|app| {
            app.manage(Session::new(
                mcplink_core::runtime::data_dir()?,
                cli_path(app.handle())?,
            ));
            #[cfg(any(windows, target_os = "macos"))]
            {
                updater::setup(app)?;
                background::setup(app)?;
            }
            Ok(())
        })
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
