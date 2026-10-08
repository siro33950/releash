use releashd::test_support::integration::process::*;
use std::time::Duration;
use tokio::process::Command;

#[cfg(unix)]
#[tokio::test]
async fn test_プロセス回収_終了を回収せず観測してsignal後に一度だけ回収する() {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: getpgrp only reads this process's own group identity.
        assert!(group_has_live_members(unsafe { libc::getpgrp() }).unwrap());
        assert!(group_has_live_members(i32::MAX).is_err());
    }
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    configure_process_group(&mut command);
    let mut child = command.spawn().unwrap();
    let pid = child.id().unwrap();
    tokio::time::timeout(Duration::from_secs(2), wait_without_reaping(pid))
        .await
        .unwrap()
        .unwrap();
    wait_without_reaping(pid).await.unwrap();
    #[cfg(target_os = "macos")]
    assert!(!group_has_live_members(pid as i32).unwrap());
    signal_process_group(pid as i32, libc::SIGKILL).unwrap();
    assert!(child.wait().await.unwrap().success());
    assert_eq!(
        wait_without_reaping(pid).await.unwrap_err().raw_os_error(),
        Some(libc::ECHILD)
    );
}
