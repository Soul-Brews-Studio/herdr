//! One server per session at a time: a server holds an exclusive lock on `herdr-server.lock` in its
//! session directory for its whole life, so a server started while the old one is still shutting down
//! (white laris-co, 2026-10-29: twelve reconnecting clients started a second server mid-shutdown)
//! waits for it, then gives up. The file is opened close-on-exec (Rust's default), so pane processes
//! never inherit the lock.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) const LOCK_FILE: &str = "herdr-server.lock";
pub(crate) const START_WAIT: Duration = Duration::from_secs(15);

pub(crate) fn lock_path() -> PathBuf {
    crate::session::data_dir().join(LOCK_FILE)
}

/// Holds the lock until dropped (the end of the server process).
pub(crate) struct ServerLock {
    _file: File,
}

#[cfg(unix)]
fn try_flock(file: &File) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    // SAFETY: `file` owns a valid descriptor for the duration of the call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err(err)
    }
}

#[cfg(not(unix))]
fn try_flock(_file: &File) -> io::Result<bool> {
    Ok(true)
}

fn open(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
}

/// Takes the lock, waiting up to `wait` for a previous server of this session to finish. `Ok(None)`
/// means it is still held after `wait`.
pub(crate) fn acquire(path: &Path, wait: Duration) -> io::Result<Option<ServerLock>> {
    let file = open(path)?;
    let deadline = Instant::now() + wait;
    loop {
        if try_flock(&file)? {
            return Ok(Some(ServerLock { _file: file }));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// For a live-handoff import: the exporting server still holds the lock while it hands over, so take
/// it in the background once that server has exited, and keep it for this process's life.
pub(crate) fn acquire_after_handoff(path: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("herdr-server-lock".into())
        .spawn(move || loop {
            match acquire(&path, Duration::from_secs(3600)) {
                Ok(Some(lock)) => {
                    std::mem::forget(lock);
                    return;
                }
                Ok(None) => continue,
                Err(err) => {
                    tracing::warn!(%err, path = %path.display(), "could not take the server lock after handoff");
                    return;
                }
            }
        });
}

/// Linux: true while systemd is stopping the machine. A server must not start then: it would come
/// up, resume agents and be killed a moment later (the 2026-10-29 white reboot).
pub(crate) fn system_is_stopping() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("systemctl")
            .arg("is-system-running")
            .output()
            .ok()
            .is_some_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "stopping")
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "herdr-server-lock-{}-{}/{LOCK_FILE}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[cfg(unix)]
    #[test]
    fn a_second_server_waits_then_gives_up_while_the_first_holds_the_lock() {
        let path = path();
        let first = acquire(&path, Duration::ZERO)
            .unwrap()
            .expect("first server takes the lock");
        let started = Instant::now();
        assert!(acquire(&path, Duration::from_millis(300))
            .unwrap()
            .is_none());
        assert!(started.elapsed() >= Duration::from_millis(300));
        drop(first);
        // a test thread forking at the same moment holds an inherited copy of the descriptor until its
        // exec (close-on-exec), so give the release a moment instead of zero
        assert!(
            acquire(&path, Duration::from_secs(2)).unwrap().is_some(),
            "free once the first one exits"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_waiting_server_starts_as_soon_as_the_old_one_exits() {
        let path = path();
        let first = acquire(&path, Duration::ZERO).unwrap().unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(first);
        });
        let second = acquire(&path, Duration::from_secs(5)).unwrap();
        releaser.join().unwrap();
        assert!(second.is_some());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
