pub(crate) fn spawn_successor() -> Result<(), String> {
    let pid = std::process::id();
    let started = crate::infrastructure::local_api::process_start_time(pid)
        .ok_or("UI process identity is unavailable")?;
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .arg("--internal-restart")
        .arg(pid.to_string())
        .arg(started.to_string())
        .args(std::env::args().filter(|arg| arg == "--hidden"))
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn wait_for_predecessor() -> Result<bool, String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_none_or(|arg| arg != "--internal-restart") {
        return Ok(false);
    }
    let pid = args
        .get(1)
        .ok_or("Missing predecessor pid")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let started = args
        .get(2)
        .ok_or("Missing predecessor identity")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let observation = crate::infrastructure::local_api::lookup_process_start_time(pid);
        if observation.process_list_available && observation.start_time != Some(started) {
            return Ok(true);
        }
        if std::time::Instant::now() >= deadline {
            return Err("Previous UI exit could not be confirmed.".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub(crate) fn restart(app: &tauri::AppHandle) -> Result<(), String> {
    spawn_successor()?;
    super::native_termination::exit(app, 0);
    Ok(())
}
