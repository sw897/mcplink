#[cfg(any(windows, target_os = "macos"))]
pub mod cli_sync;
pub mod desktop_rpc;
#[cfg(target_os = "macos")]
pub mod macos_update;
#[cfg(any(windows, target_os = "macos"))]
pub mod update_catalog;
#[cfg(any(windows, target_os = "macos"))]
mod update_files;
#[cfg(any(windows, target_os = "macos"))]
pub mod update_signature;

pub const UPDATE_PUBLIC_KEY: &str = include_str!("../updater.pub");
