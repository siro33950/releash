use parking_lot::Mutex;

pub(crate) struct Osc7Directory {
    pending: Vec<u8>,
    pub(crate) cwd: Option<String>,
    local_host: Option<String>,
    updated: bool,
}

impl Default for Osc7Directory {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            cwd: None,
            updated: false,
            local_host: local_hostname(),
        }
    }
}

fn local_hostname() -> Option<String> {
    #[cfg(unix)]
    {
        let mut name = [0u8; 256];
        if unsafe { libc::gethostname(name.as_mut_ptr().cast(), name.len()) } != 0 {
            return None;
        }
        std::ffi::CStr::from_bytes_until_nul(&name)
            .ok()?
            .to_str()
            .ok()
            .map(str::to_owned)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

impl Osc7Directory {
    pub(crate) fn observation(&mut self) -> (Option<String>, bool) {
        let updated = std::mem::take(&mut self.updated);
        (self.cwd.clone(), updated)
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.pending.is_empty() {
                if byte == 0x1b {
                    self.pending.push(byte);
                }
                continue;
            }
            if self.pending.len() == 1 && byte != b']' {
                self.pending.clear();
                if byte == 0x1b {
                    self.pending.push(byte);
                }
                continue;
            }
            self.pending.push(byte);
            let bell = byte == 7;
            let st = self.pending.ends_with(b"\x1b\\");
            if bell || st {
                let end = self.pending.len() - if bell { 1 } else { 2 };
                if let Ok(value) = std::str::from_utf8(&self.pending[2..end]) {
                    if let Some(uri) = value.strip_prefix("7;") {
                        if let Ok(mut uri) = url::Url::parse(uri) {
                            if uri.scheme() == "file" {
                                let local = uri.host_str().is_none_or(|host| {
                                    host.eq_ignore_ascii_case("localhost")
                                        || self
                                            .local_host
                                            .as_deref()
                                            .is_some_and(|local| host.eq_ignore_ascii_case(local))
                                });
                                self.cwd = if local && uri.set_host(Some("localhost")).is_ok() {
                                    uri.to_file_path()
                                        .ok()
                                        .and_then(|path| path.to_str().map(str::to_owned))
                                } else {
                                    None
                                };
                                self.updated = true;
                            }
                        }
                    }
                }
                self.pending.clear();
            } else if self.pending.len() > 8192 {
                self.pending.clear();
            }
        }
    }
}

pub(crate) type SharedOsc7Directory = std::sync::Arc<Mutex<Osc7Directory>>;

#[cfg(target_os = "macos")]
pub(crate) fn process_cwd(pid: u32) -> std::io::Result<String> {
    let mut info = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>();
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size as i32,
        )
    };
    if read != size as i32 {
        return Err(std::io::Error::last_os_error());
    }
    let info = unsafe { info.assume_init() };
    let path = unsafe { std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast()) };
    path.to_str()
        .map(str::to_owned)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(target_os = "linux")]
pub(crate) fn process_cwd(pid: u32) -> std::io::Result<String> {
    std::fs::read_link(format!("/proc/{pid}/cwd"))?
        .into_os_string()
        .into_string()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid shell cwd"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn process_cwd(_pid: u32) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Shell cwd observation unavailable",
    ))
}

#[cfg(test)]
#[path = "shell_observation_test.rs"]
mod shell_observation_tests;

#[cfg(target_os = "macos")]
pub(crate) fn process_executable(pid: u32) -> std::io::Result<String> {
    let mut path = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let read =
        unsafe { libc::proc_pidpath(pid as i32, path.as_mut_ptr().cast(), path.len() as u32) };
    if read <= 0 {
        return Err(std::io::Error::last_os_error());
    }
    let path = std::ffi::CStr::from_bytes_until_nul(&path)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    path.to_str()
        .map(str::to_owned)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(target_os = "linux")]
pub(crate) fn process_executable(pid: u32) -> std::io::Result<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))?
        .into_os_string()
        .into_string()
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid process executable",
            )
        })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn process_executable(_pid: u32) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Process executable observation unavailable",
    ))
}

#[cfg(target_os = "macos")]
pub(crate) fn child_process_groups(pid: u32) -> std::io::Result<Vec<i32>> {
    let size = unsafe { libc::proc_listchildpids(pid as i32, std::ptr::null_mut(), 0) };
    if size < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut children = vec![0i32; size as usize + 16];
    let read = unsafe {
        libc::proc_listchildpids(
            pid as i32,
            children.as_mut_ptr().cast(),
            std::mem::size_of_val(children.as_slice()) as i32,
        )
    };
    if read < 0 {
        return Err(std::io::Error::last_os_error());
    }
    children.truncate(read as usize);
    Ok(children
        .into_iter()
        .filter_map(|child| {
            let group = unsafe { libc::getpgid(child) };
            (group > 0).then_some(group)
        })
        .collect())
}

#[cfg(target_os = "linux")]
pub(crate) fn child_process_groups(pid: u32) -> std::io::Result<Vec<i32>> {
    let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))?;
    Ok(children
        .split_whitespace()
        .filter_map(|child| {
            let pid = child.parse::<i32>().ok()?;
            let group = unsafe { libc::getpgid(pid) };
            (group > 0).then_some(group)
        })
        .collect())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn child_process_groups(_pid: u32) -> std::io::Result<Vec<i32>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Child process observation unavailable",
    ))
}
