pub mod controllers;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod mpris;

#[cfg(target_os = "windows")]
pub mod windows;