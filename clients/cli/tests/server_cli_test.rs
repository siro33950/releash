#![cfg(unix)]
use releash::discovery;
use std::path::Path;
use std::process::{Command, Output};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_releash"))
        .arg("--data-dir")
        .arg(root.join("data"))
        .args(args)
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("CLAUDE_CONFIG_DIR", root.join("claude"))
        .env("CODEX_HOME", root.join("codex"))
        .env("SHELL", "/bin/sh")
        .current_dir(root)
        .output()
        .unwrap()
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
struct RunningServer(std::path::PathBuf);
impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_releash"))
            .arg("--data-dir")
            .arg(&self.0)
            .args(["server", "stop"])
            .output();
    }
}

#[test]
fn test_cli_起動後も独立して稼働し重複起動せず再起動して停止する() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let _cleanup = RunningServer(root.join("data"));
    assert!(success(run(root, &["status", "--json"])).contains("\"running\": false"));
    // When / Then
    assert!(success(run(root, &["server", "start"])).contains("server started"));
    let first = discovery::read(&root.join("data")).unwrap();
    assert_eq!(
        discovery::process_start_time(first.pid),
        Some(first.process_started_at)
    );
    assert!(success(run(root, &["server", "start"])).contains("already running"));
    assert_eq!(discovery::read(&root.join("data")).unwrap(), first);
    let standalone = root.join("releash");
    std::fs::copy(env!("CARGO_BIN_EXE_releash"), &standalone).unwrap();
    assert_eq!(
        success(
            Command::new(standalone)
                .arg("--data-dir")
                .arg(root.join("data"))
                .args(["server", "start"])
                .output()
                .unwrap()
        ),
        "server is already running\n"
    );
    assert_eq!(discovery::read(&root.join("data")).unwrap(), first);
    let discovery_path = root.join("data/client-api.json");
    let mut legacy = serde_json::to_value(&first).unwrap();
    legacy["instance_id"] = legacy.as_object_mut().unwrap().remove("daemon_id").unwrap();
    legacy["process_started_at"] = serde_json::json!(first.process_started_at);
    std::fs::write(&discovery_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let status: serde_json::Value =
        serde_json::from_str(&success(run(root, &["status", "--json"]))).unwrap();
    assert_eq!(status["server"]["daemonId"], first.daemon_id);
    assert_eq!(status["server"]["pid"], first.pid);
    assert_eq!(
        status["server"]["processStartedAt"],
        first.process_started_at
    );
    assert_eq!(status["server"]["servingStatus"], "SERVING_STATUS_SERVING");
    assert!(!status.to_string().contains(&first.token));
    std::fs::write(&discovery_path, serde_json::to_vec(&first).unwrap()).unwrap();
    assert!(success(run(root, &["server", "restart"])).contains("stopped and started"));
    let second = discovery::read(&root.join("data")).unwrap();
    assert_ne!(second.daemon_id, first.daemon_id);
    assert_ne!(
        discovery::process_start_time(first.pid),
        Some(first.process_started_at)
    );
    assert!(success(run(root, &["server", "stop"])).contains("server stopped"));
    assert!(!root.join("data/client-api.json").exists());
    assert_ne!(
        discovery::process_start_time(second.pid),
        Some(second.process_started_at)
    );
    assert_eq!(run(root, &["server", "stop"]).status.code(), Some(1));
    assert!(success(run(root, &["server", "restart"])).contains("was not running; started"));
    success(run(root, &["server", "stop"]));
    let output = success(run(root, &[]));
    assert!(output.contains("server: running"));
    assert!(output.contains("compatibility: compatible"));
    assert!(root.join("data/client-api.json").exists());
}

#[test]
fn test_cli_同梱サーバの起動失敗は終了状態とstderrを表示する() {
    use std::os::unix::fs::PermissionsExt;
    // Given
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("releash");
    std::fs::copy(env!("CARGO_BIN_EXE_releash"), &cli).unwrap();
    let daemon = dir.path().join("releashd");
    std::fs::write(&daemon, "#!/bin/sh\nprintf 'startup failure' >&2\nexit 7\n").unwrap();
    std::fs::set_permissions(&daemon, std::fs::Permissions::from_mode(0o700)).unwrap();
    // When
    let output = Command::new(cli)
        .arg("--data-dir")
        .arg(dir.path().join("data"))
        .args(["server", "start"])
        .output()
        .unwrap();
    // Then
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("7"));
    assert!(error.contains("startup failure"));
    assert!(error.contains("終了"));
}

#[cfg(target_os = "macos")]
#[test]
fn test_cli_app内の引数なし実行_symlink経由でも隣のサーバ起動後に実体のappを開く() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::time::{Duration, Instant};
    for (through_symlink, custom_data_dir) in [(false, false), (true, false), (false, true)] {
        // Given
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let data_dir = if custom_data_dir {
            root.join("data")
        } else {
            root.join("Library/Application Support").join(
                releash::data_dir::default_data_dir_name_for_profile(
                    releash::data_dir::BuildProfile::current(),
                ),
            )
        };
        let _cleanup = RunningServer(data_dir.clone());
        let app = root.join("Launch Target.app");
        let macos = app.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let cli = macos.join("releash");
        std::fs::copy(env!("CARGO_BIN_EXE_releash"), &cli).unwrap();
        let backend = Path::new(env!("CARGO_BIN_EXE_releash")).with_file_name("releashd");
        assert!(
            backend.is_file(),
            "build releashd before running CLI integration tests"
        );
        symlink(backend, macos.join("daemon-bin")).unwrap();
        let daemon = macos.join("releashd");
        std::fs::write(&daemon, "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$0\" > \"${0%/*}/started-daemon\"\nexec \"${0%/*}/daemon-bin\" \"$@\"\n").unwrap();
        std::fs::set_permissions(&daemon, std::fs::Permissions::from_mode(0o700)).unwrap();
        let app_executable = macos.join("record-launch");
        std::fs::write(&app_executable, "#!/bin/sh\nset -eu\ntest -s \"${0%/*}/started-daemon\"\n/bin/cp \"$(/bin/cat \"${0%/*}/discovery-path\")\" \"${0%/*}/opened-discovery.json\"\nprintf '%s\\n' \"$0\" > \"${0%/*}/opened-executable.tmp\"\n/bin/mv \"${0%/*}/opened-executable.tmp\" \"${0%/*}/opened-executable\"\n").unwrap();
        std::fs::set_permissions(&app_executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.releash.test.launch.{}</string>
<key>CFBundleName</key><string>Launch Target</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleExecutable</key><string>record-launch</string>
<key>LSUIElement</key><true/>
</dict></plist>
"#, uuid::Uuid::new_v4().simple())).unwrap();
        std::fs::write(
            macos.join("discovery-path"),
            discovery::discovery_file(&data_dir).to_str().unwrap(),
        )
        .unwrap();
        let invocation = if through_symlink {
            let link_directory = root.join("Link Location.app/Contents/MacOS");
            std::fs::create_dir_all(&link_directory).unwrap();
            let link = link_directory.join("releash");
            symlink(&cli, &link).unwrap();
            link
        } else {
            cli
        };
        assert!(discovery::read_optional(&data_dir).unwrap().is_none());
        let invocation_data_dir = if through_symlink {
            std::fs::create_dir_all(&data_dir).unwrap();
            let alias = root.join("data-alias");
            symlink(&data_dir, &alias).unwrap();
            alias
        } else {
            data_dir.clone()
        };
        // When
        let output = Command::new(invocation)
            .arg("--data-dir")
            .arg(&invocation_data_dir)
            .env("HOME", &root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("CLAUDE_CONFIG_DIR", root.join("claude"))
            .env("CODEX_HOME", root.join("codex"))
            .env("SHELL", "/bin/sh")
            .current_dir(&root)
            .output()
            .unwrap();
        // Then
        let output = success(output);
        if custom_data_dir {
            assert!(output
                .starts_with("app was not opened: data dir differs from the desktop default\n"));
            assert!(output.contains("server: running"));
            assert!(output.contains("compatibility: compatible"));
            let observed = discovery::read(&data_dir).unwrap();
            assert_eq!(
                discovery::process_start_time(observed.pid),
                Some(observed.process_started_at)
            );
            assert!(!macos.join("opened-executable").exists());
            continue;
        }
        assert!(output.is_empty());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !macos.join("opened-executable").exists() {
            assert!(
                Instant::now() < deadline,
                "app executable was not launched (symlink={through_symlink})"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read_to_string(macos.join("started-daemon"))
                .unwrap()
                .trim(),
            daemon.to_str().unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(macos.join("opened-executable"))
                .unwrap()
                .trim(),
            app_executable.to_str().unwrap()
        );
        let observed: discovery::LocalApiDiscovery =
            serde_json::from_slice(&std::fs::read(macos.join("opened-discovery.json")).unwrap())
                .unwrap();
        assert_eq!(observed, discovery::read(&data_dir).unwrap());
        assert_eq!(
            discovery::process_start_time(observed.pid),
            Some(observed.process_started_at)
        );
    }
}
