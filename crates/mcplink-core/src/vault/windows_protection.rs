use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        ACL,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT, SetSecurityInfo,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
        GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    },
    Storage::FileSystem::{
        CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_FLAG_WRITE_THROUGH,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
        WRITE_DAC,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

struct LocalAllocation(*mut c_void);
impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: the owning Windows API returned this LocalAlloc buffer.
        unsafe { LocalFree(self.0) };
    }
}

fn user_sid() -> io::Result<String> {
    let mut token = null_mut();
    // SAFETY: all handles and buffers are live and each successful call owns its output.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(token);
        let mut bytes = 0;
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut bytes);
        if bytes < size_of::<TOKEN_USER>() as u32 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut sid = null_mut();
        if ConvertSidToStringSidW((*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid, &mut sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _owned = LocalAllocation(sid.cast());
        let length = (0..).find(|&i| *sid.add(i) == 0).unwrap();
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
            sid, length,
        )))
    }
}

fn descriptor(directory: bool) -> io::Result<LocalAllocation> {
    let user = user_sid()?;
    let inheritance = if directory { "OICI" } else { "" };
    let value = format!("D:P(A;{inheritance};FA;;;{user})(A;{inheritance};FA;;;SY)");
    let value: Vec<_> = value.encode_utf16().chain(Some(0)).collect();
    let mut sd = null_mut();
    // SAFETY: the input is terminated and the result is freed by LocalAllocation.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            value.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(LocalAllocation(sd))
}

fn attributes(sd: &LocalAllocation) -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(super) fn private_directory(path: &Path) -> io::Result<()> {
    let sd = descriptor(true)?;
    let attrs = attributes(&sd);
    // SAFETY: path and security descriptor are live. Creation applies the ACL before publication.
    if unsafe { CreateDirectoryW(wide(path).as_ptr(), &attrs) } == 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
        restrict_existing(path, true, &sd)?;
    }
    Ok(())
}

pub(crate) fn create_private_file(path: &Path) -> io::Result<File> {
    create_private_file_with_access(path, GENERIC_WRITE)
}

pub(crate) fn create_private_file_with_access(path: &Path, access: u32) -> io::Result<File> {
    let sd = descriptor(false)?;
    let attrs = attributes(&sd);
    // SAFETY: successful creation returns a unique owned handle for a new file.
    let handle = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            access,
            0,
            &attrs,
            CREATE_NEW,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

pub(crate) fn preserve_file_dacl(source: &File, replacement: &File) -> io::Result<()> {
    let mut descriptor = null_mut();
    let mut acl = null_mut();
    // SAFETY: the source handle stays live and the out-pointers are writable.
    let error = unsafe {
        GetSecurityInfo(
            source.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let descriptor = LocalAllocation(descriptor);
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: the descriptor owns the ACL and remains live through the write.
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let inheritance = if control & SE_DACL_PROTECTED != 0 {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };
    // SAFETY: the private replacement handle is open with WRITE_DAC. The ACL,
    // including a null DACL if present on the source, comes from its descriptor.
    let error = unsafe {
        SetSecurityInfo(
            replacement.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | inheritance,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    Ok(())
}

pub(super) fn restrict_file(path: &Path) -> io::Result<()> {
    restrict_existing(path, false, &descriptor(false)?)
}

fn restrict_existing(path: &Path, directory: bool, sd: &LocalAllocation) -> io::Result<()> {
    let mut share_mode = FILE_SHARE_READ | FILE_SHARE_WRITE;
    if !directory {
        // The ACL update targets the verified file handle.
        share_mode |= FILE_SHARE_DELETE;
    }
    let mut options = OpenOptions::new();
    options
        .access_mode(WRITE_DAC | READ_CONTROL | FILE_READ_ATTRIBUTES)
        .share_mode(share_mode)
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        );
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "私有存储位置无效",
        ));
    }
    if !directory {
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: file stays open and information is writable.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if information.nNumberOfLinks != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "私有存储文件存在多处链接",
            ));
        }
    }
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl: *mut ACL = null_mut();
    // SAFETY: the ACL borrows the live descriptor and SetSecurityInfo targets the checked handle.
    unsafe {
        if GetSecurityDescriptorDacl(sd.0, &mut present, &mut acl, &mut defaulted) == 0
            || acl.is_null()
        {
            return Err(io::Error::last_os_error());
        }
        let error = SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        );
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
    }
    Ok(())
}
