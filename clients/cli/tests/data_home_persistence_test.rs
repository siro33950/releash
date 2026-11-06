#![cfg(target_os = "macos")]

use prost::Message;
use releash::{daemon, discovery, rpc, wire};
use std::path::{Path, PathBuf};
use std::process::Command;

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

    fn server(&self, directory: Option<&Path>, action: &str) {
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
        if let Some(directory) = directory {
            command.arg("--data-dir").arg(directory);
        }
        let output = command.args(["server", action]).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        for directory in [&self.current, &self.legacy] {
            if discovery::read(directory).is_ok() {
                self.server(Some(directory), "stop");
            }
        }
    }
}

async fn snapshot(directory: &Path, target: &str, args: Vec<String>) -> wire::state_payload::Value {
    let discovery = discovery::read(directory).unwrap();
    let client = daemon::client(&discovery, &discovery.token);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let client_id = uuid::Uuid::new_v4().to_string();
        let mut stream = client
            .open_state_stream(rpc::OpenStateStreamRequest {
                client_id: client_id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        loop {
            let message = stream.message().await.unwrap().unwrap().to_owned_message();
            let event = wire::StateSubscriptionEvent::decode(
                buffa::Message::encode_to_vec(&message).as_slice(),
            )
            .unwrap();
            match event.event.unwrap() {
                wire::state_subscription_event::Event::Ready(_) => {
                    client
                        .start_state_subscription(rpc::StartStateSubscriptionRequest {
                            client_id: client_id.clone(),
                            subscription_id: uuid::Uuid::new_v4().to_string(),
                            target: target.into(),
                            args: args.clone(),
                            ..Default::default()
                        })
                        .await
                        .unwrap();
                }
                wire::state_subscription_event::Event::Snapshot(payload) => {
                    return payload.value.unwrap()
                }
                wire::state_subscription_event::Event::Failure(error) => panic!("{error:?}"),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn test_保存先移行_設定とeventを保持して通常移行と途中再開後に読み出せる() {
    for resume in [false, true] {
        // Given
        let home = Home::new();
        home.server(Some(&home.legacy), "start");
        let discovery = discovery::read(&home.legacy).unwrap();
        let client = daemon::client(&discovery, &discovery.token);
        client
            .update_app_settings(rpc::UpdateAppSettingsRequest {
                app: Some(rpc::WindowSettings {
                    start_minimized: Some(true),
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            })
            .await
            .unwrap();
        client
            .save_repository_group_state(rpc::SaveRepositoryGroupStateRequest {
                repository_path: "/migration-sentinel".into(),
                collapsed: true,
                ..Default::default()
            })
            .await
            .unwrap();
        home.server(Some(&home.legacy), "stop");
        std::fs::write(home.legacy.join("sentinel"), b"retained").unwrap();
        if resume {
            std::fs::create_dir_all(&home.current).unwrap();
            std::fs::write(home.legacy.join(".relocating"), b"").unwrap();
            std::fs::rename(home.legacy.join("sentinel"), home.current.join("sentinel")).unwrap();
        }
        // When
        home.server(None, "start");
        // Then
        assert_eq!(
            std::fs::read(home.current.join("sentinel")).unwrap(),
            b"retained"
        );
        assert!(!home.legacy.exists());
        let wire::state_payload::Value::DesktopSettings(settings) =
            snapshot(&home.current, "desktop-settings", vec![]).await
        else {
            panic!("settings")
        };
        assert_eq!(settings.start_minimized, Some(true));
        let wire::state_payload::Value::RepositoryGroupState(group) = snapshot(
            &home.current,
            "repository-group-state",
            vec!["/migration-sentinel".into()],
        )
        .await
        else {
            panic!("group")
        };
        assert!(group.collapsed);
        let running = discovery::read(&home.current).unwrap();
        home.server(None, "start");
        assert_eq!(discovery::read(&home.current).unwrap(), running);
    }
}
