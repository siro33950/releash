use crate::client::{compatibility_guidance, daemon_error};
use clap::Subcommand;
use connectrpc::ConnectError;
use releash_sdk::{compatibility::Compatibility, daemon, descriptor};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Subcommand, Debug)]
pub(crate) enum ServerSubcommand {
    Start,
    Stop,
    Restart,
}

pub async fn status(dir: &Path, machine: bool) -> Result<String, ConnectError> {
    let discovery = daemon::running(dir).map_err(daemon_error)?;
    let info = match &discovery {
        Some(discovery) => Some(
            daemon::server_info(discovery, &discovery.token)
                .await
                .map_err(daemon_error)?,
        ),
        None => None,
    };
    let compatibility = info
        .as_ref()
        .map(|info| Compatibility::assess(descriptor::protocol(), info.protocol));
    let uptime = info
        .as_ref()
        .map(|info| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|now| now.as_secs().saturating_sub(info.process_started_at))
                .map_err(|error| ConnectError::internal(error.to_string()))
        })
        .transpose()?;
    let server = info
        .as_ref()
        .map(|info| {
            crate::json::from_message("releash.client.v1.ServerInfo", info)
                .map(|mut value| {
                    value["protocol"] = json!(info.protocol);
                    value
                })
                .map_err(ConnectError::internal)
        })
        .transpose()?;
    let value = json!({
        "client": {"release": env!("CARGO_PKG_VERSION"), "protocol": descriptor::protocol()},
        "data_dir": dir,
        "running": info.is_some(),
        "server": server,
        "uptime_seconds": uptime,
        "compatibility": compatibility.map(|c| match c {
            Compatibility::Compatible => "compatible",
            Compatibility::ServerOlder => "server_older",
            Compatibility::ClientOlder => "client_older",
        }),
        "guidance": compatibility.map(compatibility_guidance),
        "connection": discovery.as_ref().map(|d| json!({"host": "127.0.0.1", "port": d.port})),
        "discovery_file": dir.join("client-api.json"),
    });
    if machine {
        return serde_json::to_string_pretty(&value)
            .map(|s| format!("{s}\n"))
            .map_err(|error| ConnectError::internal(error.to_string()));
    }
    let mut output = format!(
        "client release: {}\nclient protocol: {}\ndata dir: {}\nserver: {}\n",
        env!("CARGO_PKG_VERSION"),
        descriptor::protocol(),
        dir.display(),
        if info.is_some() {
            "running"
        } else {
            "not running"
        },
    );
    if let Some(info) = info {
        output.push_str(&format!(
            "daemon_id: {}\npid: {}\nprocess_started_at: {}\nserver release: {}\nserver protocol: {}\ncapabilities: {}\nserving status: {}\nuptime seconds: {}\ncompatibility: {}\n",
            info.daemon_id, info.pid, info.process_started_at, info.release, info.protocol,
            info.capabilities.join(", "),
            releash_sdk::wire::ServingStatus::try_from(info.serving_status)
                .map(|s| s.as_str_name()).unwrap_or("UNKNOWN"),
            uptime.unwrap_or_default(), compatibility_guidance(compatibility.unwrap()),
        ));
    }
    Ok(output)
}

fn executable() -> Result<PathBuf, ConnectError> {
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|error| ConnectError::unavailable(error.to_string()))
}

async fn start(dir: &Path) -> Result<bool, ConnectError> {
    if daemon::running(dir).map_err(daemon_error)?.is_some() {
        return Ok(false);
    }
    let executable = executable()?.with_file_name("releashd");
    let cwd =
        std::env::current_dir().map_err(|error| ConnectError::unavailable(error.to_string()))?;
    daemon::start(&executable, dir, &cwd)
        .await
        .map_err(daemon_error)?;
    Ok(true)
}

pub async fn run(dir: &Path, command: ServerSubcommand) -> Result<String, ConnectError> {
    match command {
        ServerSubcommand::Start => Ok(if start(dir).await? {
            "server started\n"
        } else {
            "server is already running\n"
        }
        .into()),
        ServerSubcommand::Stop => {
            let discovery = daemon::running(dir)
                .map_err(daemon_error)?
                .ok_or_else(|| ConnectError::unavailable("server is not running"))?;
            daemon::stop(dir, &discovery).await.map_err(daemon_error)?;
            Ok("server stopped\n".into())
        }
        ServerSubcommand::Restart => {
            let discovery = daemon::running(dir).map_err(daemon_error)?;
            if let Some(discovery) = &discovery {
                daemon::stop(dir, discovery).await.map_err(daemon_error)?;
            }
            start(dir).await?;
            Ok(if discovery.is_some() {
                "server stopped and started\n"
            } else {
                "server was not running; started\n"
            }
            .into())
        }
    }
}

fn app_bundle(executable: &Path) -> Option<&Path> {
    executable
        .ancestors()
        .skip(1)
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
}

pub async fn launch(dir: &Path) -> Result<String, ConnectError> {
    start(dir).await?;
    let executable = executable()?;
    if let Some(app) = app_bundle(&executable) {
        open_app(app)?;
        Ok(String::new())
    } else {
        status(dir, false).await
    }
}

#[cfg(target_os = "macos")]
fn open_app(app: &Path) -> Result<(), ConnectError> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg(app)
        .status()
        .map_err(|error| ConnectError::unavailable(error.to_string()))?;
    if !status.success() {
        return Err(ConnectError::unavailable(format!(
            "could not open {}: {status}",
            app.display()
        )));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn open_app(_app: &Path) -> Result<(), ConnectError> {
    Err(ConnectError::unavailable("opening an .app requires macOS"))
}

#[cfg(test)]
#[path = "server_test.rs"]
mod server_tests;
