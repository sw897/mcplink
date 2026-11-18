//! Validate a signed main-program update against the existing credential helper.
//! No Keychain values are read; the lifecycle lock survives until installation finishes.
use flate2::read::GzDecoder;
use mcplink_core::keychain_helper::{UpdateGuard, update_guard};
use std::{
    collections::HashSet,
    io::Cursor,
    path::{Component, Path, PathBuf},
};

pub fn validate_package(data: &Path, bytes: &[u8]) -> Result<UpdateGuard, String> {
    let guard = update_guard(data).map_err(|e| e.to_string())?;
    if !guard.has_helper() {
        return Ok(guard);
    }
    let temporary = tempfile::Builder::new()
        .prefix("mcplink-update-check-")
        .tempdir()
        .map_err(|_| "无法创建更新检查目录")?;
    let mut archive = tar::Archive::new(GzDecoder::new(Cursor::new(bytes)));
    let mut paths = HashSet::new();
    let mut size = 0_u64;
    for entry in archive.entries().map_err(|_| "更新包归档无效")? {
        let mut entry = entry.map_err(|_| "更新包文件无效")?;
        let name = entry.path().map_err(|_| "更新包路径无效")?;
        let mut relative = PathBuf::new();
        for component in name.components() {
            match component {
                Component::Normal(value) => relative.push(value),
                Component::CurDir => {}
                _ => return Err("更新包包含越界路径，未安装".into()),
            }
        }
        if relative.components().next().is_none()
            || !relative.starts_with("mcplink.app")
            || !(entry.header().entry_type().is_file() || entry.header().entry_type().is_dir())
            || !paths.insert(relative.clone())
        {
            return Err("更新包必须只包含一个 mcplink.app，且不能包含链接或重复路径".into());
        }
        size = size.checked_add(entry.size()).ok_or("更新包解压大小无效")?;
        if size > 1024 * 1024 * 1024 || paths.len() > 20_000 {
            return Err("更新包解压范围过大".into());
        }
        // Reject special files/links above; unpack_in also checks containment.
        if !entry
            .unpack_in(temporary.path())
            .map_err(|_| "无法解包更新检查文件")?
        {
            return Err("更新包路径未留在临时目录".into());
        }
    }
    guard
        .validate_app(&temporary.path().join("mcplink.app"))
        .map_err(|e| format!("更新与固定凭据助手不兼容，当前程序保持不变：{e}"))?;
    Ok(guard)
}
