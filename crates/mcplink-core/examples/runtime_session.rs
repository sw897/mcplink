//! Exercise the same process-local Session used by the desktop and Bridge.
use mcplink_core::runtime::{Session, data_dir};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let executable = PathBuf::from(std::env::args().nth(1).expect("CLI path required"));
    let session = Session::new(data_dir()?, executable);
    let output = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut tasks = tokio::task::JoinSet::new();
    while let Some(line) = lines.next_line().await? {
        let value: Value = serde_json::from_str(&line)?;
        let session = session.clone();
        let output = output.clone();
        tasks.spawn(async move {
            if value["retryAuth"] == true {
                session.retry_authentication().await;
            }
            let method = value["method"].as_str().unwrap();
            let params = value.get("params").cloned().unwrap_or(json!({}));
            let result = if method == "process.identity" {
                mcplink_core::vault::verify_running_code().map(|_| json!({"valid":true}))
            } else if value["runningOnly"] == true {
                session.call_running(method, params).await
            } else {
                session.call(method, params).await
            };
            let response = match result {
                Ok(result) => json!({"id":value["id"],"ok":result}),
                Err(error) => json!({"id":value["id"],"error":error.to_string()}),
            };
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            let mut output = output.lock().await;
            output.write_all(&bytes).await.unwrap();
            output.flush().await.unwrap();
        });
    }
    while let Some(result) = tasks.join_next().await {
        result?;
    }
    Ok(())
}
