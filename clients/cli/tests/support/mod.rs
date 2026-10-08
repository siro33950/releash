use prost::Message;
use releash::wire;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

type Handler = Box<dyn Fn(&str, &[u8]) -> (u16, Vec<u8>) + Send>;

pub struct Server {
    directory: tempfile::TempDir,
    task: JoinHandle<Vec<String>>,
    stop: Arc<AtomicBool>,
}

impl Server {
    pub fn start(protocol: u32, payload: wire::StatePayload) -> Self {
        let event = payload
            .value
            .is_some()
            .then_some(wire::state_subscription_event::Event::Snapshot(payload));
        Self::start_event(protocol, event)
    }
    pub fn start_failure(code: i32, message: &str) -> Self {
        Self::start_event(
            1,
            Some(wire::state_subscription_event::Event::Failure(
                wire::StateReadFailure {
                    code,
                    message: message.into(),
                },
            )),
        )
    }
    fn start_event(protocol: u32, event: Option<wire::state_subscription_event::Event>) -> Self {
        Self::start_configured(protocol, event, None, "operator")
    }
    pub fn start_checked(payload: wire::StatePayload, handler: Handler) -> Self {
        Self::start_configured(
            1,
            Some(wire::state_subscription_event::Event::Snapshot(payload)),
            Some(handler),
            "operator",
        )
    }
    pub fn start_unary(handler: Handler) -> Self {
        Self::start_configured(1, None, Some(handler), "operator")
    }
    pub fn start_hook() -> Self {
        Self::start_configured(1, None, None, "hook-token")
    }
    fn start_configured(
        protocol: u32,
        event: Option<wire::state_subscription_event::Event>,
        handler: Option<Handler>,
        token: &'static str,
    ) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let pid = std::process::id();
        let started = releash::discovery::process_start_time(pid).unwrap();
        let discovery = releash::discovery::LocalApiDiscovery {
            port: port.into(),
            token: "operator".into(),
            daemon_id: "fixture".into(),
            pid,
            process_started_at: started,
            ..Default::default()
        };
        std::fs::write(
            directory.path().join("client-api.json"),
            serde_json::to_vec(&discovery).unwrap(),
        )
        .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let task = std::thread::spawn(move || {
            let mut requests = Vec::new();
            let mut deadline = Instant::now() + Duration::from_secs(10);
            let count = if protocol != 1 {
                1
            } else if event.is_none() {
                2
            } else {
                3
            };
            while requests.len() < count
                && Instant::now() < deadline
                && !stopped.load(Ordering::SeqCst)
            {
                let (mut socket, _) = match listener.accept() {
                    Ok(socket) => socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let header = String::from_utf8(request).unwrap();
                let length = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                socket.read_exact(&mut body).unwrap();
                requests.push(header.lines().next().unwrap().to_owned());
                assert!(header
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {token}")));
                let mut status = 200;
                let (content_type, response) = if header.contains("/GetServerInfo ") {
                    (
                        "application/proto",
                        wire::ServerInfo {
                            daemon_id: "fixture".into(),
                            pid,
                            process_started_at: started,
                            protocol,
                            release: "server-fixture".into(),
                            capabilities: vec!["fixture-capability".into()],
                            serving_status: wire::ServingStatus::Serving as i32,
                            cli_installation: Some(wire::CliInstallationResult {
                                status: wire::CliInstallationStatus::Allowed as i32,
                                reason: String::new(),
                            }),
                        }
                        .encode_to_vec(),
                    )
                } else if handler.is_some() && !header.contains("/OpenStateStream ") {
                    let (code, response) =
                        handler.as_ref().unwrap()(header.lines().next().unwrap(), &body);
                    status = code;
                    (
                        if status == 200 {
                            "application/proto"
                        } else {
                            "application/json"
                        },
                        response,
                    )
                } else if header.contains("/ReceiveProviderSignal ") {
                    let request =
                        wire::ReceiveProviderSignalRequest::decode(body.as_slice()).unwrap();
                    assert_eq!(request.payload, b"\0opaque provider bytes\xff");
                    assert_eq!(request.slot_id, "slot");
                    assert_eq!(request.binding_id, "binding");
                    assert_eq!(request.capability, "capability");
                    assert_eq!(request.agent_session_id, "session");
                    (
                        "application/proto",
                        wire::ReceiveProviderSignalResponse {
                            result: Some(wire::receive_provider_signal_response::Result::Applied(
                                wire::Unit {},
                            )),
                        }
                        .encode_to_vec(),
                    )
                } else if header.contains("/OpenStateStream ") {
                    let mut bytes = frame(wire::StateSubscriptionEvent {
                        event: Some(wire::state_subscription_event::Event::Ready(wire::Unit {})),
                        ..Default::default()
                    });
                    bytes.extend(frame(wire::StateSubscriptionEvent {
                        event: event.clone(),
                        ..Default::default()
                    }));
                    bytes.extend([2, 0, 0, 0, 2, b'{', b'}']);
                    ("application/connect+proto", bytes)
                } else {
                    assert!(header.contains("/StartStateSubscription "));
                    let request =
                        wire::StartStateSubscriptionRequest::decode(body.as_slice()).unwrap();
                    assert!(!request.client_id.is_empty());
                    assert!(!request.subscription_id.is_empty());
                    ("application/proto", Vec::new())
                };
                write!(socket, "HTTP/1.1 {status} OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", response.len()).unwrap();
                socket.write_all(&response).unwrap();
                deadline = Instant::now() + Duration::from_secs(10);
            }
            requests
        });
        Self {
            directory,
            task,
            stop,
        }
    }
    pub fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_releash"));
        command
            .arg("--data-dir")
            .arg(self.directory.path())
            .args(arguments)
            .env(
                "RELEASH_DATA_DIR",
                self.directory.path().join("wrong-directory"),
            );
        command
    }
    pub fn run(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().unwrap()
    }
    pub fn finish(self) -> Vec<String> {
        self.stop.store(true, Ordering::SeqCst);
        self.task.join().unwrap()
    }
}
fn frame(message: wire::StateSubscriptionEvent) -> Vec<u8> {
    let bytes = message.encode_to_vec();
    let mut result = vec![0];
    result.extend((bytes.len() as u32).to_be_bytes());
    result.extend(bytes);
    result
}
