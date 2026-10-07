#[cfg(all(debug_assertions, target_os = "macos"))]
mod acceptance {
    use releash_desktop::test_support as host;
    use releashd::test_support::client_api_acceptance as api;
    use serde_json::json;
    use std::{
        path::{Path, PathBuf},
        process::Command,
        time::Duration,
    };

    fn child(path: &Path, entry: &str) {
        let app = host::desktop_connection_app(
            tauri::Builder::default(),
            path,
            &super::support::backend_executable(),
        );
        tauri::async_runtime::block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while host::desktop_connection_status(app.handle())["phase"] != "ready" {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
        });
        let window = tauri::WebviewWindowBuilder::new(
            &app,
            "startup-failure",
            tauri::WebviewUrl::App("index.html".into()),
        )
        .visible(false)
        .build()
        .unwrap();
        host::install_desktop_native_quit(app.handle());
        let entry = entry.to_string();
        let handle = app.handle().clone();
        let quitting = handle.clone();
        handle
            .run_on_main_thread(move || {
                std::fs::write(
                    std::env::var("RELEASH_QUIT_TEST_EXIT").unwrap(),
                    "requested",
                )
                .unwrap();
                match entry.as_str() {
                    "tray" => host::dispatch_desktop_tray_quit(&quitting),
                    "os" => unsafe {
                        use objc2::{
                            msg_send,
                            runtime::{AnyClass, AnyObject},
                        };
                        let application: *mut AnyObject =
                            msg_send![AnyClass::get(c"NSApplication").unwrap(), sharedApplication];
                        let _: () =
                            msg_send![application, terminate: std::ptr::null::<AnyObject>()];
                    },
                    "failure" => {
                        assert_eq!(window.label(), "startup-failure");
                        host::quit_desktop(quitting);
                    }
                    _ => panic!("unknown quit entry"),
                }
            })
            .unwrap();
        app.run_return(|_, _| {});
    }

    async fn wait_session(client: &api::NativeClient, node: &str) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let target = format!("agent-session:{}:{node}", node.len());
                let state = api::read_state(client, &target).await.unwrap();
                if state["id"] == node {
                    assert_eq!(state["lifecycle"], "open");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    struct DaemonGuard(releash_sdk::discovery::LocalApiDiscovery);
    impl Drop for DaemonGuard {
        fn drop(&mut self) {
            if releash_sdk::discovery::process_start_time(self.0.pid)
                == Some(self.0.process_started_at)
            {
                unsafe {
                    libc::kill(self.0.pid as i32, libc::SIGTERM);
                }
            }
        }
    }

    pub fn run() {
        if let Ok(path) = std::env::var("RELEASH_QUIT_TEST_CHILD") {
            child(
                Path::new(&path),
                &std::env::var("RELEASH_QUIT_TEST_ENTRY").unwrap(),
            );
            return;
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            use std::os::unix::fs::PermissionsExt;
            // Given
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().canonicalize().unwrap();
            let repo = root.join("repository");
            let repository = git2::Repository::init(&repo).unwrap();
            let signature = git2::Signature::now("quit test", "quit@example.test").unwrap();
            let tree = repository.index().unwrap().write_tree().unwrap();
            repository.commit(Some("HEAD"), &signature, &signature, "initial", &repository.find_tree(tree).unwrap(), &[]).unwrap();
            for (key, path) in [("HOME", root.clone()), ("XDG_CONFIG_HOME", root.join("config")), ("CLAUDE_CONFIG_DIR", root.join(".claude")), ("CODEX_HOME", root.join(".codex"))] { std::env::set_var(key, path); }
            std::env::set_var("SHELL", "/bin/sh");
            let fixture = root.join("provider");
            let marker = root.join("agent-pid");
            std::fs::write(&fixture, format!("#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'claude 1.0.0'; exit; fi\necho $$ > '{}'\nexec sleep 120\n", marker.display())).unwrap();
            std::fs::set_permissions(&fixture, std::fs::Permissions::from_mode(0o755)).unwrap();
            let discovery = releash_sdk::daemon::start(&super::support::backend_executable(), &root, &repo).await.unwrap();
            let _daemon = DaemonGuard(discovery.clone());
            let endpoint = releashd::desktop_api::ClientEndpoint { url: format!("http://127.0.0.1:{}", discovery.port), token: discovery.token.clone() };
            let client = api::connect_client(&endpoint);
            api::request_client(&client, "add_repo_path", json!({"path": repo})).await.unwrap();
            api::request_client(&client, "update_provider_executable", json!({"provider":"claude", "executable": fixture})).await.unwrap();
            let selected = api::request_client(&client, "create_agent_session", json!({"workspaceIdentity":repo, "worktreePath":repo, "provider":"claude", "rows":24, "cols":80, "callerRequestId":uuid::Uuid::new_v4().to_string()})).await.unwrap();
            let node = selected["agentSessionId"].as_str().unwrap();
            let pid = tokio::time::timeout(Duration::from_secs(10), async {
                loop { if let Ok(pid) = std::fs::read_to_string(&marker) { if let Ok(pid) = pid.trim().parse::<i32>() { break pid; } } tokio::time::sleep(Duration::from_millis(20)).await; }
            }).await.unwrap();
            let cli: PathBuf = super::support::backend_executable().with_file_name("releash");
            wait_session(&client, node).await;
            for entry in ["tray", "os", "failure"] {
                // When: run the actual shell quit entry in its own native UI process.
                let exited = root.join(format!("{entry}-exit"));
                let mut ui = tokio::process::Command::new(std::env::current_exe().unwrap())
                    .env("RELEASH_QUIT_TEST_CHILD", &root).env("RELEASH_QUIT_TEST_ENTRY", entry).env("RELEASH_QUIT_TEST_EXIT", &exited)
                    .kill_on_drop(true).spawn().unwrap();
                let status = tokio::time::timeout(Duration::from_secs(15), ui.wait()).await.unwrap().unwrap();
                // Then
                assert!(status.success(), "{entry}: {status}");
                assert_eq!(std::fs::read_to_string(exited).unwrap(), "requested");
                assert_eq!(releash_sdk::discovery::read(&root).unwrap(), discovery);
                assert_eq!(unsafe { libc::kill(discovery.pid as i32, 0) }, 0);
                assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
                wait_session(&client, node).await;
                let output = Command::new(&cli).args(["--data-dir", root.to_str().unwrap(), "workflow", "diagnostics", "--json"]).output().unwrap();
                assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            }
            releash_sdk::daemon::stop(&root, &discovery).await.unwrap();
        });
    }
}

#[cfg(all(debug_assertions, target_os = "macos"))]
mod support;
fn main() {
    #[cfg(all(debug_assertions, target_os = "macos"))]
    acceptance::run();
}
