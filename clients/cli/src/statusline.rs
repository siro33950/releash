use connectrpc::ConnectError;
use std::io::{Read, Write};
use std::path::Path;

pub(crate) fn run(dir: &Path) -> Result<String, ConnectError> {
    let mut payload = Vec::new();
    std::io::stdin()
        .lock()
        .read_to_end(&mut payload)
        .map_err(|error| ConnectError::internal(error.to_string()))?;
    let inherited = std::env::var("RELEASH_CLAUDE_STATUSLINE").ok();
    let output = inherited
        .map(|command| execute(&command, &payload))
        .transpose();
    if let Ok(Some(output)) = &output {
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(output.as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(|error| ConnectError::internal(error.to_string()))?;
    }
    if let Err(error) = spawn_transfer(dir, &payload) {
        eprintln!("Provider usage: {error}");
    }
    output.map(|_| String::new())
}

fn spawn_transfer(dir: &Path, payload: &[u8]) -> std::io::Result<()> {
    if payload.len() > crate::commands::PROVIDER_PAYLOAD_LIMIT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Provider payload exceeds 65536 bytes",
        ));
    }
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    std::fs::create_dir_all(dir.join("logs"))?;
    let stderr = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("logs/provider-usage.stderr"))?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--data-dir")
        .arg(dir)
        .args(["hook", "forward-usage"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(stderr);
    // SAFETY: setsid is async-signal-safe and the closure only calls it.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("Missing usage stdin"))?
        .write_all(payload)
}

pub(crate) async fn forward_usage(dir: &Path) -> Result<String, ConnectError> {
    let payload = crate::commands::read_payload(std::io::stdin().lock())?;
    let token = std::env::var("RELEASH_PROVIDER_LIFECYCLE_TOKEN")
        .map_err(|_| ConnectError::unavailable("Provider hook token is required"))?;
    let client = crate::client::connect(dir, Some(token)).await?;
    let request = crate::wire::ReceiveProviderUsageRequest { payload };
    client
        .receive_provider_usage(crate::client::to_rpc::<
            crate::rpc::ReceiveProviderUsageRequest,
        >(&request)?)
        .await?;
    Ok(String::new())
}

fn execute(command: &str, payload: &[u8]) -> Result<String, ConnectError> {
    use std::process::{Command, Stdio};
    let mut child = Command::new("/bin/sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| ConnectError::internal(error.to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ConnectError::internal("Missing statusLine stdin"))?;
    let payload = payload.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&payload));
    let output = child
        .wait_with_output()
        .map_err(|error| ConnectError::internal(error.to_string()))?;
    let written = writer
        .join()
        .map_err(|_| ConnectError::internal("statusLine writer failed"))?;
    if let Err(error) = written {
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(ConnectError::internal(error.to_string()));
        }
    }
    if !output.status.success() {
        return Err(ConnectError::internal(format!(
            "statusLine exited with {}",
            output.status
        )));
    }
    String::from_utf8(output.stdout).map_err(|error| ConnectError::internal(error.to_string()))
}

#[cfg(test)]
#[path = "statusline_test.rs"]
mod statusline_tests;
