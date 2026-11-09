#![cfg(target_os = "macos")]

mod support;

use releash_desktop::test_support::integration::daemon_connection::{
    DaemonConnectionUsecase, DaemonServiceGateway, Deadline, RetryLimiter,
};
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command, sync::Arc};

fn cli(home: &Path) -> Command {
    let mut command = Command::new(support::backend_executable().with_file_name("releash"));
    command
        .env("HOME", home)
        .env_remove("RELEASH_DATA_DIR")
        .env_remove("RELEASH_WORKFLOWS_DIR")
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("CLAUDE_CONFIG_DIR", home.join("claude"))
        .env("CODEX_HOME", home.join("codex"))
        .env("SHELL", "/bin/sh")
        .current_dir(home);
    command
}

struct StopDaemons<'a>(&'a Path, &'a Path, &'a Path);

impl Drop for StopDaemons<'_> {
    fn drop(&mut self) {
        for path in [self.1, self.2] {
            let _ = cli(self.0)
                .arg("--data-dir")
                .arg(path)
                .args(["server", "stop"])
                .output();
        }
    }
}

fn success(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn test_旧保存先への接続_gui再起動とcli移行後の再接続が新保存先を使う() {
    releashd::desktop_api::init_deferred_local_log(releashd::desktop_api::LocalLogProcess::Gui)
        .unwrap();
    for migrate_with_cli in [false, true] {
        // Given
        let home = tempfile::tempdir().unwrap();
        let base = home.path().join("Library/Application Support");
        let suffix = if cfg!(debug_assertions) { ".dev" } else { "" };
        let current = base.join(format!("releash{suffix}"));
        let legacy = base.join(format!("com.releash.app{suffix}"));
        let _stop = StopDaemons(home.path(), &current, &legacy);
        success(
            cli(home.path())
                .arg("--data-dir")
                .arg(&legacy)
                .args(["server", "start"]),
        );
        let previous = releash::discovery::read(&legacy).unwrap();
        let executable = home.path().join("start-daemon");
        let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
        std::fs::write(&executable, format!(
            "#!/bin/sh\nexport HOME={}\nexport XDG_CONFIG_HOME=\"$HOME/config\"\nexport CLAUDE_CONFIG_DIR=\"$HOME/claude\"\nexport CODEX_HOME=\"$HOME/codex\"\nexport SHELL=/bin/sh\nunset RELEASH_DATA_DIR RELEASH_WORKFLOWS_DIR\ncd \"$HOME\"\nexec {} \"$@\"\n",
            quote(home.path()), quote(&support::backend_executable()),
        )).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let gateway = Arc::new(DaemonServiceGateway::new(
            executable,
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
        let (endpoint, _) = connection.connect().await.unwrap();
        assert_eq!(endpoint.token, previous.token);
        assert!(!current.join("client-api.json").exists());
        log::warn!("before migration");
        log::logger().flush();
        assert!(std::fs::read_to_string(legacy.join("logs/releash.log"))
            .unwrap()
            .contains("before migration"));
        assert!(!current.join("logs").exists());
        // When
        let endpoint = if migrate_with_cli {
            success(cli(home.path()).args(["server", "stop"]));
            success(cli(home.path()).args(["server", "start"]));
            let migrated = releash::discovery::read(&current).unwrap();
            let endpoint = connection.connect().await.unwrap().0;
            assert_eq!(endpoint.token, migrated.token);
            assert_eq!(releash::discovery::read(&current).unwrap(), migrated);
            endpoint
        } else {
            connection.replace().await.unwrap().0
        };
        log::warn!("after migration");
        log::logger().flush();
        // Then
        assert_ne!(endpoint.token, previous.token);
        assert_eq!(
            endpoint.token,
            releash::discovery::read(&current).unwrap().token
        );
        assert!(!legacy.join("client-api.json").exists());
        assert!(!legacy.join("logs/releash.log").exists());
        let logs = std::fs::read_to_string(current.join("logs/releash.log")).unwrap();
        assert!(logs.contains("before migration"));
        assert!(logs.contains("after migration"));
        connection.stop().await.unwrap();
        assert!(releash::daemon::running(&current).unwrap().is_none());
    }
}
