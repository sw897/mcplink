#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::Stdio;
use tauri::Manager;
use tokio::io::AsyncWriteExt;

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
    let mut child = tokio::process::Command::new(cli_path(&app)?)
        .args(["rpc", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "无法启动本地管理程序")?;
    let body = serde_json::to_vec(&serde_json::json!({"method":method,"params":params}))
        .map_err(|_| "请求格式错误")?;
    let mut input = child.stdin.take().ok_or("无法建立本地通信")?;
    input.write_all(&body).await.map_err(|_| "本地通信中断")?;
    drop(input);
    let output = child
        .wait_with_output()
        .await
        .map_err(|_| "本地管理程序已退出")?;
    serde_json::from_slice(&output.stdout).map_err(|_| "本地管理程序没有返回有效结果".into())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![rpc])
        .run(tauri::generate_context!())
        .expect("mcplink failed to start");
}
