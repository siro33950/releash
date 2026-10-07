use prost::Message;
use releash_sdk::wire;
use std::process::Command;
use std::time::{Duration, Instant};

pub fn backend_executable() -> std::path::PathBuf {
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .join(format!("releashd{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "{} is missing; run cargo build -p releashd --bin releashd first",
        path.display()
    );
    path
}

pub struct DiagnosticsHost {
    child: std::process::Child,
    data_dir: std::path::PathBuf,
}
impl DiagnosticsHost {
    pub fn start(
        data_dir: std::path::PathBuf,
        applied_directory: std::path::PathBuf,
    ) -> Result<Self, String> {
        let config = if cfg!(target_os = "macos") {
            data_dir.join("Library/Application Support")
        } else {
            data_dir.join("config")
        };
        std::fs::create_dir_all(config.join("releash")).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(applied_directory, config.join("releash/workflows"))
            .map_err(|error| error.to_string())?;
        let child = Command::new(backend_executable())
            .env("HOME", &data_dir)
            .env("RELEASH_DATA_DIR", &data_dir)
            .env_remove("RELEASH_DAEMON_LAUNCH_ID")
            .env_remove("RELEASH_DAEMON_PARENT_PIPE")
            .env("SHELL", "/bin/sh")
            .env("XDG_CONFIG_HOME", data_dir.join("config"))
            .env("CLAUDE_CONFIG_DIR", data_dir.join(".claude"))
            .env("CODEX_HOME", data_dir.join(".codex"))
            .current_dir(&data_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())?;
        let mut host = Self { child, data_dir };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if host.data_dir.join("client-api.json").is_file() {
                return Ok(host);
            }
            if let Some(status) = host.child.try_wait().map_err(|error| error.to_string())? {
                return Err(format!("daemon exited: {status}"));
            }
            if Instant::now() >= deadline {
                return Err("daemon discovery timeout".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    pub async fn diagnose(
        &self,
        directory: Option<&std::path::Path>,
    ) -> Result<serde_json::Value, String> {
        let discovery = releash_sdk::discovery::read(&self.data_dir).map_err(|e| e.to_string())?;
        let config = connectrpc::client::ClientConfig::new(
            format!("http://127.0.0.1:{}", discovery.port)
                .parse()
                .unwrap(),
        )
        .with_default_header("authorization", format!("Bearer {}", discovery.token));
        let client = releash_sdk::rpc::ClientServiceClient::new(
            connectrpc::client::HttpClient::plaintext(),
            config,
        );
        let response = client
            .diagnose_workflow_directory(releash_sdk::rpc::DiagnoseWorkflowDirectoryRequest {
                dir: directory.map(|d| d.to_string_lossy().into_owned()),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        let report = wire::DiagnoseWorkflowDirectoryResponse::decode(
            buffa::Message::encode_to_vec(&response.into_owned()).as_slice(),
        )
        .map_err(|e| e.to_string())?
        .report
        .unwrap();
        releash::json::from_message("releash.client.v1.DiagnosticReport", &report)
    }
    pub async fn shutdown(mut self) -> Result<(), String> {
        self.child.kill().map_err(|e| e.to_string())?;
        self.child.wait().map_err(|e| e.to_string())?;
        Ok(())
    }
}
impl Drop for DiagnosticsHost {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
