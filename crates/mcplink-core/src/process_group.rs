//! Synchronous group cancellation also covers a transport dropped during startup.
use process_wrap::tokio::{ChildWrapper, CommandWrap, CommandWrapper};

#[cfg(target_os = "macos")]
pub fn search_path() -> anyhow::Result<std::ffi::OsString> {
    let mut paths: Vec<_> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let home = crate::config::user_home()?;
    let mut extra = vec![
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join(".bun/bin"),
    ];
    if std::env::var_os("MCPLINK_HOME").is_none() {
        extra.extend(
            [
                "/opt/homebrew/bin",
                "/opt/homebrew/sbin",
                "/usr/local/bin",
                "/usr/local/sbin",
            ]
            .map(std::path::PathBuf::from),
        );
    }
    extra.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(std::path::PathBuf::from));
    for path in extra {
        if path.is_dir() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    Ok(std::env::join_paths(paths)?)
}

#[derive(Debug)]
pub struct KillGroupOnDrop;

impl CommandWrapper for KillGroupOnDrop {
    fn wrap_child(
        &mut self,
        child: Box<dyn ChildWrapper>,
        _core: &CommandWrap,
    ) -> std::io::Result<Box<dyn ChildWrapper>> {
        Ok(Box::new(GroupGuard { child: Some(child) }))
    }
}

#[derive(Debug)]
struct GroupGuard {
    // This wrapper is installed after ProcessGroup, so start_kill targets the owned group.
    child: Option<Box<dyn ChildWrapper>>,
}

impl ChildWrapper for GroupGuard {
    #[cfg(windows)]
    fn process_handle(&self) -> Option<std::os::windows::io::BorrowedHandle<'_>> {
        self.inner().process_handle()
    }
    fn inner(&self) -> &dyn ChildWrapper {
        self.child.as_deref().unwrap()
    }
    fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
        self.child.as_deref_mut().unwrap()
    }
    fn into_inner(mut self: Box<Self>) -> Box<dyn ChildWrapper> {
        self.child.take().unwrap()
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        // The SDK can schedule async transport cleanup while the runtime is stopping.
        // Send the group signal now, then let the SDK/Tokio reap the owned child.
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
    }
}
