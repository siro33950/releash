#[cfg(unix)]
pub fn acquire_all(
    data_dirs: &[std::path::PathBuf],
    activate: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<Option<Vec<std::fs::File>>> {
    use fs2::FileExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    let mut directories = Vec::new();
    for data_dir in data_dirs {
        std::fs::create_dir_all(data_dir)?;
        let canonical = data_dir.canonicalize()?;
        if !directories.contains(&canonical) {
            directories.push(canonical);
        }
    }
    let mut locks = Vec::new();
    for data_dir in &directories {
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data_dir.join("desktop.lock"))?;
        let socket = data_dir.join("desktop.sock");
        match lock.try_lock_exclusive() {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let context =
                    releashd::desktop_api::with_timeout(std::time::Duration::from_secs(1));
                loop {
                    if UnixStream::connect(&socket).is_ok() {
                        return Ok(None);
                    }
                    if releashd::desktop_api::sleep(
                        &context,
                        crate::common::retry::DESKTOP_POLL.delay(1, 1.0),
                    )
                    .is_err()
                    {
                        break;
                    }
                }
                return Err(std::io::Error::other(
                    "The running UI could not be activated.",
                ));
            }
            Err(error) => return Err(error),
        }
        locks.push(lock);
    }
    let mut listeners = Vec::new();
    for data_dir in &directories {
        let socket = data_dir.join("desktop.sock");
        match std::fs::remove_file(&socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&socket)?;
        listeners.push(listener);
    }
    let activate = std::sync::Arc::new(activate);
    for listener in listeners {
        let activate = activate.clone();
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                if connection.is_ok() {
                    activate();
                }
            }
        });
    }
    Ok(Some(locks))
}

#[cfg(not(unix))]
pub fn acquire_all(
    _data_dirs: &[std::path::PathBuf],
    _activate: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<Option<Vec<std::fs::File>>> {
    Err(std::io::Error::other("Desktop supervision requires macOS."))
}

#[cfg(feature = "test-support")]
pub fn acquire(
    data_dir: &std::path::Path,
    activate: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<Option<std::fs::File>> {
    acquire_all(&[data_dir.to_path_buf()], activate)
        .map(|locks| locks.map(|mut locks| locks.remove(0)))
}
