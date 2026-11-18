//! Public signing metadata only. No Keychain values or signing keys are read.
use anyhow::{Result, ensure};
use std::{ffi::c_void, os::unix::ffi::OsStrExt, path::Path};

type Ref = *const c_void;
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCodeCopySelf(flags: u32, code: *mut Ref) -> i32;
    fn SecCodeCopyStaticCode(code: Ref, flags: u32, static_code: *mut Ref) -> i32;
    fn SecStaticCodeCreateWithPath(path: Ref, flags: u32, code: *mut Ref) -> i32;
    fn SecRequirementCreateWithString(text: Ref, flags: u32, requirement: *mut Ref) -> i32;
    fn SecStaticCodeCheckValidity(code: Ref, flags: u32, requirement: Ref) -> i32;
    fn SecCodeCopySigningInformation(code: Ref, flags: u32, info: *mut Ref) -> i32;
    fn SecCertificateCopyData(certificate: Ref) -> Ref;
    static kSecCodeInfoCertificates: Ref;
    static kSecCodeInfoPList: Ref;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: Ref,
        bytes: *const u8,
        len: isize,
        directory: u8,
    ) -> Ref;
    fn CFStringCreateWithCString(
        allocator: Ref,
        bytes: *const std::ffi::c_char,
        encoding: u32,
    ) -> Ref;
    fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFDataGetLength(data: Ref) -> isize;
    fn CFDataGetBytePtr(data: Ref) -> *const u8;
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: Ref, kind: isize, value: *mut c_void) -> u8;
    fn CFRelease(value: Ref);
}
struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

pub(crate) fn certificate(path: &Path, identifier: &str) -> Result<Vec<u8>> {
    ensure!(
        matches!(
            identifier,
            "io.mcplink.keychain" | "io.mcplink.cli" | "io.mcplink.desktop"
        ),
        "不支持的 mcplink 签名标识"
    );
    let bytes = path.as_os_str().as_bytes();
    let requirement = std::ffi::CString::new(format!("identifier \"{identifier}\""))?;
    unsafe {
        let url = Owned(CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len() as isize,
            0,
        ));
        ensure!(!url.0.is_null(), "签名路径无效");
        let mut code = Owned(std::ptr::null());
        let text = Owned(CFStringCreateWithCString(
            std::ptr::null(),
            requirement.as_ptr(),
            0x08000100,
        ));
        let mut constraint = Owned(std::ptr::null());
        ensure!(
            !text.0.is_null()
                && SecStaticCodeCreateWithPath(url.0, 0, &mut code.0) == 0
                && SecRequirementCreateWithString(text.0, 0, &mut constraint.0) == 0
                && SecStaticCodeCheckValidity(code.0, 1 << 4, constraint.0) == 0,
            "mcplink 签名或固定标识无效"
        );
        extract_certificate(code.0)
    }
}

pub(crate) fn current_certificate() -> Result<Vec<u8>> {
    crate::vault::verify_running_code()?;
    unsafe {
        let mut code = Owned(std::ptr::null());
        let mut static_code = Owned(std::ptr::null());
        ensure!(
            SecCodeCopySelf(0, &mut code.0) == 0
                && SecCodeCopyStaticCode(code.0, 0, &mut static_code.0) == 0,
            "无法核对当前 mcplink 签名"
        );
        extract_certificate(static_code.0)
    }
}

/// Read the capability from signed metadata, never an adjacent unsigned file.
pub(crate) fn binding_version(path: &Path) -> Result<u32> {
    let bytes = path.as_os_str().as_bytes();
    unsafe {
        let url = Owned(CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len() as isize,
            0,
        ));
        let mut code = Owned(std::ptr::null());
        let mut info = Owned(std::ptr::null());
        ensure!(
            !url.0.is_null()
                && SecStaticCodeCreateWithPath(url.0, 0, &mut code.0) == 0
                && SecCodeCopySigningInformation(code.0, 1 << 1, &mut info.0) == 0,
            "无法读取更新程序的助手兼容版本"
        );
        let plist = CFDictionaryGetValue(info.0, kSecCodeInfoPList);
        if plist.is_null() {
            return Ok(1);
        }
        let key = Owned(CFStringCreateWithCString(
            std::ptr::null(),
            c"MCPLinkKeychainBindingVersion".as_ptr(),
            0x08000100,
        ));
        let number = CFDictionaryGetValue(plist, key.0);
        if number.is_null() {
            return Ok(1);
        }
        let mut version: i64 = 0;
        ensure!(
            CFGetTypeID(number) == CFNumberGetTypeID()
                && CFNumberGetValue(number, 4, (&mut version as *mut i64).cast()) != 0
                && (1..=1024).contains(&version),
            "更新程序的助手兼容版本无效"
        );
        Ok(version as u32)
    }
}

unsafe fn extract_certificate(code: Ref) -> Result<Vec<u8>> {
    unsafe {
        let mut info = Owned(std::ptr::null());
        ensure!(
            SecCodeCopySigningInformation(code, 1 << 1, &mut info.0) == 0,
            "无法读取代码签名元数据"
        );
        let certs = CFDictionaryGetValue(info.0, kSecCodeInfoCertificates);
        ensure!(
            !certs.is_null() && CFArrayGetCount(certs) > 0,
            "mcplink 必须使用证书签名，不能使用 ad-hoc 签名访问固定助手"
        );
        let data = Owned(SecCertificateCopyData(CFArrayGetValueAtIndex(certs, 0)));
        ensure!(!data.0.is_null(), "代码签名证书无效");
        let len = CFDataGetLength(data.0);
        let bytes = CFDataGetBytePtr(data.0);
        ensure!(
            len > 0 && len <= 1024 * 1024 && !bytes.is_null(),
            "代码签名证书无效"
        );
        Ok(std::slice::from_raw_parts(bytes, len as usize).to_vec())
    }
}
