//! Opt-in macOS credential worker. Its separately installed bytes stay fixed
//! when the desktop or CLI is rebuilt; secrets only cross private child pipes.
use crate::{model::digest, store::Store, vault};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const LIMIT: usize = 16 * 1024 * 1024;
const ID: &str = "io.mcplink.keychain";

#[derive(Clone, Deserialize, Serialize)]
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

impl Helper {
    pub fn configured(data: &Path) -> Result<Option<Self>> {
        let path = directory(data).join("active.json");
        let raw = match fs::read(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => bail!("无法读取固定钥匙串助手绑定；不会回退到其他凭据入口"),
        };
        ensure!(
            raw.len() <= 4096 && !path.is_symlink(),
            "钥匙串助手绑定无效"
        );
        let binding: Binding = serde_json::from_slice(&raw).context("钥匙串助手绑定无效")?;
        ensure!(binding.version == 1, "钥匙串助手协议不兼容");
        Ok(Some(Self {
            path: directory(data).join("mcplink-keychain"),
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
                sha256: digest(&fs::read(path).context("无法读取助手候选")?),
            },
            data: data.into(),
        };
        helper.verify()?;
        Ok(helper)
    }

    fn verify(&self) -> Result<()> {
        vault::verify_running_code()?;
        ensure!(
            !self.path.is_symlink()
                && fs::symlink_metadata(&self.path).is_ok_and(|m| m.is_file())
                && self.binding.sha256 == digest(&fs::read(&self.path)?),
            "固定钥匙串助手已改变；请核对单独的助手升级，不自动替换或回退"
        );
        verify_identity(&self.path)
    }

    pub fn request(&self, operation: &str, key: &str, value: Option<&str>) -> Result<Value> {
        self.verify()?;
        let mut request =
            json!({"version":1,"operation":operation,"dataDirectory":self.data,"key":key});
        if let Some(value) = value {
            request["value"] = json!(value);
        }
        let input = serde_json::to_vec(&request)?;
        ensure!(input.len() <= LIMIT, "钥匙串请求过大");
        let mut command = Command::new(&self.path);
        if operation == "authorize" {
            ensure!(!vault::interaction_disabled(), "无弹窗模式不能请求系统授权");
            command.arg("--authorize");
        }
        let mut child = command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("无法启动固定钥匙串助手")?;
        if child
            .stdin
            .take()
            .context("助手输入不可用")?
            .write_all(&input)
            .is_err()
        {
            let _ = child.kill();
            let _ = child.wait();
            bail!("固定钥匙串助手拒绝请求");
        }
        let output = child.wait_with_output().context("固定钥匙串助手通信失败")?;
        ensure!(output.stdout.len() <= LIMIT, "钥匙串助手响应过大");
        let response: Value =
            serde_json::from_slice(&output.stdout).context("钥匙串助手响应无效")?;
        if !output.status.success() || response["ok"] != true {
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

pub fn preview(data: &Path, candidate: &Path, operations: &[String]) -> Result<Value> {
    let helper = Helper::candidate(data, candidate)?;
    ensure!(data.is_absolute(), "保留原有绝对数据目录和 namespace");
    let mut items = vec![json!({"key":"runtime-key","label":"本机后台通信","kind":"runtime"})];
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
    let mut result = json!({"version":1,"helperIdentifier":ID,"helperSha256":helper.binding.sha256,
        "installedPath":directory(data).join("mcplink-keychain"),"dataDirectory":data,"items":items,
        "credentialValuesRead":false,"credentialsMoved":false,"historyPolicy":"Old completed plans/changes stay in Keychain; authorize explicitly if later needed."});
    result["previewHash"] = json!(digest(&serde_json::to_vec(&result)?));
    Ok(result)
}

pub fn authorize(
    data: &Path,
    candidate: &Path,
    reviewed_hash: &str,
    operations: &[String],
) -> Result<Value> {
    ensure!(!vault::interaction_disabled(), "无弹窗模式不能请求系统授权");
    let plan = preview(data, candidate, operations)?;
    ensure!(
        plan["previewHash"] == reviewed_hash,
        "授权清单或助手已改变，请重新预览和确认"
    );
    let prepared = Helper::candidate(data, candidate)?;
    let dir = directory(data);
    ensure!(!dir.is_symlink(), "助手安装目录不能是符号链接");
    vault::private_dir(&dir)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let target = dir.join("mcplink-keychain");
    if target.exists() {
        ensure!(
            !target.is_symlink() && digest(&fs::read(&target)?) == prepared.binding.sha256,
            "已安装不同助手；助手升级须单独处理，不自动覆盖"
        );
    } else {
        let raw = fs::read(candidate)?;
        ensure!(digest(&raw) == prepared.binding.sha256, "助手候选已改变");
        // create_new prevents overwriting a concurrent or existing helper.
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&target)?;
        file.write_all(&raw)?;
        file.sync_all()?;
    }
    let installed = Helper {
        path: target,
        ..prepared
    };
    installed.verify()?;
    let mut authorized = Vec::new();
    for item in plan["items"].as_array().context("授权清单无效")? {
        let key = item["key"].as_str().context("授权项目无效")?;
        let response = installed.request("authorize", key, None)?;
        ensure!(
            item["kind"] != "definition" || response["found"] == true,
            "当前托管定义缺少钥匙串项目；未启用助手，原凭据保持不变"
        );
        authorized
            .push(json!({"label":item["label"],"kind":item["kind"],"found":response["found"]}));
    }
    ensure!(
        preview(data, candidate, operations)?["previewHash"] == plan["previewHash"],
        "授权期间清单已改变；未启用助手，请重新预览"
    );
    vault::private_write(
        &dir.join("active.json"),
        &serde_json::to_vec(&installed.binding)?,
    )?;
    Ok(
        json!({"enabled":true,"helperSha256":installed.binding.sha256,"items":authorized,
              "credentialsMoved":false,"reload":"Restart mcplink desktop/backend and reload Agent MCP connections; existing processes keep their previous Vault."}),
    )
}

fn verify_identity(path: &Path) -> Result<()> {
    use std::{ffi::c_void, os::unix::ffi::OsStrExt};
    type Ref = *const c_void;
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecCodeCopySelf(flags: u32, code: *mut Ref) -> i32;
        fn SecCodeCopyStaticCode(code: Ref, flags: u32, static_code: *mut Ref) -> i32;
        fn SecStaticCodeCreateWithPath(path: Ref, flags: u32, code: *mut Ref) -> i32;
        fn SecRequirementCreateWithString(text: Ref, flags: u32, requirement: *mut Ref) -> i32;
        fn SecStaticCodeCheckValidity(code: Ref, flags: u32, requirement: Ref) -> i32;
        fn SecCodeCopySigningInformation(code: Ref, flags: u32, info: *mut Ref) -> i32;
        static kSecCodeInfoCertificates: Ref;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: Ref,
            bytes: *const u8,
            len: isize,
            directory: u8,
        ) -> Ref;
        fn CFStringCreateWithCString(
            allocator: Ref,
            bytes: *const std::ffi::c_char,
            encoding: u32,
        ) -> Ref;
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFArrayGetCount(array: Ref) -> isize;
        fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
        fn CFEqual(first: Ref, second: Ref) -> u8;
        fn CFRelease(value: Ref);
    }
    struct Owned(Ref);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }
    let bytes = path.as_os_str().as_bytes();
    let none = std::ptr::null();
    unsafe {
        let url = Owned(CFURLCreateFromFileSystemRepresentation(
            none,
            bytes.as_ptr(),
            bytes.len() as isize,
            0,
        ));
        ensure!(!url.0.is_null(), "钥匙串助手路径无效");
        let mut helper = Owned(none);
        let mut own = Owned(none);
        let mut own_static = Owned(none);
        let text = Owned(CFStringCreateWithCString(
            none,
            c"identifier \"io.mcplink.keychain\"".as_ptr(),
            0x08000100,
        ));
        let mut requirement = Owned(none);
        ensure!(
            SecStaticCodeCreateWithPath(url.0, 0, &mut helper.0) == 0
                && SecCodeCopySelf(0, &mut own.0) == 0
                && SecCodeCopyStaticCode(own.0, 0, &mut own_static.0) == 0
                && SecRequirementCreateWithString(text.0, 0, &mut requirement.0) == 0
                && SecStaticCodeCheckValidity(helper.0, 1 << 4, requirement.0) == 0,
            "固定钥匙串助手签名或标识无效"
        );
        let mut a = Owned(none);
        let mut b = Owned(none);
        ensure!(
            SecCodeCopySigningInformation(helper.0, 1 << 1, &mut a.0) == 0
                && SecCodeCopySigningInformation(own_static.0, 1 << 1, &mut b.0) == 0,
            "无法核对钥匙串助手签名身份"
        );
        let ac = CFDictionaryGetValue(a.0, kSecCodeInfoCertificates);
        let bc = CFDictionaryGetValue(b.0, kSecCodeInfoCertificates);
        ensure!(
            !ac.is_null()
                && !bc.is_null()
                && CFArrayGetCount(ac) > 0
                && CFArrayGetCount(bc) > 0
                && CFEqual(CFArrayGetValueAtIndex(ac, 0), CFArrayGetValueAtIndex(bc, 0)) != 0,
            "钥匙串助手必须与当前 mcplink 使用同一签名证书"
        );
    }
    Ok(())
}
