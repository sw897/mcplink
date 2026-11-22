use super::invalid;
use std::{
    ffi::{CString, OsStr},
    fs::{File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path},
};

pub(super) struct Directory(File);

fn filename(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|_| invalid())
}

fn open_at(parent: &File, name: &CString, flags: libc::c_int) -> io::Result<File> {
    // SAFETY: parent stays open and name is a single, terminated component.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
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

fn checked(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

impl Directory {
    pub(super) fn open(path: &Path, create: bool) -> io::Result<Self> {
        let mut current = File::open("/")?;
        for part in path.components() {
            let Component::Normal(name) = part else {
                continue;
            };
            let name = filename(name)?;
            let open = || open_at(&current, &name, libc::O_RDONLY | libc::O_DIRECTORY);
            current = match open() {
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    // SAFETY: the new directory is a single component under current.
                    let created =
                        unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) };
                    if let Err(error) = checked(created)
                        && error.kind() != io::ErrorKind::AlreadyExists
                    {
                        return Err(error);
                    }
                    current.sync_all()?;
                    open()?
                }
                result => result?,
            };
        }
        Ok(Self(current))
    }

    pub(super) fn identity(&self) -> io::Result<[u64; 2]> {
        identity(&self.0)
    }

    pub(super) fn open_file(&self, name: &OsStr) -> io::Result<File> {
        let file = open_at(&self.0, &filename(name)?, libc::O_RDONLY)?;
        if !file.metadata()?.is_file() {
            return Err(invalid());
        }
        Ok(file)
    }

    pub(super) fn create(&self, name: &OsStr) -> io::Result<File> {
        open_at(
            &self.0,
            &filename(name)?,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )
    }

    pub(super) fn replace(&self, from: &OsStr, to: &OsStr, replace: bool) -> io::Result<()> {
        let fd = self.0.as_raw_fd();
        let to = filename(to)?;
        let temporary = filename(from)?;
        // SAFETY: both names are relative to the held directory. Neither operation follows the destination leaf.
        if replace {
            checked(unsafe { libc::renameat(fd, temporary.as_ptr(), fd, to.as_ptr()) })
        } else {
            checked(unsafe { libc::linkat(fd, temporary.as_ptr(), fd, to.as_ptr(), 0) })?;
            self.remove(from)
        }
    }

    pub(super) fn remove(&self, name: &OsStr) -> io::Result<()> {
        let name = filename(name)?;
        // SAFETY: unlinkat uses the pinned parent and cannot dereference the leaf.
        checked(unsafe { libc::unlinkat(self.0.as_raw_fd(), name.as_ptr(), 0) })
    }

    pub(super) fn sync(&self, name: &OsStr) -> io::Result<()> {
        match self.open_file(name) {
            Ok(file) => file.sync_all()?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.0.sync_all()
    }
}

pub(super) fn identity(file: &File) -> io::Result<[u64; 2]> {
    let meta = file.metadata()?;
    Ok([meta.dev(), meta.ino()])
}

pub(super) fn private_lock_options(options: &mut OpenOptions) {
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
}

#[cfg(target_os = "macos")]
pub(super) fn preserve(source: &File, destination: &File) -> io::Result<()> {
    // COPYFILE_METADATA is ACL | STAT | XATTR. Both file descriptors are live.
    checked(unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            destination.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_METADATA,
        )
    })?;
    destination.set_modified(std::time::SystemTime::now())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn preserve(source: &File, destination: &File) -> io::Result<()> {
    let original = source.metadata()?;
    let created = destination.metadata()?;
    if (original.uid(), original.gid()) != (created.uid(), created.gid()) {
        // SAFETY: restrict the ownership update to the exclusively created temporary file.
        checked(unsafe { libc::fchown(destination.as_raw_fd(), original.uid(), original.gid()) })?;
    }
    destination.set_permissions(original.permissions())
}
