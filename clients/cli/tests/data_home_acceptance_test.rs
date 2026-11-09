#![cfg(target_os = "macos")]

use releash::discovery;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

struct Home {
    root: tempfile::TempDir,
    current: PathBuf,
    legacy: PathBuf,
}

impl Home {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("Library/Application Support");
        let suffix = if cfg!(debug_assertions) { ".dev" } else { "" };
        Self {
            current: base.join(format!("releash{suffix}")),
            legacy: base.join(format!("com.releash.app{suffix}")),
            root,
        }
    }

    fn command(&self, explicit: Option<&Path>, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_releash"));
        command
            .env("HOME", self.root.path())
            .env_remove("RELEASH_DATA_DIR")
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("CLAUDE_CONFIG_DIR", self.root.path().join("claude"))
            .env("CODEX_HOME", self.root.path().join("codex"))
            .env("RELEASH_WORKFLOWS_DIR", self.current.join("workflows"))
            .env("SHELL", "/bin/sh")
            .current_dir(self.root.path());
        if let Some(path) = explicit {
            command.arg("--data-dir").arg(path);
        }
        command.args(args);
        command
    }

    fn run(&self, explicit: Option<&Path>, args: &[&str]) -> Output {
        self.command(explicit, args).output().unwrap()
    }

    fn seed_legacy(&self) {
        success(self.run(Some(&self.legacy), &["server", "start"]));
        success(self.run(Some(&self.legacy), &["server", "stop"]));
        self.write(
            &self.legacy,
            "logs/migration-sentinel.log",
            b"retained log\n",
        );
        self.write(&self.legacy, "other/nested.bin", b"\0retained\xff");
    }

    fn write(&self, directory: &Path, relative: &str, bytes: &[u8]) {
        let path = directory.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn status(&self) -> Value {
        serde_json::from_slice(&success(self.run(None, &["status", "--json"]))).unwrap()
    }

    fn servers(&self) -> Vec<(u32, u64)> {
        let executable = Path::new(env!("CARGO_BIN_EXE_releash"))
            .with_file_name("releashd")
            .canonicalize()
            .unwrap();
        let mut home = std::ffi::OsString::from("HOME=");
        home.push(self.root.path());
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::Always)
                .with_environ(UpdateKind::Always),
        );
        system
            .processes()
            .values()
            .filter(|process| {
                process.exe() == Some(executable.as_path())
                    && process.environ().contains(&home)
                    && process.status() != ProcessStatus::Zombie
            })
            .map(|process| (process.pid().as_u32(), process.start_time()))
            .collect()
    }

    fn stop_servers(&self) {
        let servers = self.servers();
        for path in [&self.current, &self.legacy] {
            let _ = self.command(Some(path), &["server", "stop"]).output();
        }
        for (pid, started_at) in self.servers() {
            if servers.contains(&(pid, started_at))
                && discovery::process_start_time(pid) == Some(started_at)
            {
                // discovery に記録されなかった、このテストの daemon も終了させる。
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            }
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.servers().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn assert_moved(&self) {
        assert_eq!(self.status()["data_dir"], self.current.to_str().unwrap());
        assert_eq!(self.status()["running"], true);
        for (name, expected) in [
            ("logs/migration-sentinel.log", b"retained log\n".as_slice()),
            ("other/nested.bin", b"\0retained\xff".as_slice()),
        ] {
            assert_eq!(std::fs::read(self.current.join(name)).unwrap(), expected);
            assert!(!self.legacy.join(name).exists());
        }
        assert!(
            std::fs::metadata(self.current.join("local-event-store.sqlite3"))
                .unwrap()
                .len()
                > 0
        );
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        self.stop_servers();
    }
}

fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn test_b021_既定保存先_statusがビルド種別に対応した新保存先を示す() {
    // Given
    let home = Home::new();
    // When
    let status = home.status();
    // Then
    assert_eq!(status["data_dir"], home.current.to_str().unwrap());
    assert_eq!(status["running"], false);
}

#[test]
fn test_b022_旧サーバ稼働中_statusと起動は同じサーバを使いデータを移さない() {
    // Given
    let home = Home::new();
    success(home.run(Some(&home.legacy), &["server", "start"]));
    let first = discovery::read(&home.legacy).unwrap();
    home.write(&home.legacy, "sentinel", b"unchanged");
    // When
    let status = home.status();
    let started: Value =
        serde_json::from_slice(&success(home.run(None, &["server", "start", "--json"]))).unwrap();
    // Then
    assert_eq!(status["running"], true);
    assert_eq!(status["server"]["daemonId"], first.daemon_id);
    assert_eq!(started["started"], false);
    assert_eq!(discovery::read(&home.legacy).unwrap(), first);
    assert!(!home.current.join("client-api.json").exists());
    assert_eq!(
        std::fs::read(home.legacy.join("sentinel")).unwrap(),
        b"unchanged"
    );
}

#[test]
fn test_b023_b025_停止後の移行_利用者の定義と別環境を残し空ファイルを置き換える() {
    for empty_file in [false, true] {
        // Given
        let home = Home::new();
        home.seed_legacy();
        home.write(&home.current, "workflows/custom.yml", b"user workflow");
        home.write(&home.current, "prompts/custom.txt", b"user prompt");
        let other = home.legacy.with_file_name(if cfg!(debug_assertions) {
            "com.releash.app"
        } else {
            "com.releash.app.dev"
        });
        home.write(&other, "sentinel", b"other profile");
        if empty_file {
            home.write(&home.current, "local-event-store.sqlite3", b"");
        }
        // When
        success(home.run(None, &["server", "start"]));
        // Then
        home.assert_moved();
        assert_eq!(
            std::fs::read(home.current.join("workflows/custom.yml")).unwrap(),
            b"user workflow"
        );
        assert_eq!(
            std::fs::read(home.current.join("prompts/custom.txt")).unwrap(),
            b"user prompt"
        );
        assert_eq!(
            std::fs::read(other.join("sentinel")).unwrap(),
            b"other profile"
        );
        let wrapper = std::fs::read_to_string(home.current.join("bin/releash")).unwrap();
        assert!(wrapper.contains(home.current.to_str().unwrap()));
        assert!(!wrapper.contains(home.legacy.to_str().unwrap()));
    }
}

#[test]
fn test_b024_新旧にデータがある_理由付きで停止し両方のデータを保持する() {
    // Given
    let home = Home::new();
    success(home.run(Some(&home.current), &["server", "start"]));
    success(home.run(Some(&home.current), &["server", "stop"]));
    home.seed_legacy();
    let previous = std::fs::read(home.legacy.join("local-event-store.sqlite3")).unwrap();
    let current = std::fs::read(home.current.join("local-event-store.sqlite3")).unwrap();
    // When
    let output = home.run(None, &["server", "start", "--json"]);
    // Then
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Both the previous and the new data folders contain Releash data"));
    assert_eq!(
        std::fs::read(home.legacy.join("local-event-store.sqlite3")).unwrap(),
        previous
    );
    assert_eq!(
        std::fs::read(home.current.join("local-event-store.sqlite3")).unwrap(),
        current
    );
    assert!(!home.current.join("client-api.json").exists());
}

#[test]
fn test_b026_並行起動_移行したデータを保ち一つのサーバだけが起動する() {
    // Given
    let home = Home::new();
    home.seed_legacy();
    // When
    let first = home
        .command(None, &["server", "start", "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let second = home
        .command(None, &["server", "start", "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let outputs = [
        first.wait_with_output().unwrap(),
        second.wait_with_output().unwrap(),
    ];
    // Then
    assert!(
        outputs.iter().any(|output| output.status.success()),
        "{outputs:?}"
    );
    assert!(!home.legacy.join("client-api.json").exists());
    home.assert_moved();
    let running = discovery::read(&home.current).unwrap();
    assert_eq!(
        home.servers(),
        vec![(running.pid, running.process_started_at)]
    );
    success(home.run(None, &["server", "start"]));
    assert_eq!(discovery::read(&home.current).unwrap(), running);
    assert_eq!(
        home.servers(),
        vec![(running.pid, running.process_started_at)]
    );
}

#[test]
fn test_サーバ追跡_discoveryに記録されていないサーバも検出して終了する() {
    let home = Home::new();
    success(home.run(Some(&home.current), &["server", "start"]));
    success(home.run(Some(&home.legacy), &["server", "start"]));
    std::fs::remove_file(home.legacy.join("client-api.json")).unwrap();
    assert_eq!(home.servers().len(), 2);

    home.stop_servers();

    assert!(home.servers().is_empty());
}

#[test]
fn test_b027_旧サーバの記録を検証できない_移行せずデータを保持する() {
    // Given
    let home = Home::new();
    home.write(
        &home.legacy,
        "local-event-store.sqlite3",
        b"legacy database",
    );
    home.write(&home.legacy, "client-api.json", b"{unreadable discovery");
    // When
    let output = home.run(None, &["server", "start", "--json"]);
    // Then
    assert!(!output.status.success());
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("client discovery is invalid"));
    assert_eq!(
        std::fs::read(home.legacy.join("local-event-store.sqlite3")).unwrap(),
        b"legacy database"
    );
    assert_eq!(
        std::fs::read(home.legacy.join("client-api.json")).unwrap(),
        b"{unreadable discovery"
    );
    assert!(!home.current.join("local-event-store.sqlite3").exists());
    assert!(!home.current.join("client-api.json").exists());
}

#[test]
fn test_移行の排他_discoveryのない旧writerを待ち移行後も旧writerを拒否する() {
    use std::os::fd::AsRawFd;
    // Given
    let home = Home::new();
    home.seed_legacy();
    let writer = std::fs::OpenOptions::new()
        .write(true)
        .open(home.legacy.join("local-event-store.lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(writer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    // When / Then
    assert!(!home.run(None, &["server", "start"]).status.success());
    assert_eq!(
        std::fs::read(home.legacy.join("other/nested.bin")).unwrap(),
        b"\0retained\xff"
    );
    assert_eq!(unsafe { libc::flock(writer.as_raw_fd(), libc::LOCK_UN) }, 0);
    success(home.run(None, &["server", "start"]));
    home.assert_moved();
    assert_eq!(
        unsafe { libc::flock(writer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().kind(),
        std::io::ErrorKind::WouldBlock
    );
    success(home.run(None, &["server", "stop"]));
    assert_eq!(
        unsafe { libc::flock(writer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
}
