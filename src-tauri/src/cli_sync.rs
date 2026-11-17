use crate::update_signature::sha256;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CliTarget {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub version: Version,
    pub targets: Vec<CliTarget>,
}

fn regular_in_home(path: &Path, home: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        && path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .zip(home.canonicalize().ok())
            .is_some_and(|(p, h)| p.starts_with(h))
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "无法读取独立 CLI 文件")?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 256 * 1024 * 1024 {
        return Err("独立 CLI 文件不符合升级条件".into());
    }
    Ok(sha256(
        &fs::read(path).map_err(|_| "无法读取独立 CLI 文件")?,
    ))
}

/// Recognize the current CLI, then pin its bytes; installation never accepts a frontend path.
pub async fn discover(home: &Path, sidecar: &Path, current: &Version) -> Vec<CliTarget> {
    let binary = if cfg!(windows) {
        "mcplink.exe"
    } else {
        "mcplink"
    };
    let mut candidates = BTreeSet::from([
        home.join(".cargo/bin").join(binary),
        home.join(".local/bin").join(binary),
    ]);
    if let Some(path) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path) {
            if entry.is_absolute() && entry.starts_with(home) {
                candidates.insert(entry.join(binary));
            }
        }
    }
    let sidecar = sidecar.canonicalize().ok();
    let mut targets = Vec::new();
    for path in candidates {
        if !regular_in_home(&path, home) || path.canonicalize().ok() == sidecar {
            continue;
        }
        let Ok(before) = hash_file(&path) else {
            continue;
        };
        let mut command = tokio::process::Command::new(&path);
        command
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let Ok(mut child) = command.spawn() else {
            continue;
        };
        let probe = async {
            let mut output = Vec::new();
            child
                .stdout
                .take()?
                .take(4097)
                .read_to_end(&mut output)
                .await
                .ok()?;
            let status = child.wait().await.ok()?;
            Some((status, output))
        };
        let Ok(Some((status, output))) = tokio::time::timeout(Duration::from_secs(3), probe).await
        else {
            continue;
        };
        let version = std::str::from_utf8(&output)
            .ok()
            .and_then(|s| s.trim().strip_prefix("mcplink "))
            .and_then(|s| Version::parse(s).ok());
        if status.success()
            && version.as_ref() == Some(current)
            && hash_file(&path).ok().as_ref() == Some(&before)
        {
            targets.push(CliTarget {
                path,
                sha256: before,
            });
        }
    }
    targets
}

pub fn validate_targets(targets: &[CliTarget], home: &Path) -> Result<(), String> {
    for target in targets {
        if !regular_in_home(&target.path, home) || hash_file(&target.path)? != target.sha256 {
            return Err(format!(
                "独立 CLI 已变化，请重新下载后确认：{}",
                target.path.display()
            ));
        }
    }
    Ok(())
}

pub fn private_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("无法定位升级状态目录")?;
    fs::create_dir_all(parent).map_err(|_| "无法创建升级状态目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法保护升级状态目录")?;
    }
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法写入升级状态，请稍后重试")?;
    let bytes = serde_json::to_vec(value).map_err(|_| "升级状态格式无效")?;
    if file
        .write_all(&bytes)
        .and_then(|_| file.as_file().sync_all())
        .is_err()
    {
        return Err("保存升级状态失败".into());
    }
    file.persist(path).map_err(|_| "无法原子保存升级状态")?;
    Ok(())
}

pub fn write_receipt(
    path: &Path,
    version: Version,
    targets: Vec<CliTarget>,
    home: &Path,
) -> Result<(), String> {
    validate_targets(&targets, home)?;
    private_json(path, &Receipt { version, targets })
}

pub fn read_receipt(path: &Path) -> Result<Option<Receipt>, String> {
    if fs::symlink_metadata(path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err("升级恢复记录必须是普通文件".into());
    }
    match fs::read(path) {
        Ok(bytes) if bytes.len() <= 64 * 1024 => {
            let receipt: Receipt =
                serde_json::from_slice(&bytes).map_err(|_| "升级恢复记录损坏，请检查独立 CLI")?;
            if receipt.targets.len() > 32
                || receipt.targets.iter().any(|t| {
                    !t.path.is_absolute()
                        || t.path.file_name().and_then(|n| n.to_str())
                            != Some(if cfg!(windows) {
                                "mcplink.exe"
                            } else {
                                "mcplink"
                            })
                        || t.sha256.len() != 64
                        || !t.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                })
            {
                return Err("升级恢复记录的 CLI 路径或校验值无效".into());
            }
            Ok(Some(receipt))
        }
        Ok(_) => Err("升级恢复记录过大".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("无法读取升级恢复记录".into()),
    }
}

/// Idempotent first-start synchronization. Unknown or edited files are always preserved.
pub fn apply_receipt(
    path: &Path,
    home: &Path,
    sidecar: &Path,
    current: &Version,
) -> Result<bool, String> {
    cleanup_backups(path, home);
    let Some(receipt) = read_receipt(path)? else {
        return Ok(false);
    };
    if &receipt.version != current {
        return Err("上次升级尚未完成，独立 CLI 保持原样；请重新检查更新".into());
    }
    let new_hash = hash_file(sidecar)?;
    let mut backups = Vec::new();
    for target in &receipt.targets {
        let name = target
            .path
            .file_name()
            .ok_or("独立 CLI 路径无效")?
            .to_string_lossy();
        let stage = target
            .path
            .with_file_name(format!(".{name}.mcplink-update-{}", receipt.version));
        let backup = target
            .path
            .with_file_name(format!(".{name}.mcplink-backup-{}", &target.sha256[..16]));
        if fs::symlink_metadata(&target.path)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            && regular_in_home(&backup, home)
            && hash_file(&backup)? == target.sha256
        {
            fs::rename(&backup, &target.path)
                .map_err(|_| "未能恢复上次中断的独立 CLI，请检查文件权限")?;
        }
        if !regular_in_home(&target.path, home) {
            return Err("独立 CLI 位置已变化，已保留原文件".into());
        }
        let old_hash = hash_file(&target.path)?;
        if old_hash == new_hash {
            if regular_in_home(&backup, home)
                && hash_file(&backup).ok().as_ref() == Some(&target.sha256)
                && fs::remove_file(&backup).is_err()
            {
                backups.push(CliTarget {
                    path: backup,
                    sha256: target.sha256.clone(),
                });
            }
            continue;
        }
        if old_hash != target.sha256 {
            return Err("独立 CLI 已被修改，已保留原文件；请手动同步或重试".into());
        }
        if fs::symlink_metadata(&stage).is_ok() {
            if regular_in_home(&stage, home) && hash_file(&stage)? == new_hash {
                fs::remove_file(&stage).map_err(|_| "无法清理已校验的 CLI 临时文件")?;
            } else {
                return Err(format!(
                    "CLI 临时文件未通过校验，已保留原程序。请检查并移除 {} 后重试",
                    stage.display()
                ));
            }
        }
        if fs::symlink_metadata(&backup).is_ok() {
            return Err(format!(
                "CLI 回滚文件仍在使用，请检查 {} 后重试",
                backup.display()
            ));
        }
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        let mut file = options
            .open(&stage)
            .map_err(|_| "无法创建独立 CLI 升级文件")?;
        let copied = fs::File::open(sidecar)
            .and_then(|mut input| std::io::copy(&mut input, &mut file))
            .and_then(|_| file.sync_all());
        drop(file);
        if copied.is_err()
            || fs::set_permissions(
                &stage,
                fs::metadata(sidecar)
                    .map_err(|_| "无法读取新 CLI 权限")?
                    .permissions(),
            )
            .is_err()
        {
            let _ = fs::remove_file(&stage);
            return Err("独立 CLI 复制失败，原文件已保留".into());
        }
        if hash_file(&stage)? != new_hash || hash_file(&target.path)? != old_hash {
            let _ = fs::remove_file(&stage);
            return Err("独立 CLI 在升级时变化，未替换原文件".into());
        }
        if fs::rename(&target.path, &backup).is_err() {
            let _ = fs::remove_file(&stage);
            return Err("独立 CLI 正被使用或没有写入权限，请停止相关 Agent 后重试".into());
        }
        if fs::rename(&stage, &target.path).is_err() {
            let restored = fs::rename(&backup, &target.path).is_ok();
            let _ = fs::remove_file(&stage);
            return Err(if restored {
                "独立 CLI 替换失败，已恢复原文件".into()
            } else {
                format!(
                    "独立 CLI 替换失败，原程序保存在 {}；请恢复后重试",
                    backup.display()
                )
            });
        }
        if fs::remove_file(&backup).is_err() {
            backups.push(CliTarget {
                path: backup,
                sha256: old_hash,
            });
        }
    }
    if !backups.is_empty() {
        private_json(&path.with_file_name("cli-backups.json"), &backups)?;
    }
    Ok(true)
}

fn cleanup_backups(receipt: &Path, home: &Path) {
    let path = receipt.with_file_name("cli-backups.json");
    let Ok(bytes) = fs::read(&path) else {
        return;
    };
    if bytes.len() > 64 * 1024 {
        return;
    }
    let Ok(targets) = serde_json::from_slice::<Vec<CliTarget>>(&bytes) else {
        return;
    };
    let remaining: Vec<_> = targets
        .into_iter()
        .filter(|target| {
            target
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".mcplink") && n.contains(".mcplink-backup-"))
                && regular_in_home(&target.path, home)
                && hash_file(&target.path).ok().as_ref() == Some(&target.sha256)
                && fs::remove_file(&target.path).is_err()
        })
        .collect();
    if remaining.is_empty() {
        let _ = fs::remove_file(&path);
    } else {
        let _ = private_json(&path, &remaining);
    }
}
