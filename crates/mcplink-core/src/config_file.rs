//! Native configuration transactions stay anchored to a checked directory.
use crate::{config, model::digest, vault::private_dir};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

pub(crate) const LIMIT: u64 = 4 * 1024 * 1024;

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "配置位置必须是普通目录和文件")
}

pub(crate) fn regular(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.is_file() && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    #[cfg(unix)]
    metadata.is_file()
}

pub(crate) fn read_text(file: &File) -> Result<String> {
    ensure!(regular(&file.metadata()?), "配置须为普通文件");
    let mut text = String::new();
    file.take(LIMIT + 1)
        .read_to_string(&mut text)
        .context("无法读取配置，请检查文件访问状态")?;
    ensure!(text.len() as u64 <= LIMIT, "配置超过 4 MiB，请检查所选文件");
    Ok(text)
}

pub(crate) fn lock(path: &Path) -> Result<File> {
    // The root is independent of MCPLINK_DATA_DIR and of the Agent's working tree.
    let root = lock_root()?;
    private_dir(&root)?;
    let identity = config::path_identity(path);
    let location = root.join(format!("{}.lock", digest(identity.as_bytes())));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    platform::private_lock_options(&mut options);
    let file = options.open(location).context("无法打开用户配置写锁")?;
    ensure!(regular(&file.metadata()?), "配置写锁位置无效");
    fs2::FileExt::try_lock_exclusive(&file).context("另一个配置操作正在进行，请稍后重试")?;
    Ok(file)
}

fn lock_root() -> Result<PathBuf> {
    #[cfg(feature = "test-fixtures")]
    if std::env::var_os("MCPLINK_VAULT").as_deref() == Some(std::ffi::OsStr::new("fixture")) {
        let home = config::user_home()?;
        if home.join(".mcplink-test-fixture").is_file() {
            return Ok(home.join(".mcplink/config-locks"));
        }
    }
    directories::ProjectDirs::from("io", "mcplink", "mcplink")
        .map(|dirs| dirs.data_local_dir().join("config-locks"))
        .context("无法定位用户配置写锁目录")
}

pub(crate) struct Target {
    path: PathBuf,
    parent: platform::Directory,
    name: OsString,
}

pub(crate) struct Current {
    pub text: String,
    file: Option<File>,
    identity: Option<[u64; 2]>,
}

impl Target {
    pub(crate) fn open(path: &Path, create: bool) -> io::Result<Self> {
        if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
            return Err(invalid());
        }
        Ok(Self {
            path: path.to_owned(),
            parent: platform::Directory::open(path.parent().ok_or_else(invalid)?, create)?,
            name: path.file_name().ok_or_else(invalid)?.to_owned(),
        })
    }

    pub(crate) fn read(&self) -> Result<Current> {
        let file = match self.parent.open_file(&self.name) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("无法安全读取目标配置"),
        };
        let text = file
            .as_ref()
            .map(read_text)
            .transpose()?
            .unwrap_or_default();
        let identity = file.as_ref().map(platform::identity).transpose()?;
        Ok(Current {
            text,
            file,
            identity,
        })
    }

    pub(crate) fn attached(&self) -> Result<()> {
        let live = Self::open(&self.path, false)?;
        ensure!(
            self.parent.identity()? == live.parent.identity()?,
            "配置目录已变化，请重新预览"
        );
        Ok(())
    }

    pub(crate) fn replace(
        &self,
        current: &Current,
        rendered: &[u8],
        arm: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        ensure!(rendered.len() as u64 <= LIMIT, "更新后的配置超过 4 MiB");
        let name = OsString::from(format!(".mcplink-{}.tmp", uuid::Uuid::new_v4()));
        let mut temp = self.parent.create(&name)?;
        let result = (|| {
            temp.write_all(rendered)?;
            if let Some(source) = &current.file {
                platform::preserve(source, &temp).context("无法保留原配置文件权限")?;
            }
            temp.sync_all()?;
            let temporary_identity = platform::identity(&temp)?;
            drop(temp);
            self.attached()?;
            let live = self.read()?;
            ensure!(
                current.identity == live.identity && current.text == live.text,
                "目标文件已被外部修改，请重新预览"
            );
            arm()?;
            self.parent
                .replace(&name, &self.name, current.file.is_some())?;
            #[cfg(feature = "test-fixtures")]
            if crate::vault::fixture_failpoint("config-sync") {
                anyhow::bail!("测试注入的配置替换后同步失败");
            }
            self.sync()?;
            let published = self.parent.open_file(&self.name)?;
            ensure!(
                platform::identity(&published)? == temporary_identity,
                "替换后的目标已变化，需要核对配置"
            );
            Ok(())
        })();
        if result.is_err() {
            let _ = self.parent.remove(&name);
        }
        result
    }

    pub(crate) fn sync(&self) -> Result<()> {
        self.parent.sync(&self.name).context("无法同步目标配置")?;
        self.attached()
    }
}
