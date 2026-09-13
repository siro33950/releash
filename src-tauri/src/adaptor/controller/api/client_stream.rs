use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use prost::Message;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

use crate::adaptor::controller::api::protocol::client::{self as wire, TerminalEvent};
use crate::adaptor::controller::terminal_surface::attach;
use crate::adaptor::protocol::terminal::TerminalSurfaceStreamItemV1;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

#[derive(Clone)]
pub(crate) struct TerminalApiDeps {
    application: Arc<TerminalSurfaceApplication>,
    attachment_limit: Arc<Semaphore>,
}

impl TerminalApiDeps {
    pub(crate) fn new(application: Arc<TerminalSurfaceApplication>) -> Self {
        Self {
            application,
            attachment_limit: Arc::new(Semaphore::new(16)),
        }
    }
}

pub(crate) struct TerminalConnection {
    deps: Option<TerminalApiDeps>,
    next_generation: u64,
    attachments: HashMap<String, Attachment>,
    sender: mpsc::Sender<OutputFrame>,
    pub(crate) receiver: mpsc::Receiver<OutputFrame>,
}

struct Attachment {
    _permit: OwnedSemaphorePermit,
    task: tokio::task::JoinHandle<()>,
    generation: u64,
    sequence: u64,
    acknowledged: u64,
    pending: VecDeque<(u64, OwnedSemaphorePermit)>,
}

impl TerminalConnection {
    pub(crate) fn new(deps: Option<TerminalApiDeps>) -> Self {
        let (sender, receiver) = mpsc::channel(16);
        Self {
            deps,
            next_generation: 0,
            attachments: HashMap::new(),
            sender,
            receiver,
        }
    }

    pub(crate) fn dispatch(
        &mut self,
        command: wire::command_request::Command,
    ) -> Result<wire::command_result::Command, wire::CommandError> {
        match command {
            wire::command_request::Command::AttachTerminalSurface(args) => self
                .attach(args)
                .map(wire::command_result::Command::AttachTerminalSurface),
            wire::command_request::Command::DetachTerminalSurface(args) => {
                let id = crate::adaptor::controller::client::required(
                    args.attachment_id,
                    "attachmentId",
                )?;
                self.detach(&id);
                Ok(wire::command_result::Command::DetachTerminalSurface(
                    wire::Unit {},
                ))
            }
            _ => Err(invalid("Not a terminal stream command")),
        }
    }

    pub(crate) fn attach(
        &mut self,
        args: wire::AttachTerminalSurfaceRequest,
    ) -> Result<wire::Unit, wire::CommandError> {
        let attachment_id =
            crate::adaptor::controller::client::required(args.attachment_id, "attachmentId")?;
        let owner = crate::adaptor::controller::client::convert(
            crate::adaptor::controller::client::required(args.owner, "owner")?,
        )?;
        let recovery = crate::adaptor::controller::client::required(args.recovery, "recovery")?;
        if self.attachments.contains_key(&attachment_id) {
            return Err(invalid("Attachment already exists on this connection"));
        }
        let deps = self
            .deps
            .as_ref()
            .ok_or_else(|| invalid("Terminal unavailable"))?;
        let permit = deps
            .attachment_limit
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                wire::CommandError::from(crate::other::AppError::coded(
                    "ATTACHMENT_LIMIT",
                    "Too many terminal attachments",
                ))
            })?;
        let mut stream = attach(&deps.application, &attachment_id, owner, recovery)
            .map_err(wire::CommandError::from)?;
        let sender = self.sender.clone();
        let id = attachment_id.clone();
        self.next_generation += 1;
        let generation = self.next_generation;
        let task = tokio::spawn(async move {
            let window = Arc::new(Semaphore::new(16));
            let mut sequence = 0;
            let mut exited = false;
            while let Some(item) = stream.next().await {
                let event = TerminalEvent::from(TerminalSurfaceStreamItemV1::from(item));
                exited |= matches!(event.item, Some(wire::terminal_event::Item::Exit(_)))
                    || matches!(&event.item, Some(wire::terminal_event::Item::Snapshot(snapshot)) if snapshot.is_exited);
                let data = event.encode_to_vec();
                let overhead = wire::stream_frame(&id, u64::MAX, vec![], true).len() + 8;
                let chunks = data.chunks(wire::MAX_STREAM_FRAME_BYTES - overhead);
                let count = chunks.len();
                for (index, chunk) in chunks.enumerate() {
                    let Ok(permit) = window.clone().acquire_owned().await else {
                        return;
                    };
                    sequence += 1;
                    let end = index + 1 == count;
                    let frame = OutputFrame {
                        id: id.clone(),
                        generation,
                        sequence,
                        permit: Some(permit),
                        bytes: wire::stream_frame(&id, sequence, chunk.to_vec(), end),
                    };
                    if sender.send(frame).await.is_err() {
                        return;
                    }
                }
            }
            let _ = sender
                .send(OutputFrame {
                    bytes: if exited {
                        Vec::new()
                    } else {
                        wire::Envelope {
                            body: Some(wire::envelope::Body::StreamClosed(wire::StreamClosed {
                                attachment_id: id.clone(),
                            })),
                        }
                        .encode_to_vec()
                    },
                    id,
                    generation,
                    sequence,
                    permit: None,
                })
                .await;
        });
        self.attachments.insert(
            attachment_id,
            Attachment {
                _permit: permit,
                task,
                generation,
                sequence: 0,
                acknowledged: 0,
                pending: VecDeque::new(),
            },
        );
        Ok(wire::Unit {})
    }

    pub(crate) fn detach(&mut self, id: &str) {
        if let Some(attachment) = self.attachments.remove(id) {
            attachment.task.abort();
        }
        if let Some(deps) = &self.deps {
            deps.application.detach(id);
        }
    }

    pub(crate) fn acknowledge_output(&self, id: &str, sequence: u64) {
        if let Some(deps) = &self.deps {
            deps.application.acknowledge_output(id, sequence);
        }
    }

    pub(crate) fn acknowledge(
        &mut self,
        id: &str,
        sequence: u64,
    ) -> Result<(), wire::CommandError> {
        let Some(attachment) = self.attachments.get_mut(id) else {
            return Ok(());
        };
        if sequence > attachment.sequence {
            return Err(invalid("Ack exceeds sent sequence"));
        }
        if sequence <= attachment.acknowledged {
            return Ok(());
        }
        attachment.acknowledged = sequence;
        while attachment
            .pending
            .front()
            .is_some_and(|(frame, _)| *frame <= sequence)
        {
            attachment.pending.pop_front();
        }
        Ok(())
    }

    pub(crate) fn receive(&mut self, frame: OutputFrame) -> Option<Vec<u8>> {
        let attachment = self.attachments.get_mut(&frame.id)?;
        if attachment.generation != frame.generation {
            return None;
        }
        let Some(permit) = frame.permit else {
            self.detach(&frame.id);
            return (!frame.bytes.is_empty()).then_some(frame.bytes);
        };
        attachment.sequence = frame.sequence;
        attachment.pending.push_back((frame.sequence, permit));
        Some(frame.bytes)
    }
}

impl Drop for TerminalConnection {
    fn drop(&mut self) {
        for (id, attachment) in self.attachments.drain() {
            attachment.task.abort();
            if let Some(deps) = &self.deps {
                deps.application.detach(&id);
            }
        }
    }
}

pub(crate) fn invalid(message: impl Into<String>) -> wire::CommandError {
    crate::adaptor::controller::client::invalid_request(message)
}

pub(crate) struct OutputFrame {
    id: String,
    generation: u64,
    sequence: u64,
    permit: Option<OwnedSemaphorePermit>,
    bytes: Vec<u8>,
}

#[cfg(test)]
#[path = "client_stream_test.rs"]
mod client_stream_tests;
