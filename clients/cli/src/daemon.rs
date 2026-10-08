use crate::{descriptor, discovery, rpc, wire};
use connectrpc::client::{ClientConfig, HttpClient};
use discovery::LocalApiDiscovery;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Discovery(#[from] discovery::DiscoveryReadError),
    #[error(transparent)]
    Connect(#[from] connectrpc::ConnectError),
    #[error("サーバのプロセスが終了しました ({status})\n{stderr}")]
    Exited {
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("サーバの起動を確認できませんでした\n{stderr}")]
    StartupTimeout { stderr: String },
    #[error("サーバの停止を確認できませんでした")]
    ShutdownTimeout,
}

pub fn timeout(name: &str) -> Duration {
    Duration::from_millis(
        descriptor::option(&descriptor::client_service().options(), name)
            .as_u32()
            .expect("service timeout")
            .into(),
    )
}

pub fn client(discovery: &LocalApiDiscovery, token: &str) -> rpc::ClientServiceClient<HttpClient> {
    rpc::ClientServiceClient::new(
        HttpClient::plaintext(),
        ClientConfig::new(
            format!("http://127.0.0.1:{}", discovery.port)
                .parse()
                .expect("loopback URL"),
        )
        .with_default_header("authorization", format!("Bearer {token}"))
        .with_default_timeout(timeout("default_timeout_ms")),
    )
}

pub async fn server_info(
    discovery: &LocalApiDiscovery,
    token: &str,
) -> Result<wire::ServerInfo, DaemonError> {
    let response = client(discovery, token)
        .get_server_info(rpc::Unit::default())
        .await?;
    let info = <wire::ServerInfo as prost::Message>::decode(
        buffa::Message::encode_to_vec(&response.into_owned()).as_slice(),
    )
    .map_err(|error| connectrpc::ConnectError::internal(error.to_string()))?;
    discovery.verify_server(&info)?;
    Ok(info)
}

pub fn running(data_dir: &Path) -> Result<Option<LocalApiDiscovery>, DaemonError> {
    let Some(discovery) = discovery::read_optional(data_dir)? else {
        return Ok(None);
    };
    let process = discovery::lookup_process_start_time(discovery.pid);
    if process.process_list_available && process.start_time != Some(discovery.process_started_at) {
        return Ok(None);
    }
    discovery.verify_process(|_| process)?;
    Ok(Some(discovery))
}

pub fn current_executable() -> Result<PathBuf, std::io::Error> {
    std::env::current_exe()?.canonicalize()
}

pub fn executable(current_executable: &Path) -> PathBuf {
    current_executable.with_file_name("releashd")
}

pub async fn start(
    executable: &Path,
    data_dir: &Path,
    cwd: &Path,
) -> Result<LocalApiDiscovery, DaemonError> {
    std::fs::create_dir_all(data_dir.join("logs"))?;
    let log = data_dir.join("logs/daemon-startup.stderr");
    let stderr = std::fs::File::create(&log)?;
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("--data-dir")
        .arg(data_dir)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let deadline = tokio::time::Instant::now() + timeout("min_connect_timeout_ms");
    loop {
        if let Some(discovery) = running(data_dir)? {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            return Ok(discovery);
        }
        if let Some(status) = child.try_wait()? {
            return Err(DaemonError::Exited {
                status,
                stderr: stderr_tail(&log)?,
            });
        }
        if tokio::time::Instant::now() >= deadline {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            return Err(DaemonError::StartupTimeout {
                stderr: stderr_tail(&log)?,
            });
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn stop(data_dir: &Path, discovery: &LocalApiDiscovery) -> Result<(), DaemonError> {
    tokio::time::timeout(timeout("shutdown_timeout_ms"), async {
        discovery.verify_process(discovery::lookup_process_start_time)?;
        server_info(discovery, &discovery.token).await?;
        client(discovery, &discovery.token)
            .stop_daemon(rpc::StopDaemonRequest::default())
            .await?;
        loop {
            let process = discovery::lookup_process_start_time(discovery.pid);
            let exited = process.process_list_available
                && process.start_time != Some(discovery.process_started_at);
            if exited && discovery::read_optional(data_dir)?.is_none() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| DaemonError::ShutdownTimeout)?
}

fn stderr_tail(path: &Path) -> Result<String, std::io::Error> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(file.metadata()?.len().saturating_sub(8192)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
#[path = "daemon_test.rs"]
mod daemon_tests;
