use crate::{
    agent_config, config,
    model::{Agent, Source},
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn names(agent: &Agent) -> (&'static str, &'static str, &'static str, &'static str) {
    match agent {
        Agent::Codex => ("Codex", "Codex.exe", "Codex.app", "codex"),
        Agent::Hermes => ("Hermes", "hermes.exe", "Hermes.app", "hermes"),
        Agent::Claude => ("Claude Code", "claude.exe", "", "claude"),
        Agent::Opencode => (
            "ai.opencode.desktop",
            "OpenCode.exe",
            "OpenCode.app",
            "opencode",
        ),
        Agent::Zcode => ("ZCode", "ZCode.exe", "ZCode.app", "zcode"),
        Agent::Deepseek => (
            "DeepSeek Harness",
            "DeepSeek Harness.exe",
            "DeepSeek Harness.app",
            "dsh",
        ),
        Agent::Openclaw => ("OpenClaw", "OpenClaw.exe", "OpenClaw.app", "openclaw"),
        Agent::Kimicode => ("Kimi Code", "Kimi Code.exe", "Kimi Code.app", "kimi"),
        Agent::Removed => unreachable!(),
    }
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

pub fn valid_launcher(agent: &Agent, path: &Path) -> bool {
    if *agent == Agent::Removed || !path.is_absolute() || !executable(path) {
        return false;
    }
    let (_, _, _, cli) = names(agent);
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if cfg!(windows) {
        ["exe", "cmd", "bat"]
            .iter()
            .any(|ext| name.eq_ignore_ascii_case(&format!("{cli}.{ext}")))
    } else {
        name == cli
    }
}

fn installed(agent: &Agent, store: &crate::store::Store) -> Result<bool> {
    if store
        .launcher(agent)?
        .as_deref()
        .is_some_and(|p| valid_launcher(agent, Path::new(p)))
    {
        return Ok(true);
    }
    let home = config::user_home()?;
    let isolated = std::env::var_os("MCPLINK_HOME").is_some();
    let local = if std::env::var_os("MCPLINK_HOME").is_none() {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Local"))
    } else {
        home.join("AppData/Local")
    };
    let (folder, executable_name, mac_app, cli) = names(agent);
    let mut installed = if cfg!(windows) {
        local
            .join("Programs")
            .join(folder)
            .join(executable_name)
            .is_file()
    } else if cfg!(target_os = "macos") && !mac_app.is_empty() {
        (!isolated && mac_app_installed(&Path::new("/Applications").join(mac_app)))
            || mac_app_installed(&home.join("Applications").join(mac_app))
    } else {
        false
    };
    // Inspect launcher files, never execute programs during inventory.
    let mut launchers = vec![
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
    ];
    #[cfg(target_os = "macos")]
    if !isolated {
        launchers.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    #[cfg(unix)]
    {
        let nvm = std::env::var_os("NVM_DIR")
            .map(PathBuf::from)
            .filter(|p| !isolated || p.starts_with(&home))
            .unwrap_or_else(|| home.join(".nvm"));
        if let Ok(versions) = std::fs::read_dir(nvm.join("versions/node")) {
            launchers.extend(versions.flatten().map(|v| v.path().join("bin")));
        }
    }
    if cfg!(windows) {
        let roaming = if isolated {
            home.join("AppData/Roaming")
        } else {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData/Roaming"))
        };
        launchers.push(roaming.join("npm"));
    }
    if let Some(paths) = std::env::var_os("PATH") {
        launchers.extend(
            std::env::split_paths(&paths).filter(|path| !isolated || path.starts_with(&home)),
        );
    }
    installed |= launchers.iter().any(|path| {
        if cfg!(windows) {
            ["exe", "cmd", "bat"]
                .iter()
                .any(|extension| path.join(format!("{cli}.{extension}")).is_file())
        } else {
            executable(&path.join(cli))
        }
    });
    #[cfg(windows)]
    if !isolated && *agent == Agent::Codex && !installed {
        installed = codex_package_installed();
    }
    if cfg!(windows)
        && !isolated
        && *agent == Agent::Codex
        && !installed
        && let Some(programs) = std::env::var_os("ProgramFiles")
        && let Ok(packages) = std::fs::read_dir(PathBuf::from(programs).join("WindowsApps"))
    {
        installed = packages.flatten().any(|package| {
            package
                .file_name()
                .to_string_lossy()
                .starts_with("OpenAI.Codex_")
                && package.path().join("app/resources/codex.exe").is_file()
        });
    }
    Ok(installed)
}

fn mac_app_installed(bundle: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Ok(info) = plist::Value::from_file(bundle.join("Contents/Info.plist")) else {
            return false;
        };
        let Some(executable) = info
            .as_dictionary()
            .and_then(|value| value.get("CFBundleExecutable"))
            .and_then(plist::Value::as_string)
        else {
            return false;
        };
        let mut components = Path::new(executable).components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return false;
        }
        bundle.join("Contents/MacOS").join(executable).is_file()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bundle;
        false
    }
}

#[cfg(windows)]
fn codex_package_installed() -> bool {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
        Storage::Packaging::Appx::{GetPackagePathByFullName, GetPackagesByPackageFamily},
    };

    let family: Vec<u16> = "OpenAI.Codex_2p2nqsd0c76g0\0".encode_utf16().collect();
    let (mut count, mut length) = (0, 0);
    // SAFETY: The system returns required buffer sizes first. All buffers and the
    // pointers into `names` remain alive through both queries and each path lookup.
    // Query the current user's registered package; WindowsApps directory listing
    // can be denied even when its registered executable is accessible.
    unsafe {
        if GetPackagesByPackageFamily(
            family.as_ptr(),
            &mut count,
            ptr::null_mut(),
            &mut length,
            ptr::null_mut(),
        ) != ERROR_INSUFFICIENT_BUFFER
            || count == 0
            || length == 0
        {
            return false;
        }
        let mut packages = vec![ptr::null_mut(); count as usize];
        let mut names = vec![0u16; length as usize];
        if GetPackagesByPackageFamily(
            family.as_ptr(),
            &mut count,
            packages.as_mut_ptr(),
            &mut length,
            names.as_mut_ptr(),
        ) != ERROR_SUCCESS
        {
            return false;
        }
        packages.into_iter().take(count as usize).any(|package| {
            let mut path_length = 0;
            if package.is_null()
                || GetPackagePathByFullName(package, &mut path_length, ptr::null_mut())
                    != ERROR_INSUFFICIENT_BUFFER
                || path_length == 0
            {
                return false;
            }
            let mut path = vec![0u16; path_length as usize];
            if GetPackagePathByFullName(package, &mut path_length, path.as_mut_ptr())
                != ERROR_SUCCESS
            {
                return false;
            }
            let end = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            PathBuf::from(OsString::from_wide(&path[..end]))
                .join("app/resources/codex.exe")
                .is_file()
        })
    }
}

pub fn require_installed(agent: &Agent, store: &crate::store::Store) -> Result<()> {
    if !installed(agent, store)? {
        bail!("{} 未安装，无法配置；安装后请刷新", agent.label());
    }
    Ok(())
}

pub fn inventory(sources: &[Source], store: &crate::store::Store) -> Result<Value> {
    let home = config::user_home()?;
    let mut clients = Vec::new();
    for agent in Agent::supported() {
        let profile = match agent {
            Agent::Codex => std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            Agent::Hermes => std::env::var_os("HERMES_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".hermes")),
            Agent::Claude => config::claude_home()?,
            Agent::Opencode => crate::opencode::directory()?,
            _ => agent_config::directory(&agent)?,
        };
        let installed = installed(&agent, store)?;
        let detected = installed
            || profile.is_dir()
            || sources
                .iter()
                .any(|source| source.agent == agent && source.path.is_file());
        clients.push(
            json!({"id":agent,"label":agent.label(),"detected":detected,"installed":installed,
            "configurable":installed,"profilePath":profile,"management":"writable"}),
        );
    }
    Ok(Value::Array(clients))
}
