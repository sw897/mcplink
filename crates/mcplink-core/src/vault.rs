use crate::model::digest;
#[cfg(any(not(windows), feature = "test-fixtures"))]
use anyhow::bail;
use anyhow::{Context, Result};
#[cfg(feature = "test-fixtures")]
use std::path::PathBuf;
use std::{fs, path::Path};

static INTERACTION_DISABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn interaction_disabled() -> bool {
    INTERACTION_DISABLED.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(feature = "test-fixtures")]
pub(crate) fn fixture_failpoint(point: &str) -> bool {
    std::env::var("MCPLINK_FAILPOINT").ok().as_deref() == Some(point)
        && std::env::var("MCPLINK_VAULT").ok().as_deref() == Some("fixture")
        && std::env::var_os("MCPLINK_HOME")
            .is_some_and(|home| PathBuf::from(home).join(".mcplink-test-fixture").is_file())
}

/// Refuse Keychain access from a process whose executable was replaced in place.
pub fn verify_running_code() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::c_void;
        #[link(name = "Security", kind = "framework")]
        unsafe extern "C" {
            fn SecCodeCopySelf(flags: u32, code: *mut *const c_void) -> i32;
            fn SecCodeCheckValidity(
                code: *const c_void,
                flags: u32,
                requirement: *const c_void,
            ) -> i32;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFRelease(value: *const c_void);
        }
        let mut code = std::ptr::null();
        if unsafe { SecCodeCopySelf(0, &mut code) } != 0 {
            bail!("无法验证当前程序身份，请使用已签名的 mcplink 构建");
        }
        let status = unsafe { SecCodeCheckValidity(code, 0, std::ptr::null()) };
        unsafe { CFRelease(code) };
        if status == -67034 {
            bail!("mcplink 程序已更新；请重新启动当前客户端或重载 MCP 连接后再访问凭据");
        }
        if status != 0 {
            bail!("当前程序签名校验失败，请使用已签名的 mcplink 构建");
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
mod windows;
#[cfg(windows)]
pub(crate) mod windows_protection;

const SERVICE: &str = "io.mcplink.credentials";

#[derive(Clone)]
pub struct Vault {
    namespace: String,
    #[cfg(feature = "test-fixtures")]
    fixture: Option<PathBuf>,
    #[cfg(target_os = "macos")]
    helper: Option<crate::keychain_helper::Helper>,
}

/// A diagnostic/headless process may fail rather than display system consent UI.
/// This never changes an item's permissions or authorizes access.
pub fn disable_interaction() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "Security", kind = "framework")]
        unsafe extern "C" {
            fn SecKeychainSetUserInteractionAllowed(allowed: u8) -> i32;
        }
        if unsafe { SecKeychainSetUserInteractionAllowed(0) } != 0 {
            bail!("无法关闭本进程的钥匙串授权弹窗");
        }
    }
    INTERACTION_DISABLED.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

pub fn private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
        fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(path)
            .context("无法创建本地数据目录")?;
        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
            .context("私有数据目录须为真实目录")?;
        let meta = directory.metadata()?;
        // SAFETY: geteuid does not access caller memory.
        anyhow::ensure!(
            meta.uid() == unsafe { libc::geteuid() },
            "私有数据目录须由当前用户所有"
        );
        #[cfg(target_os = "macos")]
        clear_extended_acl(&directory)?;
        if meta.mode() & 0o777 != 0o700 {
            directory.set_permissions(fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(windows)]
    {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        windows_protection::private_directory(path).context("无法保护本地数据目录")?;
    }
    Ok(())
}

pub fn private_write(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("无效的文件位置")?;
    private_dir(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?
    };
    #[cfg(windows)]
    let mut file = windows_protection::create_private_file(&temporary)?;
    let result = (|| -> Result<()> {
        #[cfg(target_os = "macos")]
        clear_extended_acl(&file)?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path).context("无法原子更新本地文件")?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub(crate) fn protect_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let meta = file.metadata()?;
        // SAFETY: geteuid returns the current effective user without reading memory.
        anyhow::ensure!(
            meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.nlink() == 1,
            "私有存储文件须由当前用户所有且只有一个文件名"
        );
        #[cfg(target_os = "macos")]
        clear_extended_acl(&file)?;
        if meta.mode() & 0o777 != 0o600 {
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
    }
    #[cfg(windows)]
    windows_protection::restrict_file(path)?;
    Ok(())
}

pub(crate) fn prepare_private_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    };
    #[cfg(windows)]
    let created = windows_protection::create_private_file(path);
    match created {
        Ok(_file) => {
            #[cfg(target_os = "macos")]
            clear_extended_acl(&_file)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            protect_private_file(path)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "macos")]
fn clear_extended_acl(file: &fs::File) -> Result<()> {
    use std::{ffi::c_void, os::fd::AsRawFd};
    unsafe extern "C" {
        fn acl_init(count: i32) -> *mut c_void;
        fn acl_set_fd_np(fd: i32, acl: *mut c_void, kind: i32) -> i32;
        fn acl_free(acl: *mut c_void) -> i32;
    }
    // SAFETY: the descriptor stays open, and the ACL is freed exactly once.
    let result = unsafe {
        let acl = acl_init(0);
        if acl.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let status = acl_set_fd_np(file.as_raw_fd(), acl, 0x100);
        let result = (status != 0).then(std::io::Error::last_os_error);
        acl_free(acl);
        result
    };
    if let Some(error) = result
        && error.raw_os_error() != Some(libc::ENOTSUP)
    {
        return Err(error).context("无法保护应用私有数据的访问权限");
    }
    Ok(())
}

impl Vault {
    pub fn new(data: &Path) -> Result<Self> {
        #[cfg(not(feature = "test-fixtures"))]
        anyhow::ensure!(
            std::env::var_os("MCPLINK_VAULT").as_deref() != Some(std::ffi::OsStr::new("fixture")),
            "此构建使用系统凭据库，测试凭据库不可用"
        );
        #[cfg(feature = "test-fixtures")]
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
            #[cfg(all(target_os = "macos", feature = "test-fixtures"))]
            helper: if fixture.is_some() {
                None
            } else {
                crate::keychain_helper::Helper::configured(data)?
            },
            #[cfg(all(target_os = "macos", not(feature = "test-fixtures")))]
            helper: crate::keychain_helper::Helper::configured(data)?,
            #[cfg(feature = "test-fixtures")]
            fixture,
        })
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        verify_running_code()?;
        keyring::Entry::new(SERVICE, &format!("{}/{}", self.namespace, key))
            .context("无法访问系统凭据库")
    }

    /// Enumerate only this application's old plan and transaction keys.
    /// The native query requests metadata and never returns a password.
    pub(crate) fn old_change_keys(&self) -> Result<std::collections::BTreeSet<String>> {
        let mut keys = std::collections::BTreeSet::new();
        #[cfg(feature = "test-fixtures")]
        if let Some(root) = &self.fixture {
            use std::io::Read;
            let entries = match fs::read_dir(root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(keys),
                Err(error) => return Err(error.into()),
            };
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let mut bytes = Vec::new();
                fs::File::open(entry.path())?
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    continue;
                };
                let Some(id) = value["id"].as_str().filter(|id| canonical_uuid(id)) else {
                    continue;
                };
                for kind in ["plan", "change"] {
                    let key = format!("{kind}/{id}");
                    if entry.file_name() == std::ffi::OsStr::new(&digest(key.as_bytes())) {
                        keys.insert(key);
                    }
                }
            }
            return Ok(keys);
        }
        verify_running_code()?;
        keyring::Entry::store_status()
            .as_ref()
            .map_err(|_| anyhow::anyhow!("系统凭据库不可用"))?;
        let prefix = format!("{}/", self.namespace);
        #[cfg(windows)]
        let pattern = format!(
            r"^{prefix}(plan|change)/[0-9a-f-]+(/chunks-v1/[ab]/index(/[0-9]+)?)?\.io\.mcplink\.credentials$"
        );
        #[cfg(windows)]
        let spec = [("pattern", pattern.as_str())];
        #[cfg(not(windows))]
        let spec = [("service", SERVICE)];
        for entry in keyring_core::Entry::search(&spec.into_iter().collect())? {
            let Some((service, account)) = entry.get_specifiers() else {
                continue;
            };
            if service != SERVICE {
                continue;
            }
            let Some(key) = account.strip_prefix(&prefix) else {
                continue;
            };
            let Some((kind, rest)) = key.split_once('/') else {
                continue;
            };
            let id = rest.split('/').next().unwrap_or_default();
            if matches!(kind, "plan" | "change") && canonical_uuid(id) {
                keys.insert(format!("{kind}/{id}"));
            }
        }
        Ok(keys)
    }

    pub fn get(&self, key: &str) -> Result<Option<String>> {
        #[cfg(target_os = "macos")]
        if let Some(helper) = &self.helper {
            return helper.get(key);
        }
        #[cfg(feature = "test-fixtures")]
        if let Some(root) = &self.fixture {
            let file = root.join(digest(key.as_bytes()));
            return match fs::read_to_string(file) {
                Ok(value) => Ok(Some(value)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(_) => bail!("无法读取测试凭据库"),
            };
        }
        #[cfg(windows)]
        {
            windows::get(self, key)
        }
        #[cfg(not(windows))]
        match self.entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!("系统凭据库不可用或访问未获允许"),
        }
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        if let Some(helper) = &self.helper {
            helper.request("set", key, Some(value))?;
            return Ok(());
        }
        #[cfg(feature = "test-fixtures")]
        if let Some(root) = &self.fixture {
            return private_write(&root.join(digest(key.as_bytes())), value.as_bytes());
        }
        #[cfg(windows)]
        {
            windows::set(self, key, value)
        }
        #[cfg(not(windows))]
        self.entry(key)?
            .set_password(value)
            .context("无法保存到系统凭据库")
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        if let Some(helper) = &self.helper {
            helper.request("delete", key, None)?;
            return Ok(());
        }
        #[cfg(feature = "test-fixtures")]
        if let Some(root) = &self.fixture {
            let path = root.join(digest(key.as_bytes()));
            if path.exists() {
                fs::remove_file(path)?;
            }
            return Ok(());
        }
        #[cfg(windows)]
        {
            windows::delete(self, key)
        }
        #[cfg(not(windows))]
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("无法清理系统凭据库中的连接"),
        }
    }
}

pub(crate) fn canonical_uuid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}
