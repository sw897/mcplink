//! Explicit, metadata-only plans and serialized helper lifecycle transactions.
use super::*;
use std::{fs::File, os::unix::fs::OpenOptionsExt};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    previous_json: String,
    next: Binding,
    preview_hash: String,
}

fn root(data: &Path) -> Result<PathBuf> {
    ensure!(
        data.is_absolute() && !data.is_symlink(),
        "保留原有绝对数据目录和 namespace"
    );
    vault::private_dir(data)?;
    use std::os::unix::fs::PermissionsExt;
    ensure!(
        fs::metadata(data)?.permissions().mode() & 0o022 == 0,
        "助手数据目录不能允许其他用户写入"
    );
    let dir = directory(data);
    private_directory(&dir)?;
    Ok(dir)
}

fn lock(data: &Path) -> Result<File> {
    let path = root(data)?.join(".lifecycle.lock");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "助手事务锁无效"
        );
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(0x100)
        .open(path)?;
    fs2::FileExt::try_lock_exclusive(&file)
        .context("助手迁移、恢复或程序更新正在进行，请等待完成后重新预览")?;
    Ok(file)
}

pub struct UpdateGuard {
    _lock: File,
    helper: Option<Helper>,
}

impl UpdateGuard {
    pub fn has_helper(&self) -> bool {
        self.helper.is_some()
    }

    pub fn validate_app(&self, app: &Path) -> Result<()> {
        let Some(helper) = &self.helper else {
            return Ok(());
        };
        helper.verify()?;
        let expected = crate::keychain_signature::certificate(&helper.path, ID)?;
        for (path, identifier) in [
            (app.to_owned(), "io.mcplink.desktop"),
            (
                app.join("Contents/MacOS/mcplink-desktop"),
                "io.mcplink.desktop",
            ),
            (app.join("Contents/MacOS/mcplink"), "io.mcplink.cli"),
        ] {
            ensure!(
                crate::keychain_signature::certificate(&path, identifier)? == expected,
                "更新程序与固定助手的签名证书不兼容；已保留当前程序，请使用本机签名部署或单独审阅助手迁移"
            );
            if helper.binding.version > 1
                && path
                    .file_name()
                    .is_some_and(|name| name != "mcplink-desktop")
            {
                ensure!(
                    crate::keychain_signature::binding_version(&path)? >= helper.binding.version,
                    "更新程序不支持当前助手绑定版本；已保留当前程序"
                );
            }
        }
        Ok(())
    }
}

/// Keep the exclusive lifecycle lock alive until installation/restart.
pub fn update_guard(data: &Path) -> Result<UpdateGuard> {
    let guard = lock(data)?;
    let helper = Helper::configured(data)?;
    if let Some(helper) = &helper {
        helper.verify()?;
    }
    Ok(UpdateGuard {
        _lock: guard,
        helper,
    })
}

fn finish_plan(mut value: Value) -> Result<Value> {
    value["previewHash"] = json!(digest(&serde_json::to_vec(&value)?));
    Ok(value)
}

fn check_plan(plan: &Value, reviewed: &str) -> Result<()> {
    ensure!(
        plan["previewHash"] == reviewed,
        "授权清单、助手或恢复状态已改变，请重新预览和确认"
    );
    Ok(())
}

fn write_metadata(path: &Path, raw: &[u8]) -> Result<()> {
    vault::private_write(path, raw)?;
    File::open(path.parent().context("助手元数据路径无效")?)?.sync_all()?;
    Ok(())
}

fn receipt_bytes(data: &Path) -> Result<Option<Vec<u8>>> {
    read_private_file(&directory(data).join("rollback.json"), 16 * 1024)
}

fn parse_receipt(raw: &[u8]) -> Result<Receipt> {
    let receipt: Receipt = serde_json::from_slice(raw).context("助手恢复记录无效")?;
    let before = parse_binding(receipt.previous_json.as_bytes())?;
    parse_binding(&serde_json::to_vec(&receipt.next)?)?;
    ensure!(
        receipt.version == 1
            && receipt.next.version == 2
            && before != receipt.next
            && receipt.preview_hash.len() == 64,
        "助手恢复记录无效"
    );
    Ok(receipt)
}

pub fn preview(data: &Path, candidate: &Path, operations: &[String]) -> Result<Value> {
    let helper = Helper::candidate(data, candidate)?;
    ensure!(data.is_absolute(), "保留原有绝对数据目录和 namespace");
    let active = binding_bytes(data)?;
    let installed = Helper::configured(data)?;
    let target = if let Some(installed) = installed {
        installed.verify()?;
        ensure!(
            installed.binding.sha256 == helper.binding.sha256,
            "已接入不同助手；请使用 keychain migrate-preview / migrate 单独迁移"
        );
        installed.path
    } else {
        let target = directory(data).join("mcplink-keychain");
        if fs::symlink_metadata(&target).is_ok() {
            ensure!(
                read_private_file(&target, 128 * 1024 * 1024)?
                    .is_some_and(|raw| digest(&raw) == helper.binding.sha256),
                "存在未接入的不同或不完整助手；先用 keychain recover-preview / recover 核对恢复"
            );
        }
        target
    };
    finish_plan(json!({
        "version":1,"action":"authorize","helperIdentifier":ID,"helperSha256":helper.binding.sha256,
        "installedPath":target,"dataDirectory":data,"items":credential_items(data, operations)?,
        "activeBindingSha256":active.as_ref().map(|v| digest(v)),
        "credentialValuesRead":false,"credentialsMoved":false,
        "historyPolicy":"Old completed plans/changes stay in Keychain; authorize explicitly if later needed."
    }))
}

struct Stage(PathBuf);
impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn install_atomic(prepared: &Helper, target: &Path) -> Result<Helper> {
    if fs::symlink_metadata(target).is_ok() {
        ensure!(
            read_private_file(target, 128 * 1024 * 1024)?
                .is_some_and(|raw| digest(&raw) == prepared.binding.sha256),
            "已存在不同或不完整助手；先预览恢复，不覆盖原文件"
        );
    } else {
        let parent = target.parent().context("助手安装路径无效")?;
        let stage = Stage(parent.join(format!(".install-{}.tmp", uuid::Uuid::new_v4())));
        let raw = fs::read(&prepared.path).context("无法读取助手候选")?;
        ensure!(digest(&raw) == prepared.binding.sha256, "助手候选已改变");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&stage.0)?;
        file.write_all(&raw)?;
        file.sync_all()?;
        let staged = Helper {
            path: stage.0.clone(),
            ..prepared.clone()
        };
        staged.verify()?;
        // Same-directory hard link publishes complete bytes atomically without clobbering a winner.
        match fs::hard_link(&stage.0, target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                ensure!(
                    read_private_file(target, 128 * 1024 * 1024)?
                        .is_some_and(|raw| digest(&raw) == prepared.binding.sha256),
                    "助手目标已并发改变；未覆盖"
                );
            }
            Err(e) => return Err(e.into()),
        }
        File::open(parent)?.sync_all()?;
    }
    let helper = Helper {
        path: target.into(),
        ..prepared.clone()
    };
    helper.verify()?;
    Ok(helper)
}

fn check_grants(helper: &Helper, plan: &Value, interactive: bool) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for item in plan["items"].as_array().context("授权清单无效")? {
        let response = helper.request(
            if interactive { "authorize" } else { "get" },
            item["key"].as_str().context("授权项目无效")?,
            None,
        )?;
        ensure!(
            item["kind"] != "definition" || response["found"] == true,
            "当前托管定义缺少钥匙串项目；未启用助手，原凭据保持不变"
        );
        result.push(json!({"label":item["label"],"kind":item["kind"],"found":response["found"]}));
        // A no-UI check may read a pre-authorized value internally; it is never returned or stored.
    }
    Ok(result)
}

pub fn authorize(
    data: &Path,
    candidate: &Path,
    reviewed: &str,
    operations: &[String],
) -> Result<Value> {
    ensure!(!vault::interaction_disabled(), "无弹窗模式不能请求系统授权");
    let _lock = lock(data)?;
    let plan = preview(data, candidate, operations)?;
    check_plan(&plan, reviewed)?;
    let prepared = Helper::candidate(data, candidate)?;
    let current = Helper::configured(data)?;
    let installed = if let Some(helper) = current {
        helper.verify()?;
        helper
    } else {
        install_atomic(&prepared, &directory(data).join("mcplink-keychain"))?
    };
    let authorized = check_grants(&installed, &plan, true)?;
    check_plan(&preview(data, candidate, operations)?, reviewed)?;
    if binding_bytes(data)?.is_none() {
        write_metadata(
            &directory(data).join("active.json"),
            &serde_json::to_vec(&installed.binding)?,
        )?;
    }
    Ok(
        json!({"enabled":true,"helperSha256":installed.binding.sha256,"items":authorized,
              "credentialsMoved":false,"reload":"Restart mcplink desktop/backend and reload Agent MCP connections."}),
    )
}

pub fn migration_preview(data: &Path, candidate: &Path, operations: &[String]) -> Result<Value> {
    ensure!(
        data.is_absolute() && !data.is_symlink(),
        "保留原有绝对数据目录和 namespace"
    );
    let old_raw = binding_bytes(data)?
        .context("尚未接入助手；首次接入请使用 keychain preview / authorize")?;
    let old = Helper::configured(data)?.context("缺少原助手")?;
    old.verify_pin()?;
    let prepared = Helper::candidate(data, candidate)?;
    ensure!(
        old.binding.sha256 != prepared.binding.sha256,
        "候选就是当前助手，无需迁移或重新授权"
    );
    let receipt = receipt_bytes(data)?;
    if let Some(raw) = &receipt {
        let previous = parse_receipt(raw)?;
        ensure!(
            digest(&old_raw) == digest(&serde_json::to_vec(&previous.next)?),
            "存在未完成的助手事务；先使用 keychain recover-preview / recover"
        );
    }
    let next = Binding {
        version: 2,
        sha256: prepared.binding.sha256.clone(),
    };
    finish_plan(json!({
        "version":1,"action":"migrate","dataDirectory":data,"helperIdentifier":ID,
        "previousBindingSha256":digest(&old_raw),"previousHelperSha256":old.binding.sha256,
        "previousSignerSha256":digest(&crate::keychain_signature::certificate(&old.path, ID)?),
        "helperSha256":next.sha256,"candidateSignerSha256":digest(&crate::keychain_signature::certificate(candidate, ID)?),
        "installedPath":binding_path(data, &next),
        "previousRecoverySha256":receipt.as_ref().map(|v| digest(v)),
        "items":credential_items(data, operations)?,"credentialValuesRead":false,"credentialsMoved":false,
        "reload":"Reload App/backend/Agent MCP after activation; signer rotation also requires matching main binaries."
    }))
}

pub fn migrate(
    data: &Path,
    candidate: &Path,
    reviewed: &str,
    operations: &[String],
) -> Result<Value> {
    let _lock = lock(data)?;
    let plan = migration_preview(data, candidate, operations)?;
    check_plan(&plan, reviewed)?;
    let old_raw = binding_bytes(data)?.context("缺少原助手绑定")?;
    let mut prepared = Helper::candidate(data, candidate)?;
    prepared.binding.version = 2;
    let versions = directory(data).join("versions");
    private_directory(&versions)?;
    private_directory(&versions.join(&prepared.binding.sha256))?;
    let installed = install_atomic(&prepared, &binding_path(data, &prepared.binding))?;
    let authorized = check_grants(&installed, &plan, !vault::interaction_disabled())?;
    check_plan(&migration_preview(data, candidate, operations)?, reviewed)?;
    let receipt = Receipt {
        version: 1,
        previous_json: String::from_utf8(old_raw)?,
        next: installed.binding.clone(),
        preview_hash: reviewed.into(),
    };
    // Persist recovery before the only commit point. Both old and new helper files remain intact.
    write_metadata(
        &directory(data).join("rollback.json"),
        &serde_json::to_vec(&receipt)?,
    )?;
    write_metadata(
        &directory(data).join("active.json"),
        &serde_json::to_vec(&installed.binding)?,
    )?;
    Ok(
        json!({"enabled":true,"bindingVersion":2,"helperSha256":installed.binding.sha256,
              "items":authorized,"credentialsMoved":false,"previousHelperPreserved":true,
              "recovery":"keychain recover-preview / recover; reload the matching App/backend/Agent MCP"}),
    )
}

fn staging_files(data: &Path) -> Result<Vec<Value>> {
    let dir = directory(data);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut parents = vec![dir.clone()];
    let versions = dir.join("versions");
    match fs::symlink_metadata(&versions) {
        Ok(meta) => {
            ensure!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "助手版本目录无效"
            );
            for entry in fs::read_dir(versions)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.len() == 64 && name.bytes().all(|c| c.is_ascii_hexdigit()) {
                    ensure!(
                        entry.file_type()?.is_dir() && !entry.file_type()?.is_symlink(),
                        "助手版本目录无效"
                    );
                    parents.push(entry.path());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut files = Vec::new();
    for parent in parents {
        for entry in fs::read_dir(parent)? {
            let path = entry?.path();
            let name = path.file_name().context("暂存文件无效")?.to_string_lossy();
            if let Some(id) = name
                .strip_prefix(".install-")
                .and_then(|n| n.strip_suffix(".tmp"))
                && uuid::Uuid::parse_str(id).is_ok()
            {
                let raw = read_private_file(&path, 128 * 1024 * 1024)?.context("暂存文件已变化")?;
                files.push(json!({"path":path,"sha256":digest(&raw)}));
            }
        }
    }
    files.sort_by_key(|v| v["path"].as_str().unwrap_or_default().to_owned());
    Ok(files)
}

pub fn recovery_preview(data: &Path) -> Result<Value> {
    ensure!(data.is_absolute(), "恢复需要原有绝对数据目录");
    let active = binding_bytes(data)?;
    let receipt_raw = receipt_bytes(data)?;
    let mut action = "staging";
    let mut destination = None;
    if let Some(raw) = &receipt_raw {
        let receipt = parse_receipt(raw)?;
        let current = active
            .as_ref()
            .context("恢复记录存在但当前绑定缺失；不覆盖未知状态")?;
        let after = serde_json::to_vec(&receipt.next)?;
        if digest(current) == digest(&after) {
            action = "rollback";
        } else {
            ensure!(
                digest(current) == digest(receipt.previous_json.as_bytes()),
                "助手绑定已外部改变；不回滚未知状态"
            );
            action = "abort-pending";
        }
        let previous = parse_binding(receipt.previous_json.as_bytes())?;
        let helper = Helper {
            path: binding_path(data, &previous),
            binding: previous.clone(),
            data: data.into(),
        };
        helper.verify_pin()?;
        destination = Some(previous);
    }
    let unbound = if active.is_none() {
        let path = directory(data).join("mcplink-keychain");
        read_private_file(&path, 128 * 1024 * 1024)?
            .map(|raw| json!({"path":path,"sha256":digest(&raw)}))
    } else {
        None
    };
    if unbound.is_some() && receipt_raw.is_none() {
        action = "quarantine-unbound";
    }
    finish_plan(json!({
        "version":1,"action":action,"dataDirectory":data,"activeBindingSha256":active.as_ref().map(|v| digest(v)),
        "recoveryReceiptSha256":receipt_raw.as_ref().map(|v| digest(v)),
        "restoreBinding":destination,"unboundHelper":unbound,"stagingFiles":staging_files(data)?,
        "credentialValuesRead":false,"credentialsMoved":false,
        "recoveryPolicy":"Only the reviewed binding/receipt and unactivated files are changed; helper bytes and Keychain grants are retained."
    }))
}

pub fn recover(data: &Path, reviewed: &str) -> Result<Value> {
    let _lock = lock(data)?;
    let plan = recovery_preview(data)?;
    check_plan(&plan, reviewed)?;
    if matches!(plan["action"].as_str(), Some("rollback" | "abort-pending")) {
        let receipt = parse_receipt(&receipt_bytes(data)?.context("恢复记录已变化")?)?;
        if plan["action"] == "rollback" {
            write_metadata(
                &directory(data).join("active.json"),
                receipt.previous_json.as_bytes(),
            )?;
        }
        fs::remove_file(directory(data).join("rollback.json"))?;
        File::open(directory(data))?.sync_all()?;
    }
    let mut files = plan["stagingFiles"]
        .as_array()
        .context("暂存清单无效")?
        .clone();
    if !plan["unboundHelper"].is_null() {
        files.push(plan["unboundHelper"].clone());
    }
    let mut moved = 0;
    if !files.is_empty() {
        let recovered = directory(data).join("recovered");
        private_directory(&recovered)?;
        for file in files {
            let path = Path::new(file["path"].as_str().context("恢复文件路径无效")?);
            let raw = read_private_file(path, 128 * 1024 * 1024)?.context("恢复文件已变化")?;
            ensure!(file["sha256"] == digest(&raw), "恢复文件已变化；未覆盖");
            fs::rename(
                path,
                recovered.join(format!("{}-helper", uuid::Uuid::new_v4())),
            )?;
            moved += 1;
        }
        File::open(recovered)?.sync_all()?;
    }
    Ok(
        json!({"recovered":true,"action":plan["action"],"quarantinedFiles":moved,
              "credentialValuesRead":false,"credentialsMoved":false,
              "reload":"Reload App/backend/Agent MCP when a previous binding was restored."}),
    )
}
