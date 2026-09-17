#[cfg(unix)]
pub(crate) fn acquire(
    data_dir: &std::path::Path,
    activate: impl Fn() + Send + 'static,
) -> std::io::Result<Option<std::fs::File>> {
    use fs2::FileExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    std::fs::create_dir_all(data_dir)?;
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
            for _ in 0..50 {
                if UnixStream::connect(&socket).is_ok() {
                    return Ok(None);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            return Err(std::io::Error::other(
                "The running UI could not be activated.",
            ));
        }
        Err(error) => return Err(error),
    }
    match std::fs::remove_file(&socket) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(&socket)?;
    std::thread::spawn(move || {
        for connection in listener.incoming() {
            if connection.is_ok() {
                activate();
            }
        }
    });
    Ok(Some(lock))
}

#[cfg(not(unix))]
pub(crate) fn acquire(
    _data_dir: &std::path::Path,
    _activate: impl Fn() + Send + 'static,
) -> std::io::Result<Option<std::fs::File>> {
    Err(std::io::Error::other("Desktop supervision requires macOS."))
}

#[cfg(all(test, unix))]
#[path = "single_instance_test.rs"]
mod single_instance_tests;
