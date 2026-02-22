use anyhow::Context;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;

pub enum AcquireResult {
    Acquired(SingleInstanceGuard),
    AlreadyRunning,
}

pub struct SingleInstanceGuard {
    #[cfg(unix)]
    _lock_file: std::fs::File,
    #[cfg(not(any(unix, windows)))]
    _lock_file: std::fs::File,
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.handle) };
    }
}

fn lock_file_path() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("vleer")
        .join("instance.lock")
}

#[cfg(unix)]
pub fn try_acquire() -> anyhow::Result<AcquireResult> {
    use std::io::ErrorKind;
    use std::os::fd::AsRawFd;
    use std::os::raw::c_int;

    const LOCK_EX: c_int = 2;
    const LOCK_NB: c_int = 4;

    unsafe extern "C" {
        fn flock(fd: c_int, operation: c_int) -> c_int;
    }

    let lock_path = lock_file_path();
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create lock directory '{}'", parent.display()))?;
    }

    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open lock file '{}'", lock_path.display()))?;

    let result = unsafe { flock(lock_file.as_raw_fd(), LOCK_EX | LOCK_NB) };
    if result == 0 {
        return Ok(AcquireResult::Acquired(SingleInstanceGuard {
            _lock_file: lock_file,
        }));
    }

    let err = std::io::Error::last_os_error();
    if err.kind() == ErrorKind::WouldBlock {
        return Ok(AcquireResult::AlreadyRunning);
    }

    Err(err).with_context(|| format!("failed to acquire lock file '{}'", lock_path.display()))
}

#[cfg(windows)]
pub fn try_acquire() -> anyhow::Result<AcquireResult> {
    use anyhow::anyhow;
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;

    let handle = unsafe { CreateMutexW(None, false, w!("Global\\VleerSingleInstance"))? };
    let last_error = unsafe { GetLastError() };
    if last_error == ERROR_ALREADY_EXISTS {
        let _ = unsafe { CloseHandle(handle) };
        return Ok(AcquireResult::AlreadyRunning);
    }

    Ok(AcquireResult::Acquired(SingleInstanceGuard { handle }))
}

#[cfg(not(any(unix, windows)))]
pub fn try_acquire() -> anyhow::Result<AcquireResult> {
    use std::io::ErrorKind;

    let lock_path = lock_file_path();
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create lock directory '{}'", parent.display()))?;
    }

    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&lock_path);

    match lock_file {
        Ok(file) => Ok(AcquireResult::Acquired(SingleInstanceGuard {
            _lock_file: file,
        })),
        Err(err) if err.kind() == ErrorKind::AlreadyExists => Ok(AcquireResult::AlreadyRunning),
        Err(err) => Err(err)
            .with_context(|| format!("failed to create lock file '{}'", lock_path.display())),
    }
}
