//! Validate the archive before installation and check any fixed helper binding.
//! No Keychain values are read; the lifecycle lock survives until installation finishes.
use flate2::bufread::GzDecoder;
use mcplink_core::keychain_helper::{UpdateGuard, update_guard};
use std::{
    collections::HashSet,
    ffi::OsStr,
    io::{self, Cursor, Read, Take},
    path::{Component, Path, PathBuf},
};

const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;
const MAX_EXTENSION: u64 = 16 * 1024;

pub fn validate_package(data: &Path, bytes: &[u8]) -> Result<UpdateGuard, String> {
    let guard = update_guard(data).map_err(|e| e.to_string())?;
    validate_headers(bytes)?;
    let temporary = tempfile::Builder::new()
        .prefix("mcplink-update-check-")
        .tempdir()
        .map_err(|_| "无法创建更新检查目录")?;
    let mut expanded = expanded(bytes);
    let mut archive = tar::Archive::new(&mut expanded);
    let mut paths = HashSet::new();
    let mut required = HashSet::from([
        "mcplink.app/Contents/Info.plist",
        "mcplink.app/Contents/MacOS/mcplink",
        "mcplink.app/Contents/MacOS/mcplink-desktop",
    ]);
    for entry in archive.entries().map_err(|_| "更新包归档无效")? {
        let mut entry = entry.map_err(|_| "更新包文件无效")?;
        let name = entry.path().map_err(|_| "更新包路径无效")?;
        let mut components = name.components();
        if components.next() != Some(Component::Normal(OsStr::new("mcplink.app")))
            || components.any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("更新包包含越界路径，未安装".into());
        }
        let relative: PathBuf = name.components().collect();
        let path = relative.to_str().ok_or("更新包路径编码无效")?;
        let kind = entry.header().entry_type();
        if path.len() > 4096
            || path.chars().any(|ch| ch == '\\' || ch.is_control())
            || !(kind.is_file() || kind.is_dir())
            || (relative == Path::new("mcplink.app") && !kind.is_dir())
            || !paths.insert(path.to_lowercase())
        {
            return Err("更新包必须只包含一个 mcplink.app，且不能包含链接或重复路径".into());
        }
        if paths.len() > MAX_ENTRIES || entry.size() > MAX_EXPANDED {
            return Err("更新包解压范围过大".into());
        }
        if kind.is_file() && entry.size() > 0 {
            required.remove(path);
        }
        if kind.is_file() && temporary.path().join(&relative).exists() {
            return Err("更新包包含重复文件路径".into());
        }
        // Reject special files/links above; unpack_in also checks containment.
        if !entry
            .unpack_in(temporary.path())
            .map_err(|_| "无法解包更新检查文件")?
        {
            return Err("更新包路径未留在临时目录".into());
        }
    }
    if !required.is_empty() {
        return Err("更新包缺少 mcplink 应用文件".into());
    }
    if guard.has_helper() {
        guard
            .validate_app(&temporary.path().join("mcplink.app"))
            .map_err(|e| format!("更新与固定凭据助手不兼容，当前程序保持不变：{e}"))?;
    }
    Ok(guard)
}

fn expanded(bytes: &[u8]) -> Take<GzDecoder<Cursor<&[u8]>>> {
    GzDecoder::new(Cursor::new(bytes)).take(MAX_EXPANDED + 1)
}

fn validate_headers(bytes: &[u8]) -> Result<(), String> {
    let mut decoded = expanded(bytes);
    {
        // Bound extension records before tar materializes effective paths and metadata.
        let mut archive = tar::Archive::new(&mut decoded);
        let entries = archive.entries().map_err(|_| "更新包归档无效")?.raw(true);
        for (index, entry) in entries.enumerate() {
            let mut entry = entry.map_err(|_| "更新包归档无效")?;
            let kind = entry.header().entry_type();
            if index >= 2 * MAX_ENTRIES || entry.size() > MAX_EXPANDED {
                return Err("更新包解压范围过大".into());
            }
            if kind.is_gnu_longname() || kind.is_pax_local_extensions() {
                if entry.size() > MAX_EXTENSION {
                    return Err("更新包扩展记录过大".into());
                }
            } else if !(kind.is_file() || (kind.is_dir() && entry.size() == 0)) {
                return Err("更新包包含链接或不支持的文件类型".into());
            }
            if io::copy(&mut entry, &mut io::sink()).map_err(|_| "更新包归档无效")? != entry.size()
            {
                return Err("更新包归档不完整".into());
            }
        }
    }
    io::copy(&mut decoded, &mut io::sink()).map_err(|_| "更新包压缩流无效")?;
    if decoded.limit() == 0 || decoded.into_inner().into_inner().position() != bytes.len() as u64 {
        return Err("更新包解压范围过大或包含额外压缩流".into());
    }
    Ok(())
}
