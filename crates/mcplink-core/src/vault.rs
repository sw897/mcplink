use crate::model::digest;
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Vault {
    namespace: String,
    fixture: Option<PathBuf>,
}

pub fn private_dir(path: &Path) -> Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(path).context("无法创建本地数据目录")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    atomic_write(path, data, false)
}

pub fn write_config(path: &Path, data: &[u8]) -> Result<()> {
    atomic_write(path, data, true)
}

fn atomic_write(path: &Path, data: &[u8], preserve: bool) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("无效的文件位置")?;
    private_dir(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).context("无法写入本地文件")?;
    let result = (|| -> Result<()> {
        file.write_all(data)?;
        let existing = if preserve {
            match fs::symlink_metadata(path) {
                Ok(meta) if meta.is_file() => true,
                Ok(_) => bail!("目标已变为符号链接或特殊文件，请重新选择配置"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(e.into()),
            }
        } else {
            false
        };
        #[cfg(unix)]
        if existing {
            preserve_permissions(path, &file)?;
        }
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        if existing {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
            let wide = |p: &Path| {
                p.as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            // ReplaceFile preserves the existing file's ACL, attributes and named streams.
            let success = unsafe {
                ReplaceFileW(
                    wide(path).as_ptr(),
                    wide(&temporary).as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            if success == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            return Ok(());
        }
        fs::rename(&temporary, path).context("无法原子更新本地文件")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(target_os = "macos")]
fn preserve_permissions(path: &Path, destination: &fs::File) -> Result<()> {
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn fcopyfile(src: i32, dst: i32, state: *mut std::ffi::c_void, flags: u32) -> i32;
    }
    let source = fs::File::open(path)?;
    // COPYFILE_METADATA from copyfile.h (ACL | STAT | XATTR). Both fds stay live.
    if unsafe {
        fcopyfile(
            source.as_raw_fd(),
            destination.as_raw_fd(),
            std::ptr::null_mut(),
            7,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error()).context("无法保留原配置文件权限");
    }
    destination.set_modified(std::time::SystemTime::now())?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn preserve_permissions(path: &Path, destination: &fs::File) -> Result<()> {
    Ok(destination.set_permissions(fs::metadata(path)?.permissions())?)
}

impl Vault {
    pub fn new(data: &Path) -> Result<Self> {
        let fixture = if std::env::var("MCPLINK_VAULT").ok().as_deref() == Some("fixture") {
            let root = std::env::var_os("MCPLINK_HOME")
                .map(PathBuf::from)
                .context("测试凭据库要求合成用户目录")?;
            if !root.join(".mcplink-test-fixture").is_file() {
                bail!("测试凭据库要求显式 fixture 标记");
            }
            Some(data.join("fixture-vault"))
        } else {
            None
        };
        Ok(Self {
            namespace: digest(data.to_string_lossy().as_bytes())[..20].into(),
            fixture,
        })
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(
            "io.mcplink.credentials",
            &format!("{}/{}", self.namespace, key),
        )
        .context("无法访问系统凭据库")
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        if let Some(root) = &self.fixture {
            let file = root.join(digest(key.as_bytes()));
            return match fs::read_to_string(file) {
                Ok(value) => Ok(Some(value)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(_) => bail!("无法读取测试凭据库"),
            };
        }
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!("系统凭据库不可用或访问未获允许"),
        }
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        if let Some(root) = &self.fixture {
            return private_write(&root.join(digest(key.as_bytes())), value.as_bytes());
        }
        self.entry(key)?
            .set_password(value)
            .context("无法保存到系统凭据库")
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        if let Some(root) = &self.fixture {
            let path = root.join(digest(key.as_bytes()));
            if path.exists() {
                fs::remove_file(path)?;
            }
            return Ok(());
        }
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("无法清理系统凭据库中的连接"),
        }
    }
}
