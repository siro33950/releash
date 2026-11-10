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
}

impl Home {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("Library/Application Support");
        let suffix = if cfg!(debug_assertions) { ".dev" } else { "" };
        Self {
            current: base.join(format!("releash{suffix}")),
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
        let _ = self
            .command(Some(&self.current), &["server", "stop"])
            .output();
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
fn test_b026_並行起動_一つのサーバだけが起動する() {
    // Given
    let home = Home::new();
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
