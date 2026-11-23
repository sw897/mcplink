//! Open updater directories and files without following links on supported platforms.
use std::{
    ffi::OsStr,
    fs::File,
    io,
    path::{Component, Path},
};

#[cfg(target_os = "macos")]
use std::{
    ffi::CString,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
};
#[cfg(windows)]
use std::{
    fs::OpenOptions,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

pub(crate) struct Directory {
    #[cfg(target_os = "macos")]
    file: File,
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    _ancestors: Vec<File>,
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "unsafe updater path")
}

#[cfg(target_os = "macos")]
fn filename(name: &OsStr) -> io::Result<CString> {
    if !matches!(
        Path::new(name).components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(_)]
    ) {
        return Err(invalid());
    }
    CString::new(name.as_bytes()).map_err(|_| invalid())
}

#[cfg(target_os = "macos")]
fn open_at(directory: &File, name: &OsStr, flags: libc::c_int) -> io::Result<File> {
    let name = filename(name)?;
    // SAFETY: both arguments live until openat returns. Success transfers one fd.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

impl Directory {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        Self::walk(path, false)
    }

    pub(crate) fn ensure(path: &Path) -> io::Result<Self> {
        Self::walk(path, true)
    }

    fn walk(path: &Path, create: bool) -> io::Result<Self> {
        if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
            return Err(invalid());
        }
        #[cfg(target_os = "macos")]
        {
            let mut file = File::open("/")?;
            for part in path.components() {
                if let Component::Normal(name) = part {
                    let open = || open_at(&file, name, libc::O_RDONLY | libc::O_DIRECTORY);
                    file = match open() {
                        Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                            let name = filename(name)?;
                            // SAFETY: one component relative to the held directory descriptor.
                            if unsafe { libc::mkdirat(file.as_raw_fd(), name.as_ptr(), 0o700) } != 0
                            {
                                let error = io::Error::last_os_error();
                                if error.kind() != io::ErrorKind::AlreadyExists {
                                    return Err(error);
                                }
                            }
                            open()?
                        }
                        other => other?,
                    };
                }
            }
            Ok(Self { file })
        }
        #[cfg(windows)]
        {
            let mut ancestors = Vec::new();
            for directory in path
                .ancestors()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .filter(|directory| directory.is_absolute())
            {
                // Directory-list access makes the handle participate in delete sharing checks.
                let open = || {
                    OpenOptions::new()
                        .access_mode(FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY)
                        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                        .open(directory)
                };
                let file = match open() {
                    Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                        if let Err(error) = std::fs::create_dir(directory)
                            && error.kind() != io::ErrorKind::AlreadyExists
                        {
                            return Err(error);
                        }
                        open()?
                    }
                    other => other?,
                };
                let metadata = file.metadata()?;
                if !metadata.is_dir()
                    || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                {
                    return Err(invalid());
                }
                ancestors.push(file);
            }
            Ok(Self {
                path: path.to_owned(),
                _ancestors: ancestors,
            })
        }
    }

    pub(crate) fn open_file(&self, name: &OsStr) -> io::Result<File> {
        #[cfg(target_os = "macos")]
        let file = open_at(&self.file, name, libc::O_RDONLY)?;
        #[cfg(windows)]
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(name))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(invalid());
        }
        #[cfg(windows)]
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(invalid());
        }
        Ok(file)
    }

    pub(crate) fn create(&self, name: &OsStr) -> io::Result<File> {
        #[cfg(target_os = "macos")]
        return open_at(
            &self.file,
            name,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        );
        #[cfg(windows)]
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.path.join(name))
    }

    pub(crate) fn rename(&self, from: &OsStr, to: &OsStr, replace: bool) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        let succeeded = {
            let from = filename(from)?;
            let to = filename(to)?;
            let fd = self.file.as_raw_fd();
            let flags = if replace { 0 } else { libc::RENAME_EXCL };
            // SAFETY: both names are single components and use the same live directory fd.
            unsafe { libc::renameatx_np(fd, from.as_ptr(), fd, to.as_ptr(), flags) == 0 }
        };
        #[cfg(windows)]
        let succeeded = {
            let wide = |name| {
                self.path
                    .join(name)
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            let from = wide(from);
            let to = wide(to);
            let flags = MOVEFILE_WRITE_THROUGH
                | if replace {
                    MOVEFILE_REPLACE_EXISTING
                } else {
                    0
                };
            // SAFETY: both wide strings are terminated and their parent is held open.
            unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) != 0 }
        };
        if succeeded {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub(crate) fn remove(&self, name: &OsStr) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        {
            let name = filename(name)?;
            // SAFETY: one path component relative to the live directory fd, without dereferencing.
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(windows)]
        std::fs::remove_file(self.path.join(name))
    }
}

pub(crate) fn open(path: &Path) -> io::Result<(Directory, &OsStr)> {
    Ok((
        Directory::open(path.parent().ok_or_else(invalid)?)?,
        path.file_name().ok_or_else(invalid)?,
    ))
}
