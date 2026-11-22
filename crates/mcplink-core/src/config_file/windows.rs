use super::{invalid, regular};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::GENERIC_WRITE,
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_WRITE_THROUGH, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW, WRITE_DAC,
    },
};

pub(super) struct Directory {
    path: PathBuf,
    ancestors: Vec<File>,
}

const SHARE: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn open_file(path: &Path, write: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .share_mode(SHARE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if !regular(&file.metadata()?) {
        return Err(invalid());
    }
    Ok(file)
}

impl Directory {
    pub(super) fn open(path: &Path, create: bool) -> io::Result<Self> {
        let mut ancestors = Vec::new();
        for directory in path
            .ancestors()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .filter(|path| path.is_absolute())
        {
            // Keep every directory open without FILE_SHARE_DELETE until the transaction ends.
            let open = || {
                OpenOptions::new()
                    .access_mode(FILE_READ_ATTRIBUTES)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(directory)
            };
            let file = match open() {
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    if let Err(error) = fs::create_dir(directory)
                        && error.kind() != io::ErrorKind::AlreadyExists
                    {
                        return Err(error);
                    }
                    open()?
                }
                result => result?,
            };
            let metadata = file.metadata()?;
            if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(invalid());
            }
            ancestors.push(file);
        }
        Ok(Self {
            path: path.to_owned(),
            ancestors,
        })
    }

    pub(super) fn identity(&self) -> io::Result<[u64; 2]> {
        identity(self.ancestors.last().ok_or_else(invalid)?)
    }

    pub(super) fn open_file(&self, name: &OsStr) -> io::Result<File> {
        open_file(&self.path.join(name), false)
    }

    pub(super) fn create(&self, name: &OsStr) -> io::Result<File> {
        crate::vault::windows_protection::create_private_file_with_access(
            &self.path.join(name),
            GENERIC_WRITE | WRITE_DAC,
        )
    }

    pub(super) fn replace(&self, from: &OsStr, to: &OsStr, replace: bool) -> io::Result<()> {
        let from = wide(&self.path.join(from));
        let to = wide(&self.path.join(to));
        // SAFETY: terminated paths are within the pinned directory. ReplaceFile merges
        // the current DACL and named streams, and does not ignore merge failures.
        let succeeded = unsafe {
            if replace {
                ReplaceFileW(
                    to.as_ptr(),
                    from.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            } else {
                MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH)
            }
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(super) fn remove(&self, name: &OsStr) -> io::Result<()> {
        fs::remove_file(self.path.join(name))
    }

    pub(super) fn sync(&self, name: &OsStr) -> io::Result<()> {
        match open_file(&self.path.join(name), true) {
            Ok(file) => file.sync_all(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub(super) fn identity(file: &File) -> io::Result<[u64; 2]> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle and writable output remain live throughout the call.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok([
        u64::from(information.dwVolumeSerialNumber),
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    ])
}

pub(super) fn private_lock_options(options: &mut OpenOptions) {
    options
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH);
}

pub(super) fn preserve(source: &File, replacement: &File) -> io::Result<()> {
    // Match the original inheritance state before ReplaceFile merges its streams.
    crate::vault::windows_protection::preserve_file_dacl(source, replacement)
}
