use crate::adaptor::controller::api::protocol::client as wire;
use crate::domain::daemon_supervision::{verify_identity, Failure, FailureStage, StopIntent};
use crate::domain::daemon_supervision::{DaemonExit, DaemonProcessPort, ShutdownResponse};
use crate::usecase::client_connection::ClientConnectionQueryService;
use crate::usecase::daemon_supervision::{DaemonConnection, DaemonGateway};
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use std::io::{BufRead, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as Frame};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Process {
    child: Child,
    launch_id: String,
    diagnostic: Arc<parking_lot::Mutex<Vec<u8>>>,
    shutdown_complete: Arc<AtomicBool>,
    readers: Vec<tokio::sync::oneshot::Receiver<()>>,
}

pub(crate) struct DaemonProcessGateway {
    origin: std::time::Instant,
    executable: PathBuf,
    data_dir: PathBuf,
    process: parking_lot::Mutex<Option<Process>>,
    connection: parking_lot::Mutex<Option<DaemonConnection>>,
    client: parking_lot::Mutex<Option<Arc<super::desktop_client::DesktopClient>>>,
    handoff: Arc<crate::usecase::client_handoff::ClientHandoffUsecase>,
}

impl DaemonProcessGateway {
    pub fn new(
        executable: PathBuf,
        data_dir: PathBuf,
        handoff: Arc<crate::usecase::client_handoff::ClientHandoffUsecase>,
    ) -> Self {
        Self {
            client: parking_lot::Mutex::new(None),
            handoff,
            origin: std::time::Instant::now(),
            executable,
            data_dir,
            process: parking_lot::Mutex::new(None),
            connection: parking_lot::Mutex::new(None),
        }
    }
    pub fn client(&self) -> Result<Arc<super::desktop_client::DesktopClient>, String> {
        self.client
            .lock()
            .clone()
            .filter(|client| client.connected())
            .ok_or_else(|| "Desktop client is unavailable".into())
    }
    async fn connect(
        &self,
    ) -> Result<
        (
            Socket,
            wire::ClientHello,
            crate::usecase::client_connection::ClientConnectionDto,
        ),
        String,
    > {
        let endpoint = super::local_api::ClientConnectionFileQuery(self.data_dir.clone())
            .read()
            .map_err(|e| e.to_string())?;
        let mut request = endpoint
            .url
            .clone()
            .into_client_request()
            .map_err(|e| e.to_string())?;
        request.headers_mut().insert(
            "sec-websocket-protocol",
            endpoint
                .auth_subprotocol
                .parse()
                .map_err(|e| format!("invalid client token: {e}"))?,
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| e.to_string())?;
        socket
            .send(Frame::Binary(
                wire::Envelope {
                    body: Some(wire::envelope::Body::Hello(wire::ClientHello::default())),
                }
                .encode_to_vec()
                .into(),
            ))
            .await
            .map_err(|e| e.to_string())?;
        loop {
            let frame = socket
                .next()
                .await
                .ok_or("Daemon connection closed")?
                .map_err(|e| e.to_string())?;
            if let Frame::Binary(bytes) = frame {
                if let Some(wire::envelope::Body::Hello(hello)) = wire::Envelope::decode(bytes)
                    .map_err(|e| e.to_string())?
                    .body
                {
                    return Ok((socket, hello, endpoint));
                }
            }
        }
    }
    async fn connect_client(&self, launch_id: &str) -> Result<Option<DaemonConnection>, Failure> {
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(500), self.connect()).await;
        let (socket, hello, endpoint) = match result {
            Ok(Ok(value)) => value,
            Ok(Err(reason)) => {
                return Err(Failure {
                    stage: FailureStage::Initialization,
                    reason,
                })
            }
            Err(_) => return Ok(None),
        };
        verify_identity(launch_id, &hello.launch_id, &hello.release)?;
        let settings = hello.desktop_settings.ok_or_else(|| Failure {
            stage: FailureStage::Initialization,
            reason: "Daemon settings are unavailable.".into(),
        })?;
        let connection = DaemonConnection {
            connected_at_ms: self.monotonic_ms(),
            endpoint,
            settings: settings.into(),
            launch_id: hello.launch_id.clone(),
            release: hello.release.clone(),
        };
        *self.client.lock() = Some(Arc::new(super::desktop_client::DesktopClient::start(
            socket,
            hello,
            self.handoff.clone(),
        )));
        *self.connection.lock() = Some(connection.clone());
        Ok(Some(connection))
    }
}

#[async_trait::async_trait]
impl DaemonProcessPort for DaemonProcessGateway {
    fn monotonic_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }
    async fn wait_for_poll(&self) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    async fn spawn(&self) -> Result<String, String> {
        let mut process = self.process.lock();
        if process.is_some() {
            return Err("Previous daemon exit has not been confirmed.".into());
        }
        let launch_id = uuid::Uuid::new_v4().to_string();
        let mut child = Command::new(&self.executable)
            .arg("--internal-daemon")
            .arg(&self.data_dir)
            .env("RELEASH_DAEMON_LAUNCH_ID", &launch_id)
            .env("RELEASH_DAEMON_PARENT_PIPE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let diagnostic = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let output = diagnostic.clone();
        let mut stderr = child.stderr.take().ok_or("Daemon stderr is missing")?;
        let (diagnostic_done, diagnostic_reader) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let mut buffer = [0; 1024];
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                let mut output = output.lock();
                output.extend_from_slice(&buffer[..count]);
                let excess = output.len().saturating_sub(8192);
                output.drain(..excess);
            }
            let _ = diagnostic_done.send(());
        });
        let shutdown_complete = Arc::new(AtomicBool::new(false));
        let completed = shutdown_complete.clone();
        let stdout = child.stdout.take().ok_or("Daemon stdout is missing")?;
        let (output_done, output_reader) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            let mut buffer = Vec::new();
            while let Ok(count) = reader.by_ref().take(1024).read_until(b'\n', &mut buffer) {
                if count == 0 {
                    break;
                }
                if buffer == b"releash-shutdown-complete\n" {
                    completed.store(true, Ordering::SeqCst);
                }
                buffer.clear();
            }
            let _ = output_done.send(());
        });
        *process = Some(Process {
            child,
            launch_id: launch_id.clone(),
            diagnostic,
            shutdown_complete,
            readers: vec![diagnostic_reader, output_reader],
        });
        *self.client.lock() = None;
        *self.connection.lock() = None;
        Ok(launch_id)
    }

    async fn exited(&self) -> Result<Option<DaemonExit>, String> {
        let exited = {
            let mut guard = self.process.lock();
            let Some(process) = guard.as_mut() else {
                return Ok(None);
            };
            process
                .child
                .try_wait()
                .map_err(|e| e.to_string())?
                .map(|status| (guard.take().unwrap(), status))
        };
        let Some((process, status)) = exited else {
            return Ok(None);
        };
        for reader in process.readers {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), reader).await;
        }
        let reason = String::from_utf8_lossy(&process.diagnostic.lock())
            .trim()
            .to_owned();
        *self.client.lock() = None;
        *self.connection.lock() = None;
        Ok(Some(DaemonExit {
            success: status.success(),
            shutdown_complete: process.shutdown_complete.load(Ordering::SeqCst),
            reason: if reason.is_empty() {
                format!("Daemon exited: {status}")
            } else {
                reason
            },
        }))
    }

    async fn terminate_and_wait(&self) -> Result<(), String> {
        {
            let mut guard = self.process.lock();
            if let Some(process) = guard.as_mut() {
                process.child.stdin.take();
            }
        }
        wait_for_termination(
            || async { Ok(self.exited().await?.is_some() || self.process.lock().is_none()) },
            || {
                if let Some(process) = self.process.lock().as_mut() {
                    crate::infrastructure::process::parent_lifetime::terminate_descendants(
                        process.child.id(),
                    );
                    process.child.kill().map_err(|e| e.to_string())?;
                }
                Ok(())
            },
        )
        .await
    }

    async fn request_shutdown(&self, intent: StopIntent) -> Result<ShutdownResponse, String> {
        let client = self.client()?;
        let id = uuid::Uuid::new_v4().to_string();
        let code = match intent {
            StopIntent::Quit(code) => code,
            _ => 0,
        };
        let response = client
            .request(wire::CommandRequest {
                request_id: id.clone(),
                deadline_unix_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .as_millis() as u64
                    + 30_000,
                command: Some(wire::command_request::Command::RequestApplicationQuit(
                    wire::RequestApplicationQuitRequest {
                        request: Some(wire::ApplicationQuitRequestDtoV1 {
                            request_id: Some(id),
                            intent: Some(wire::ApplicationQuitIntentDtoV1 {
                                variant: Some(match intent {
                                    StopIntent::Restart => {
                                        wire::application_quit_intent_dto_v1::Variant::Restart(
                                            wire::ApplicationQuitIntentDtoV1Restart {
                                                code: Some(code),
                                            },
                                        )
                                    }
                                    _ => wire::application_quit_intent_dto_v1::Variant::Exit(
                                        wire::ApplicationQuitIntentDtoV1Exit { code: Some(code) },
                                    ),
                                }),
                            }),
                        }),
                    },
                )),
                ..Default::default()
            })
            .await?;
        shutdown_response(response)
    }
}

#[async_trait::async_trait]
impl DaemonGateway for DaemonProcessGateway {
    fn attach(
        &self,
        id: String,
    ) -> Result<crate::usecase::daemon_supervision::DesktopAttachment, String> {
        let client = self.client()?;
        let (frames, cancelled) = client.attach(id);
        Ok(crate::usecase::daemon_supervision::DesktopAttachment {
            hello: client.hello(),
            frames,
            cancelled,
        })
    }
    fn detach(&self, id: &str) {
        if let Ok(client) = self.client() {
            client.detach(id);
        }
    }
    async fn forward(
        &self,
        bytes: Vec<u8>,
    ) -> Result<(), crate::usecase::daemon_supervision::DesktopSendError> {
        use crate::usecase::daemon_supervision::DesktopSendError;
        let envelope =
            wire::Envelope::decode(bytes.as_slice()).map_err(DesktopSendError::not_sent)?;
        self.client()
            .map_err(DesktopSendError::not_sent)?
            .forward(envelope)
            .await
    }
    async fn connection(&self) -> Result<Option<DaemonConnection>, Failure> {
        if let Ok(client) = self.client() {
            return Ok(if client.connected() {
                self.connection.lock().clone()
            } else {
                None
            });
        }

        let previous_failure = self
            .client
            .lock()
            .as_ref()
            .and_then(|client| client.failure());
        if let Some(reason) = previous_failure {
            self.client.lock().take();
            return Err(Failure {
                stage: FailureStage::Initialization,
                reason,
            });
        }
        let discovery = crate::infrastructure::local_api::read_local_api_discovery(&self.data_dir)
            .map_err(|error| Failure {
                stage: FailureStage::Initialization,
                reason: format!("{error:?}"),
            })?;
        let Some(discovery) = discovery else {
            return Ok(None);
        };
        let expected = self
            .process
            .lock()
            .as_ref()
            .map(|p| (p.child.id(), p.launch_id.clone()));
        let Some((pid, launch_id)) = expected else {
            return Ok(None);
        };
        if discovery.pid != pid {
            return Err(Failure {
                stage: FailureStage::Initialization,
                reason: "Discovery still points to a different daemon process.".into(),
            });
        }
        self.connect_client(&launch_id).await
    }
    fn restored(&self) -> bool {
        self.client().is_ok_and(|client| client.restored())
    }
    async fn finish_restoration(&self, attachment_id: &str) -> Result<(), String> {
        self.client()?.finish_restoration(attachment_id).await
    }
    fn connected(&self) -> bool {
        self.client().is_ok_and(|client| client.connected())
    }
}

async fn wait_for_termination<F: std::future::Future<Output = Result<bool, String>>>(
    mut exited: impl FnMut() -> F,
    mut kill: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let start = tokio::time::Instant::now();
    let mut killed = false;
    loop {
        if exited().await? {
            return Ok(());
        }
        if start.elapsed() >= std::time::Duration::from_secs(10) {
            return Err("Daemon exit could not be confirmed after termination.".into());
        }
        if !killed && start.elapsed() >= std::time::Duration::from_secs(5) {
            kill()?;
            killed = true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
#[path = "daemon_supervision_test.rs"]
mod daemon_supervision_tests;

fn shutdown_response(response: wire::Envelope) -> Result<ShutdownResponse, String> {
    match response.body {
        Some(wire::envelope::Body::Response(response)) => match response.outcome {
            Some(wire::command_response::Outcome::Result(result)) => match result.command {
                Some(wire::command_result::Command::RequestApplicationQuit(outcome)) => match outcome.variant {
                    Some(wire::application_quit_outcome_dto_v1::Variant::Accepted(_)) => Ok(ShutdownResponse::Accepted),
                    Some(wire::application_quit_outcome_dto_v1::Variant::PreviousShutdownReconciliationRequired(_)) => Ok(ShutdownResponse::DecisionRequired("Previous shutdown requires a decision before switching.".into())),
                    other => Err(format!("Shutdown requires confirmation: {other:?}")),
                },
                _ => Err("Unexpected shutdown response.".into()),
            },
            other => Err(format!("Shutdown request was not accepted: {other:?}")),
        },
        _ => Err("Unexpected shutdown response.".into()),
    }
}
