use crate::client::{compatibility_guidance, daemon_error, startup_guidance};
use crate::{compatibility::Compatibility, daemon, data_dir, descriptor, discovery};
use clap::Subcommand;
use connectrpc::ConnectError;
use serde_json::json;
use std::path::Path;

#[derive(Subcommand, Debug)]
pub(crate) enum ServerSubcommand {
    Start {
        #[arg(long)]
        json: bool,
    },
    Stop,
    Restart,
}

pub async fn status(dir: &Path, machine: bool) -> Result<String, ConnectError> {
    let discovery = daemon::running(dir).map_err(daemon_error)?;
    struct RunningStatus {
        info: crate::wire::ServerInfo,
        compatibility: Compatibility,
        uptime: u64,
        server: serde_json::Value,
    }
    let running = match &discovery {
        Some(discovery) => {
            let info = daemon::server_info(discovery, &discovery.token)
                .await
                .map_err(daemon_error)?;
            let compatibility = Compatibility::assess(descriptor::protocol(), info.protocol);
            let uptime = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|now| now.as_secs().saturating_sub(info.process_started_at))
                .map_err(|error| ConnectError::internal(error.to_string()))?;
            let mut server = crate::json::from_message("releash.client.v1.ServerInfo", &info)
                .map_err(ConnectError::internal)?;
            server["protocol"] = json!(info.protocol);
            Some(RunningStatus {
                info,
                compatibility,
                uptime,
                server,
            })
        }
        None => None,
    };
    let value = json!({
        "client": {"release": env!("CARGO_PKG_VERSION"), "protocol": descriptor::protocol()},
        "data_dir": dir,
        "running": running.is_some(),
        "server": running.as_ref().map(|s| &s.server),
        "uptime_seconds": running.as_ref().map(|s| s.uptime),
        "compatibility": running.as_ref().map(|s| match s.compatibility {
            Compatibility::Compatible => "compatible",
            Compatibility::ServerOlder => "server_older",
            Compatibility::ClientOlder => "client_older",
        }),
        "guidance": running.as_ref().map(|s| compatibility_guidance(s.compatibility)),
        "startup_guidance": running.is_none().then(startup_guidance),
        "connection": discovery.as_ref().map(|d| json!({"host": "127.0.0.1", "port": d.port})),
        "discovery_file": discovery::discovery_file(dir),
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
        if running.is_some() {
            "running"
        } else {
            "not running"
        },
    );
    if let Some(running) = running {
        let info = running.info;
        output.push_str(&format!(
            "daemon_id: {}\npid: {}\nprocess_started_at: {}\nserver release: {}\nserver protocol: {}\ncapabilities: {}\nserving status: {}\nuptime seconds: {}\ncompatibility: {}\n",
            info.daemon_id, info.pid, info.process_started_at, info.release, info.protocol,
            info.capabilities.join(", "),
            crate::wire::ServingStatus::try_from(info.serving_status)
                .map_or("UNKNOWN", |s| s.as_str_name()),
            running.uptime, compatibility_guidance(running.compatibility),
        ));
    }
    Ok(output)
}

fn current_executable() -> Result<std::path::PathBuf, ConnectError> {
    daemon::current_executable().map_err(|error| ConnectError::unavailable(error.to_string()))
}

async fn start(dir: &Path) -> Result<bool, ConnectError> {
    if daemon::running(dir).map_err(daemon_error)?.is_some() {
        return Ok(false);
    }
    let executable = daemon::executable(&current_executable()?);
    let cwd =
        std::env::current_dir().map_err(|error| ConnectError::unavailable(error.to_string()))?;
    daemon::start(&executable, dir, &cwd)
        .await
        .map_err(daemon_error)?;
    Ok(true)
}

pub async fn run(dir: &Path, command: ServerSubcommand) -> Result<String, ConnectError> {
    match command {
        ServerSubcommand::Start { json } => Ok(start_output(start(dir).await?, json)),
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

fn start_output(started: bool, machine: bool) -> String {
    if machine {
        return format!("{}\n", json!({"started": started}));
    }
    if started {
        "server started\n"
    } else {
        "server is already running\n"
    }
    .into()
}

fn app_bundle(executable: &Path) -> Option<&Path> {
    executable
        .ancestors()
        .skip(1)
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
}

pub async fn launch(dir: &Path) -> Result<String, ConnectError> {
    start(dir).await?;
    let executable = current_executable()?;
    launch_started(dir, &executable, open_app).await
}

async fn launch_started(
    dir: &Path,
    executable: &Path,
    open_app: impl FnOnce(&Path) -> Result<(), ConnectError>,
) -> Result<String, ConnectError> {
    if let Some(app) = app_bundle(executable) {
        let resolved = dir
            .canonicalize()
            .map_err(|error| ConnectError::unavailable(error.to_string()))?;
        let default = data_dir::default_data_dir_for_profile(data_dir::BuildProfile::current())
            .and_then(|path| path.canonicalize().ok());
        if default.as_ref() == Some(&resolved) {
            open_app(app)?;
            return Ok(String::new());
        }
        return Ok(format!(
            "app was not opened: data dir differs from the desktop default\n{}",
            status(dir, false).await?,
        ));
    }
    status(dir, false).await
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
