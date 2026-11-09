#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::process::{Command, Output};

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

    fn command(&self) -> Command {
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
        command
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self
            .command()
            .arg("--data-dir")
            .arg(&self.current)
            .args(["server", "stop"])
            .output();
    }
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_保存先の明示指定_既定と実体が同じでも旧保存先を観測も移行もしない() {
    for environment in [false, true] {
        for alias in [false, true] {
            // Given
            let home = Home::new();
            std::fs::create_dir_all(&home.current).unwrap();
            std::fs::create_dir_all(&home.legacy).unwrap();
            std::fs::write(home.legacy.join("sentinel"), b"legacy data").unwrap();
            std::fs::write(home.legacy.join("client-api.json"), b"unverifiable").unwrap();
            let selected = if alias {
                let alias = home.root.path().join("alias");
                std::os::unix::fs::symlink(&home.current, &alias).unwrap();
                alias
            } else {
                home.current.clone()
            };
            // When / Then
            for args in [vec!["server", "start"], vec!["server", "restart"], vec![]] {
                let mut command = home.command();
                if environment {
                    command.env("RELEASH_DATA_DIR", &selected);
                } else {
                    command.arg("--data-dir").arg(&selected);
                }
                success(command.args(args).output().unwrap());
                assert_eq!(
                    std::fs::read(home.legacy.join("sentinel")).unwrap(),
                    b"legacy data"
                );
                assert_eq!(
                    std::fs::read(home.legacy.join("client-api.json")).unwrap(),
                    b"unverifiable"
                );
                assert!(!home.current.join("sentinel").exists());
                assert!(home.current.join("client-api.json").exists());
                success(
                    home.command()
                        .arg("--data-dir")
                        .arg(&selected)
                        .args(["server", "stop"])
                        .output()
                        .unwrap(),
                );
            }
        }
    }
}

#[test]
fn test_保存先の空環境変数_明示指定とせず旧保存先を移行する() {
    // Given
    let home = Home::new();
    std::fs::create_dir_all(&home.legacy).unwrap();
    std::fs::write(home.legacy.join("sentinel"), b"legacy data").unwrap();
    // When
    success(
        home.command()
            .env("RELEASH_DATA_DIR", "")
            .args(["server", "start"])
            .output()
            .unwrap(),
    );
    // Then
    assert_eq!(
        std::fs::read(home.current.join("sentinel")).unwrap(),
        b"legacy data"
    );
    assert!(!home.legacy.join("sentinel").exists());
}
