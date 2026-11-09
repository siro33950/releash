#![cfg(target_os = "macos")]

mod support;

use releash_desktop::test_support::integration::daemon_connection::{
    DaemonConnectionUsecase, DaemonServiceGateway, Deadline, RetryLimiter,
};
use std::{os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc};

struct StopDaemon(PathBuf);

impl Drop for StopDaemon {
    fn drop(&mut self) {
        let _ = std::process::Command::new(support::backend_executable().with_file_name("releash"))
            .arg("--data-dir")
            .arg(&self.0)
            .args(["server", "stop"])
            .output();
    }
}

#[tokio::test]
async fn test_移行前の起動失敗_ログを保留し再試行で移行と接続を完了する() {
    // Given
    let home = tempfile::tempdir().unwrap();
    let base = home.path().join("Library/Application Support");
    let suffix = if cfg!(debug_assertions) { ".dev" } else { "" };
    let current = base.join(format!("releash{suffix}"));
    let legacy = base.join(format!("com.releash.app{suffix}"));
    let _stop = StopDaemon(current.clone());
    std::fs::create_dir_all(legacy.join("logs")).unwrap();
    let previous = "{\"message\":\"legacy log\"}\n";
    std::fs::write(legacy.join("logs/releash.log"), previous).unwrap();
    let executable = home.path().join("start-daemon");
    std::fs::write(&executable, "#!/bin/sh\nexit 7\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    releashd::desktop_api::init_deferred_local_log(releashd::desktop_api::LocalLogProcess::Gui)
        .unwrap();
    let gateway = Arc::new(DaemonServiceGateway::new(
        executable.clone(),
        releash::data_dir::DataDir {
            path: current.clone(),
            legacy: Some(legacy.clone()),
        },
        Arc::new(RetryLimiter::new()),
        Deadline(releash::daemon::timeout("min_connect_timeout_ms")),
        tokio::sync::watch::channel(None).0,
    ));
    let connection = DaemonConnectionUsecase::new(
        gateway.clone(),
        gateway,
        releash::descriptor::protocol(),
        env!("CARGO_PKG_VERSION").into(),
    );
    // When
    assert!(connection.connect().await.is_err());
    log::error!("startup failure before migration");
    log::logger().flush();
    assert!(!current.join("logs").exists());
    assert_eq!(
        std::fs::read_to_string(legacy.join("logs/releash.log")).unwrap(),
        previous
    );
    let quote =
        |path: &std::path::Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
    std::fs::write(&executable, format!(
        "#!/bin/sh\nexport HOME={}\nexport XDG_CONFIG_HOME=\"$HOME/config\"\nexport CLAUDE_CONFIG_DIR=\"$HOME/claude\"\nexport CODEX_HOME=\"$HOME/codex\"\nexport SHELL=/bin/sh\nunset RELEASH_DATA_DIR RELEASH_WORKFLOWS_DIR\ncd \"$HOME\"\nexec {} \"$@\"\n",
        quote(home.path()), quote(&support::backend_executable()),
    )).unwrap();
    connection.connect().await.unwrap();
    log::logger().flush();
    // Then
    assert!(!legacy.join("logs/releash.log").exists());
    let log = std::fs::read_to_string(current.join("logs/releash.log")).unwrap();
    assert!(log.contains("legacy log"));
    assert!(log.contains("startup failure before migration"));
    assert!(current.join("client-api.json").exists());
    connection.stop().await.unwrap();
}
