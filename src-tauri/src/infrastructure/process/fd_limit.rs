pub(crate) fn raise_open_file_limit() -> std::io::Result<()> {
    let mut limit = current_limit()?;
    let target = target_soft_limit(limit.rlim_max)?;
    if limit.rlim_cur >= target {
        return Ok(());
    }
    limit.rlim_cur = target;
    // SAFETY: limit is a valid rlimit that only raises the soft limit up to the hard limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn current_limit() -> std::io::Result<libc::rlimit> {
    let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
    // SAFETY: getrlimit writes one rlimit value to the valid out pointer.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: getrlimit succeeded and initialized the value.
    Ok(unsafe { limit.assume_init() })
}

#[cfg(target_os = "macos")]
fn target_soft_limit(hard: libc::rlim_t) -> std::io::Result<libc::rlim_t> {
    let mut max_files_per_process: libc::c_int = 0;
    let mut size = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated and the out pointer and size describe one c_int.
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.maxfilesperproc".as_ptr(),
            (&mut max_files_per_process as *mut libc::c_int).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(hard.min(max_files_per_process as libc::rlim_t))
}

#[cfg(not(target_os = "macos"))]
fn target_soft_limit(hard: libc::rlim_t) -> std::io::Result<libc::rlim_t> {
    Ok(hard)
}

#[cfg(test)]
#[path = "fd_limit_test.rs"]
mod fd_limit_tests;
