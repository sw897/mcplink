//! Opt-in macOS credential worker. Its separately installed bytes stay fixed
//! when the desktop or CLI is rebuilt; secrets only cross private child pipes.
use crate::{model::digest, store::Store, vault};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};
mod lifecycle;
pub use lifecycle::{
    UpdateGuard, authorize, migrate, migration_preview, preview, recover, recovery_preview,
    update_guard,
};

const LIMIT: usize = 16 * 1024 * 1024;
const ID: &str = "io.mcplink.keychain";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    version: u32,
    sha256: String,
}

#[derive(Clone)]
pub struct Helper {
    path: PathBuf,
    binding: Binding,
    data: PathBuf,
}

fn directory(data: &Path) -> PathBuf {
    data.join("keychain-helper")
}

fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    match fs::symlink_metadata(path) {
        Ok(meta) => ensure!(
            meta.is_dir()
                && !meta.file_type().is_symlink()
                && meta.permissions().mode() & 0o022 == 0,
            "助手目录必须是普通目录，不能使用符号链接"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn read_private_file(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("无法检查助手文件；不会回退到其他凭据入口"),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= limit,
        "助手绑定或记录必须是受限的普通文件"
    );
    // O_NOFOLLOW on macOS also rejects a link substituted after lstat.
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .context("无法读取助手文件；不会回退到其他凭据入口")?;
    ensure!(file.metadata()?.is_file(), "助手记录必须是普通文件");
    let mut raw = Vec::new();
    file.take(limit + 1).read_to_end(&mut raw)?;
    ensure!(raw.len() as u64 <= limit, "助手记录过大");
    Ok(Some(raw))
}

fn binding_bytes(data: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(directory(data)) {
        Ok(meta) => ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "助手目录无效"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    read_private_file(&directory(data).join("active.json"), 4096)
}

fn parse_binding(raw: &[u8]) -> Result<Binding> {
    let value: Binding = serde_json::from_slice(raw).context("钥匙串助手绑定无效")?;
    ensure!(
        matches!(value.version, 1 | 2)
            && value.sha256.len() == 64
            && value
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "钥匙串助手绑定或版本无效"
    );
    Ok(value)
}

fn binding_path(data: &Path, binding: &Binding) -> PathBuf {
    if binding.version == 1 {
        directory(data).join("mcplink-keychain")
    } else {
        directory(data)
            .join("versions")
            .join(&binding.sha256)
            .join("mcplink-keychain")
    }
}

impl Helper {
    pub fn configured(data: &Path) -> Result<Option<Self>> {
        let Some(raw) = binding_bytes(data)? else {
            return Ok(None);
        };
        let binding = parse_binding(&raw)?;
        Ok(Some(Self {
            path: binding_path(data, &binding),
            binding,
            data: data.into(),
        }))
    }

    fn candidate(data: &Path, path: &Path) -> Result<Self> {
        ensure!(path.is_absolute(), "助手候选须为绝对路径");
        let helper = Self {
            path: path.into(),
            binding: Binding {
                version: 1,
                sha256: digest(
                    &read_private_file(path, 128 * 1024 * 1024)?.context("无法读取助手候选")?,
                ),
            },
            data: data.into(),
        };
        helper.verify()?;
        Ok(helper)
    }

    fn verify(&self) -> Result<Vec<u8>> {
        vault::verify_running_code()?;
        let certificate = self.verify_pin()?;
        ensure!(
            certificate == crate::keychain_signature::current_certificate()?,
            "钥匙串助手必须与当前 mcplink 使用同一签名证书"
        );
        Ok(certificate)
    }

    fn verify_pin(&self) -> Result<Vec<u8>> {
        let snapshot = self.pinned_snapshot()?;
        crate::keychain_signature::certificate(&snapshot, ID)
    }

    fn pinned_snapshot(&self) -> Result<tempfile::TempPath> {
        if self.binding.version == 2 {
            for parent in [
                directory(&self.data).join("versions"),
                directory(&self.data)
                    .join("versions")
                    .join(&self.binding.sha256),
            ] {
                ensure!(
                    fs::symlink_metadata(parent)
                        .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()),
                    "助手版本目录无效"
                );
            }
        }
        let raw = read_private_file(&self.path, 128 * 1024 * 1024)?
            .context("固定钥匙串助手缺失；不会回退到其他凭据入口")?;
        ensure!(
            self.binding.sha256 == digest(&raw),
            "固定钥匙串助手已改变；请核对单独的助手升级，不自动替换或回退"
        );
        // Security.framework verifies a pathname. Resolve signature identity
        // from the pinned bytes, independently of the live launch pathname.
        let mut snapshot = tempfile::NamedTempFile::with_prefix("mcplink-keychain-")?;
        snapshot.write_all(&raw)?;
        snapshot
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o500))?;
        Ok(snapshot.into_temp_path())
    }

    pub fn request(&self, operation: &str, key: &str, value: Option<&str>) -> Result<Value> {
        let call = || self.request_blocking(operation, key, value);
        if tokio::runtime::Handle::try_current().is_ok_and(|runtime| {
            runtime.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread
        }) {
            tokio::task::block_in_place(call)
        } else {
            call()
        }
    }

    fn request_blocking(&self, operation: &str, key: &str, value: Option<&str>) -> Result<Value> {
        if operation == "authorize" {
            ensure!(!vault::interaction_disabled(), "无弹窗模式不能请求系统授权");
        }
        vault::verify_running_code()?;
        let snapshot = self.pinned_snapshot()?;
        let cdhash = crate::keychain_signature::helper_cdhash(
            &snapshot,
            ID,
            &crate::keychain_signature::current_certificate()?,
        )?;
        drop(snapshot);
        let mut request =
            json!({"version":1,"operation":operation,"dataDirectory":self.data,"key":key});
        if let Some(value) = value {
            request["value"] = json!(value);
        }
        let input = serde_json::to_vec(&request)?;
        ensure!(input.len() <= LIMIT, "钥匙串请求过大");
        let mut command = Command::new(&self.path);
        if operation == "authorize" {
            command.arg("--authorize");
        }
        let mut child = command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("无法启动固定钥匙串助手")?;
        if let Err(error) = crate::keychain_signature::verify_helper_process(child.id(), &cdhash) {
            stop_child(&mut child);
            return Err(error);
        }
        let timeout = Duration::from_secs(if operation == "authorize" { 120 } else { 20 });
        let (status, output) = helper_exchange(&mut child, input, timeout)?;
        let response: Value = serde_json::from_slice(&output).context("钥匙串助手响应无效")?;
        if !status.success() || response["ok"] != true {
            let status = response["status"].as_i64();
            bail!(
                "固定钥匙串助手访问未获允许或身份校验失败（系统状态 {status:?}）；普通操作不会弹窗，请预览并明确授权对应项目"
            );
        }
        Ok(response)
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let response = self.request("get", key, None)?;
        if response["found"] == false {
            return Ok(None);
        }
        Ok(Some(
            response["value"]
                .as_str()
                .context("钥匙串助手响应缺少值")?
                .into(),
        ))
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn helper_exchange(
    child: &mut Child,
    input: Vec<u8>,
    timeout: Duration,
) -> Result<(ExitStatus, Vec<u8>)> {
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        stop_child(child);
        bail!("钥匙串助手通信不可用");
    };
    let deadline = Instant::now() + timeout;
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(&input));
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        scope.spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take((LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = send.send(result);
        });
        let result = (|| {
            let output = receive
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .context("固定钥匙串助手响应超时")??;
            ensure!(output.len() <= LIMIT, "钥匙串助手响应过大");
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                ensure!(Instant::now() < deadline, "固定钥匙串助手退出超时");
                std::thread::sleep(Duration::from_millis(5));
            };
            writer
                .join()
                .map_err(|_| anyhow::anyhow!("助手写入任务中断"))?
                .context("固定钥匙串助手拒绝请求")?;
            Ok((status, output))
        })();
        if result.is_err() {
            stop_child(child);
        }
        result
    })
}

fn credential_items(data: &Path, operations: &[String]) -> Result<Vec<Value>> {
    let mut items =
        vec![json!({"key":crate::runtime_auth::VAULT_KEY,"label":"本机后台通信","kind":"runtime"})];
    let store = Store::open(data)?;
    for connection in store.connections()? {
        items.push(
            json!({"key":connection.definition_ref,"label":connection.title,"kind":"definition"}),
        );
        if connection.auth_kind == "oauth" {
            items.push(json!({"key":format!("oauth/{}",connection.id),"label":connection.title,"kind":"oauth"}));
        }
    }
    for operation in store.pending_operations()? {
        if let Some(key) = operation["credentialRef"].as_str() {
            items.push(json!({"key":key,"label":"待恢复的配置事务","kind":"recovery"}));
        }
    }
    for (_, key) in store
        .pending_credential_cleanup()?
        .into_iter()
        .chain(store.pending_credential_deletions()?)
    {
        items.push(json!({"key":key,"label":"待清理的已提交连接定义","kind":"cleanup"}));
    }
    for id in operations {
        let operation = store.operation(id)?;
        let key = operation["credentialRef"]
            .as_str()
            .context("历史变更缺少凭据引用")?;
        items.push(json!({"key":key,"label":format!("历史变更 {id}"),"kind":"history"}));
    }
    items.sort_by_key(|v| v["key"].as_str().unwrap_or_default().to_owned());
    items.dedup_by(|a, b| a["key"] == b["key"]);
    Ok(items)
}
