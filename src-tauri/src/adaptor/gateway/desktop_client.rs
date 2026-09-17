use super::shared::client_operation::identity;
use crate::adaptor::controller::api::protocol::client as wire;
use crate::domain::client_operation::{policy, transmission::TransmissionFailure};
use crate::usecase::client_handoff::ClientHandoffUsecase;
use crate::usecase::desktop_client::DesktopClientUsecase;
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message as Frame;

pub(super) type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type Reply = oneshot::Sender<Result<wire::Envelope, String>>;
use crate::usecase::daemon_supervision::DesktopSendError as FrameSendError;
enum Outbound {
    Frame {
        envelope: Box<wire::Envelope>,
        reply: Option<Reply>,
        sent: oneshot::Sender<Result<(), FrameSendError>>,
    },
    Restored(oneshot::Sender<Result<(), String>>),
}
#[derive(Default)]
struct ConnectionState {
    restored: AtomicBool,
    failure: parking_lot::Mutex<Option<String>>,
}

pub(crate) struct DesktopClient {
    sender: mpsc::Sender<Outbound>,
    frames: broadcast::Sender<Option<Vec<u8>>>,
    hello: wire::ClientHello,
    task: tokio::task::JoinHandle<()>,
    state: Arc<ConnectionState>,
    attachment: parking_lot::Mutex<Option<(String, oneshot::Sender<()>)>>,
    stop: Option<oneshot::Sender<()>>,
}
impl Drop for DesktopClient {
    fn drop(&mut self) {
        self.stop.take();
    }
}
impl DesktopClient {
    pub fn start(
        socket: Socket,
        hello: wire::ClientHello,
        handoff: Arc<ClientHandoffUsecase>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(128);
        let (frames, _) = broadcast::channel(256);
        let state = Arc::new(ConnectionState::default());
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(run(
            socket,
            receiver,
            stopped,
            frames.clone(),
            handoff,
            state.clone(),
            hello.instance_id.clone(),
        ));
        Self {
            sender,
            frames,
            hello,
            task,
            state,
            attachment: Default::default(),
            stop: Some(stop),
        }
    }
    pub fn restored(&self) -> bool {
        self.connected() && self.state.restored.load(Ordering::Acquire)
    }
    pub fn connected(&self) -> bool {
        !self.task.is_finished()
    }
    pub fn failure(&self) -> Option<String> {
        self.state.failure.lock().clone()
    }
    pub fn hello(&self) -> Vec<u8> {
        wire::Envelope {
            body: Some(wire::envelope::Body::Hello(self.hello.clone())),
        }
        .encode_to_vec()
    }
    pub fn attach(
        &self,
        id: String,
    ) -> (broadcast::Receiver<Option<Vec<u8>>>, oneshot::Receiver<()>) {
        let (cancel, cancelled) = oneshot::channel();
        *self.attachment.lock() = Some((id, cancel));
        (self.frames.subscribe(), cancelled)
    }
    pub fn detach(&self, id: &str) {
        let mut attachment = self.attachment.lock();
        if attachment
            .as_ref()
            .is_some_and(|(current, _)| current == id)
        {
            attachment.take();
        }
    }
    pub async fn forward(&self, envelope: wire::Envelope) -> Result<(), FrameSendError> {
        self.send(envelope, None).await
    }
    pub async fn finish_restoration(&self, attachment_id: &str) -> Result<(), String> {
        if self
            .attachment
            .lock()
            .as_ref()
            .is_none_or(|(id, _)| id != attachment_id)
        {
            return Err("Desktop attachment changed during restoration".into());
        }
        let (sender, receiver) = oneshot::channel();
        self.sender
            .send(Outbound::Restored(sender))
            .await
            .map_err(|_| "Desktop disconnected during restoration")?;
        receiver
            .await
            .map_err(|_| "Desktop disconnected during restoration".to_string())?
    }
    async fn send(
        &self,
        envelope: wire::Envelope,
        reply: Option<Reply>,
    ) -> Result<(), FrameSendError> {
        let (sent, result) = oneshot::channel();
        self.sender
            .send(Outbound::Frame {
                envelope: Box::new(envelope),
                reply,
                sent,
            })
            .await
            .map_err(|_| FrameSendError::not_sent("Desktop client disconnected"))?;
        result.await.unwrap_or_else(|_| {
            Err(FrameSendError::write_attempted(
                "Desktop client disconnected during write",
            ))
        })
    }
    pub async fn request(&self, request: wire::CommandRequest) -> Result<wire::Envelope, String> {
        let (reply, response) = oneshot::channel();
        self.send(
            wire::Envelope {
                body: Some(wire::envelope::Body::Request(Box::new(request))),
            },
            Some(reply),
        )
        .await
        .map_err(|e| e.reason)?;
        tokio::time::timeout(std::time::Duration::from_secs(35), response)
            .await
            .map_err(|_| "Daemon request outcome is unknown (response deadline exceeded).")?
            .map_err(|_| {
                "Daemon request outcome is unknown (response channel closed).".to_string()
            })?
    }
}

async fn transmit(
    socket: &mut Socket,
    envelope: wire::Envelope,
    reply: Option<Reply>,
    replies: &mut HashMap<String, Reply>,
    operations: &mut DesktopClientUsecase,
) -> Result<(), FrameSendError> {
    use wire::envelope::Body;
    if let Some(Body::RequestAck(ack)) = &envelope.body {
        operations
            .acknowledge(&ack.request_id)
            .map_err(FrameSendError::not_sent)?;
        replies.remove(&ack.request_id);
    }
    if let Some(Body::Request(request)) = &envelope.body {
        let (command, args) = request
            .clone()
            .into_value()
            .map_err(FrameSendError::not_sent)?;
        let identity = identity(command, args);
        operations
            .transmit(
                &request.request_id,
                identity,
                request.deadline_unix_ms,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(FrameSendError::not_sent)?
                    .as_millis() as u64,
            )
            .map_err(FrameSendError::not_sent)?;
        if let Some(reply) = reply {
            replies.insert(request.request_id.clone(), reply);
        }
    }
    tokio::time::timeout(
        std::time::Duration::from_millis(policy::CONNECT_TIMEOUT_MS),
        socket.send(Frame::Binary(envelope.encode_to_vec().into())),
    )
    .await
    .map_err(|_| FrameSendError::write_attempted("Desktop transmission deadline exceeded"))?
    .map_err(FrameSendError::write_attempted)
}

async fn run(
    mut socket: Socket,
    mut outgoing: mpsc::Receiver<Outbound>,
    mut stopped: oneshot::Receiver<()>,
    frames: broadcast::Sender<Option<Vec<u8>>>,
    handoff: Arc<ClientHandoffUsecase>,
    state: Arc<ConnectionState>,
    instance_id: String,
) {
    use wire::envelope::Body;
    let mut replies = HashMap::<String, Reply>::new();
    let mut operations = DesktopClientUsecase::new(handoff);
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_millis(
        policy::HEARTBEAT_INTERVAL_MS,
    ));
    let mut awaiting_heartbeat = None;
    let mut heartbeat_deadline = tokio::time::Instant::now();
    let result: Result<(), String> = async {
        loop {
            tokio::select! {
                biased;
                _ = &mut stopped => break,
                outbound = outgoing.recv() => {
                    let Some(outbound) = outbound else { break; };
                    let (envelope, reply, sent) = match outbound {
                        Outbound::Restored(reply) => {
                            let result = operations.finish_restoration();
                            if result.is_ok() { state.restored.store(true, Ordering::Release); }
                            let _ = reply.send(result);
                            continue;
                        }
                        Outbound::Frame { envelope, reply, sent } => (*envelope, reply, sent),
                    };
                    if let Some(Body::OperationQuery(query)) = &envelope.body {
                        if let Some(request) = &query.request {
                            let (command, args) = request.clone().into_value()?;
                            let identity = identity(command, args);
                            match operations.query(&query.request_id, identity, query.sent, &query.instance_id, &instance_id) {
                                Ok(true) => {},
                                Ok(false) => {
                                    let _ = frames.send(Some(wire::Envelope { body: Some(Body::OperationStatus(wire::OperationStatus { request_id: query.request_id.clone(), query_id: query.query_id.clone(), state: "blocked".into(), ..Default::default() })) }.encode_to_vec()));
                                    let _ = sent.send(Ok(()));
                                    continue;
                                }
                                Err(reason) => { let _ = sent.send(Err(FrameSendError::not_sent(reason))); continue; }
                            }
                        }
                    }
                    let result = transmit(&mut socket, envelope, reply, &mut replies, &mut operations).await;
                    let failure = result.as_ref().err().filter(|e| e.state == TransmissionFailure::Unknown).map(|e| e.reason.clone());
                    let _ = sent.send(result);
                    if let Some(reason) = failure { return Err(reason); }
                }
                frame = socket.next() => {
                    let Some(frame) = frame else { break; };
                    match frame.map_err(|e| e.to_string())? {
                        Frame::Binary(bytes) => {
                            let mut envelope = wire::Envelope::decode(bytes.clone()).map_err(|e| e.to_string())?;
                            if let Some(Body::Heartbeat(reply)) = &envelope.body {
                                if awaiting_heartbeat.as_ref() == Some(&reply.nonce) { awaiting_heartbeat = None; continue; }
                            }
                            if let Some(Body::OperationStatus(status)) = &mut envelope.body {
                                if status.state == "unknown" && operations.unknown_after_restart(&status.request_id) { status.state = "restored_unknown".into(); }
                            }
                            if let Some(Body::Response(response)) = &envelope.body {
                                operations.responded(&response.request_id, matches!(response.outcome, Some(wire::command_response::Outcome::Result(_))));
                                if let Some(reply) = replies.remove(&response.request_id) {
                                    let id = response.request_id.clone();
                                    operations.acknowledge(&id)?;
                                    socket.send(Frame::Binary(wire::Envelope { body: Some(Body::RequestAck(wire::RequestAck { request_id: id, ..Default::default() })) }.encode_to_vec().into())).await.map_err(|e| e.to_string())?;
                                    let _ = reply.send(Ok(envelope));
                                    continue;
                                }
                            }
                            let _ = frames.send(Some(envelope.encode_to_vec()));
                        }
                        Frame::Close(_) => break,
                        Frame::Ping(bytes) => socket.send(Frame::Pong(bytes)).await.map_err(|e| e.to_string())?,
                        _ => {},
                    }
                }
                _ = heartbeat.tick(), if awaiting_heartbeat.is_none() => {
                    let nonce = uuid::Uuid::new_v4().to_string();
                    awaiting_heartbeat = Some(nonce.clone());
                    heartbeat_deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(policy::HEARTBEAT_TIMEOUT_MS);
                    socket.send(Frame::Binary(wire::Envelope { body: Some(Body::Heartbeat(wire::Heartbeat { nonce })) }.encode_to_vec().into())).await.map_err(|e| e.to_string())?;
                }
                _ = tokio::time::sleep_until(heartbeat_deadline), if awaiting_heartbeat.is_some() => { return Err("Daemon heartbeat timed out".into()); }
            }
        }
        Ok(())
    }.await;
    if let Err(error) = result {
        log::warn!("Desktop client connection: {error}");
        *state.failure.lock() = Some(error);
    }
    outgoing.close();
    while let Ok(outbound) = outgoing.try_recv() {
        if let Outbound::Frame { sent, .. } = outbound {
            let _ = sent.send(Err(FrameSendError::not_sent(
                "Desktop client disconnected before transmission",
            )));
        }
    }
    let _ = frames.send(None);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), socket.close(None)).await;
}

#[cfg(test)]
#[path = "desktop_client_test.rs"]
mod desktop_client_tests;
