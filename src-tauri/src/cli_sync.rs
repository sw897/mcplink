use crate::update_files::{self, Directory};
use semver::Version;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const BINARY_LIMIT: u64 = 256 * 1024 * 1024;
const RECEIPT_LIMIT: u64 = 64 * 1024;
const MAX_TARGETS: usize = 32;
const BINARY: &str = if cfg!(windows) {
    "mcplink.exe"
} else {
    "mcplink"
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CliTarget {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: Version,
    pub targets: Vec<CliTarget>,
}

fn target_directory<'a>(path: &'a Path, home: &Path) -> Result<(Directory, &'a OsStr), String> {
    if !home.is_absolute() || !path.starts_with(home) {
        return Err("独立 CLI 不在用户目录内".into());
    }
    update_files::open(path).map_err(|_| "独立 CLI 位置已变化，已保留原文件".into())
}

fn hash_open(file: &mut File) -> io::Result<String> {
    let mut hash = Sha256::new();
    if io::copy(&mut file.take(BINARY_LIMIT + 1), &mut hash)? > BINARY_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CLI is too large",
        ));
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn hash_at(directory: &Directory, name: &OsStr) -> Result<Option<String>, String> {
    match directory.open_file(name) {
        Ok(mut file) => hash_open(&mut file)
            .map(Some)
            .map_err(|_| "独立 CLI 文件不符合升级条件".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("独立 CLI 位置已变化，已保留原文件".into()),
    }
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let (directory, name) = update_files::open(path).map_err(|_| "无法读取独立 CLI 文件")?;
    hash_at(&directory, name)?.ok_or_else(|| "无法读取独立 CLI 文件".into())
}

/// Recognize existing bytes from the bundled build. Discovery never launches a candidate.
pub fn discover(home: &Path, sidecar: &Path) -> Vec<CliTarget> {
    let Ok(trusted_hash) = hash_file(sidecar) else {
        return Vec::new();
    };
    let mut candidates = BTreeSet::from([
        home.join(".cargo/bin").join(BINARY),
        home.join(".local/bin").join(BINARY),
    ]);
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&paths)
                .filter(|entry| entry.is_absolute() && entry.starts_with(home))
                .map(|entry| entry.join(BINARY)),
        );
    }
    let sidecar = sidecar.canonicalize().ok();
    candidates
        .into_iter()
        .filter_map(|path| {
            let (directory, name) = target_directory(&path, home).ok()?;
            if path.canonicalize().ok() == sidecar
                || hash_at(&directory, name).ok().flatten().as_ref() != Some(&trusted_hash)
            {
                return None;
            }
            Some(CliTarget {
                path,
                sha256: trusted_hash.clone(),
            })
        })
        .take(MAX_TARGETS)
        .collect()
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_target(target: &CliTarget) -> bool {
    target.path.is_absolute()
        && target.path.file_name() == Some(OsStr::new(BINARY))
        && is_hash(&target.sha256)
}

pub fn validate_targets(targets: &[CliTarget], home: &Path) -> Result<(), String> {
    if targets.len() > MAX_TARGETS || !targets.iter().all(valid_target) {
        return Err("升级恢复记录的 CLI 路径或校验值无效".into());
    }
    for target in targets {
        let (directory, name) = target_directory(&target.path, home)?;
        if hash_at(&directory, name)?.as_ref() != Some(&target.sha256) {
            return Err(format!(
                "独立 CLI 已变化，请重新下载后确认：{}",
                target.path.display()
            ));
        }
    }
    Ok(())
}

fn private_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let directory = Directory::ensure(path.parent().ok_or("无法定位升级状态目录")?)
        .map_err(|_| "无法保护升级状态目录")?;
    let name = path.file_name().ok_or("无法定位升级状态文件")?;
    let temporary = format!(".mcplink-state-{}", uuid::Uuid::new_v4());
    let temporary = OsStr::new(&temporary);
    let bytes = serde_json::to_vec(value).map_err(|_| "升级状态格式无效")?;
    if bytes.len() as u64 > RECEIPT_LIMIT {
        return Err("升级状态过大".into());
    }
    let mut file = directory
        .create(temporary)
        .map_err(|_| "无法创建升级状态文件")?;
    let written = file.write_all(&bytes).and_then(|_| file.sync_all());
    drop(file);
    let saved = written.and_then(|_| directory.rename(temporary, name, true));
    if saved.is_err() {
        let _ = directory.remove(temporary);
        return Err("无法原子保存升级状态".into());
    }
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

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let file = update_files::open(path).and_then(|(directory, name)| directory.open_file(name));
    let file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("升级恢复记录必须是普通文件".into()),
    };
    let mut bytes = Vec::new();
    file.take(RECEIPT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取升级恢复记录")?;
    if bytes.len() as u64 > RECEIPT_LIMIT {
        return Err("升级恢复记录过大".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "升级恢复记录损坏，请检查独立 CLI".into())
}

pub fn read_receipt(path: &Path) -> Result<Option<Receipt>, String> {
    let value: Option<Receipt> = read_json(path)?;
    if value.as_ref().is_some_and(|receipt| {
        receipt.targets.len() > MAX_TARGETS || !receipt.targets.iter().all(valid_target)
    }) {
        return Err("升级恢复记录的 CLI 路径或校验值无效".into());
    }
    Ok(value)
}

/// Keep an abandoned receipt available for inspection without retrying its CLI changes.
pub fn archive_stale_receipt(path: &Path, current: &Version) -> Result<(), String> {
    if read_receipt(path)?.is_none_or(|receipt| receipt.version == *current) {
        return Err("没有需要清除的旧版升级记录".into());
    }
    let (directory, name) = update_files::open(path).map_err(|_| "无法定位升级状态文件")?;
    let archive = format!("pending-abandoned-{}.json", uuid::Uuid::new_v4());
    directory
        .rename(name, OsStr::new(&archive), false)
        .map_err(|_| "无法归档旧版升级记录".into())
}

fn stage_cli(directory: &Directory, stage: &OsStr, sidecar: &mut File) -> io::Result<()> {
    let mut file = directory.create(stage)?;
    let result = (|| {
        sidecar.seek(SeekFrom::Start(0))?;
        if io::copy(&mut sidecar.take(BINARY_LIMIT + 1), &mut file)? > BINARY_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "CLI is too large",
            ));
        }
        file.set_permissions(sidecar.metadata()?.permissions())?;
        file.sync_all()
    })();
    drop(file);
    if result.is_err() {
        let _ = directory.remove(stage);
    }
    result
}

/// Idempotent first-start synchronization. Each rename stays in the validated directory.
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
    let (directory, name) = update_files::open(sidecar).map_err(|_| "无法定位应用 CLI")?;
    let mut sidecar = directory.open_file(name).map_err(|_| "无法读取应用 CLI")?;
    let new_hash = hash_open(&mut sidecar).map_err(|_| "应用 CLI 文件不符合升级条件")?;
    let mut backups = Vec::new();
    for target in receipt.targets {
        let (directory, name) = target_directory(&target.path, home)?;
        let stage_name = format!(".{BINARY}.mcplink-update-{}", receipt.version);
        let backup_name = format!(".{BINARY}.mcplink-backup-{}", &target.sha256[..16]);
        let stage = OsStr::new(&stage_name);
        let backup = OsStr::new(&backup_name);
        let existing_backup = hash_at(&directory, backup)?;
        let mut existing = hash_at(&directory, name)?;
        if existing.is_none() && existing_backup.as_ref() == Some(&target.sha256) {
            directory
                .rename(backup, name, false)
                .map_err(|_| "未能恢复上次中断的独立 CLI，请检查文件权限")?;
            existing = hash_at(&directory, name)?;
        }
        if existing.as_ref() != Some(&new_hash) {
            if existing.as_ref() != Some(&target.sha256) {
                return Err("独立 CLI 已被修改，已保留原文件；请手动同步或重试".into());
            }
            if let Some(hash) = hash_at(&directory, stage)? {
                if hash != new_hash {
                    return Err("CLI 临时文件未通过校验，已保留原程序".into());
                }
                directory
                    .remove(stage)
                    .map_err(|_| "无法清理已校验的 CLI 临时文件")?;
            }
            if hash_at(&directory, backup)?.is_some() {
                return Err("CLI 回滚文件仍在使用，请检查后重试".into());
            }
            stage_cli(&directory, stage, &mut sidecar)
                .map_err(|_| "独立 CLI 复制失败，原文件已保留")?;
            if hash_at(&directory, stage)?.as_ref() != Some(&new_hash) {
                let _ = directory.remove(stage);
                return Err("独立 CLI 复制或校验失败，原文件已保留".into());
            }
            directory
                .rename(name, backup, false)
                .map_err(|_| "独立 CLI 正被使用或没有写入权限，请停止相关 Agent 后重试")?;
            let backed_up =
                hash_at(&directory, backup).is_ok_and(|hash| hash.as_ref() == Some(&target.sha256));
            if !backed_up || directory.rename(stage, name, false).is_err() {
                let _ = directory.rename(backup, name, false);
                return Err("独立 CLI 替换失败，请核对原文件与回滚文件后重试".into());
            }
        }
        if hash_at(&directory, backup).ok().flatten().as_ref() == Some(&target.sha256)
            && directory.remove(backup).is_err()
        {
            backups.push(CliTarget {
                path: target.path.with_file_name(backup),
                sha256: target.sha256,
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
    let Ok(Some(targets)) = read_json::<Vec<CliTarget>>(&path) else {
        return;
    };
    if targets.len() > MAX_TARGETS {
        return;
    }
    let remaining: Vec<_> = targets
        .into_iter()
        .filter(|target| {
            if !is_hash(&target.sha256) {
                return false;
            }
            let expected = format!(".{BINARY}.mcplink-backup-{}", &target.sha256[..16]);
            if target.path.file_name() != Some(OsStr::new(&expected)) {
                return false;
            }
            let Ok((directory, name)) = target_directory(&target.path, home) else {
                return false;
            };
            hash_at(&directory, name).ok().flatten().as_ref() == Some(&target.sha256)
                && directory.remove(name).is_err()
        })
        .collect();
    if remaining.is_empty() {
        if let Ok((directory, name)) = update_files::open(&path) {
            let _ = directory.remove(name);
        }
    } else {
        let _ = private_json(&path, &remaining);
    }
}
