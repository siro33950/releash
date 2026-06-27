use crate::protocol::AgentStreamSync;
use crate::protocol::{AgentStreamDeltaMsg, AgentStreamPartMsg, WsMessage};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, Notify};

pub type WsSender = mpsc::UnboundedSender<WsMessage>;
pub type WsReceiver = mpsc::UnboundedReceiver<WsMessage>;

const STREAM_DELTA_QUEUE_LIMIT: usize = 1024;
const STREAM_DELTA_QUEUE_BYTE_LIMIT: usize = 512 * 1024;

#[derive(Debug, Clone)]
enum StreamOutbound {
    Delta(AgentStreamDeltaMsg),
    Snapshot(AgentStreamSync),
}

impl StreamOutbound {
    fn key(&self) -> (String, String) {
        match self {
            Self::Delta(msg) => (msg.session_id.clone(), msg.message_id.clone()),
            Self::Snapshot(msg) => (msg.session_id.clone(), msg.message_id.clone()),
        }
    }

    fn estimated_bytes(&self) -> usize {
        match self {
            Self::Delta(msg) => estimate_stream_parts_bytes(&msg.parts),
            Self::Snapshot(msg) => estimate_stream_parts_bytes(&msg.parts),
        }
    }
}

pub struct WsBroadcaster {
    sender: Mutex<Option<WsSender>>,
    /// Ordered delta/snapshot queue for stream messages. Normal operation
    /// appends `AgentStreamDelta`; when the queue exceeds its cap for a slow
    /// consumer, queued entries for that message collapse to a cumulative
    /// snapshot at the same seq so the receiver converges without another
    /// resync round trip.
    stream_queue: Mutex<VecDeque<StreamOutbound>>,
    /// Wakeup for the consumer to drain `stream_queue`. `Notify`
    /// collapses multiple producer signals into a single permit, so a burst
    /// of producer updates yields at most one drain pass.
    stream_notify: Arc<Notify>,
}

impl Default for WsBroadcaster {
    fn default() -> Self {
        Self {
            sender: Mutex::new(None),
            stream_queue: Mutex::new(VecDeque::new()),
            stream_notify: Arc::new(Notify::new()),
        }
    }
}

impl WsBroadcaster {
    pub fn try_send(&self, msg: WsMessage) {
        let guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(msg);
        }
    }

    /// Forward a message to the active WS session without buffering.
    /// Returns `true` if the message was delivered to the channel, `true`
    /// when no sender is registered (no WS client to satisfy — not a failure),
    /// and `false` only when the sender is registered but `send` failed
    /// (i.e. the receiver was dropped).
    pub fn send_without_buffer(&self, msg: WsMessage) -> bool {
        let guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(sender) => sender.send(msg).is_ok(),
            None => true,
        }
    }

    /// Best-effort enqueue of a normal streaming delta.
    pub fn send_stream_delta<F>(&self, msg: AgentStreamDeltaMsg, overflow_snapshot: F)
    where
        F: FnOnce() -> AgentStreamSync,
    {
        // Hold the sender lock for the entire critical section so a concurrent
        // `set_sender(None)` cannot clear stream buffers between our
        // sender-present check and the slot insert (which would leave a stale
        // message in the queue after disconnect).
        let sender_guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        if sender_guard.is_none() {
            return;
        }
        let mut queue = self.stream_queue.lock().unwrap_or_else(|e| e.into_inner());
        queue.push_back(StreamOutbound::Delta(msg));
        if stream_queue_exceeds_limit(&queue) {
            let overflow_snapshot = overflow_snapshot();
            let key = (
                overflow_snapshot.session_id.clone(),
                overflow_snapshot.message_id.clone(),
            );
            collapse_stream_queue_to_snapshot(&mut queue, key, overflow_snapshot);
        }
        drop(sender_guard);
        self.stream_notify.notify_one();
    }

    /// Best-effort enqueue of a resync snapshot.
    pub fn send_stream_snapshot(&self, msg: AgentStreamSync) {
        let sender_guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        if sender_guard.is_none() {
            return;
        }
        let key = (msg.session_id.clone(), msg.message_id.clone());
        let mut queue = self.stream_queue.lock().unwrap_or_else(|e| e.into_inner());
        collapse_stream_queue_to_snapshot(&mut queue, key, msg);
        drop(sender_guard);
        self.stream_notify.notify_one();
    }

    /// Drain every queued stream delta/snapshot in send order.
    pub fn drain_stream_messages(&self) -> Vec<WsMessage> {
        let mut queue = self.stream_queue.lock().unwrap_or_else(|e| e.into_inner());
        queue
            .drain(..)
            .map(|item| match item {
                StreamOutbound::Delta(delta) => WsMessage::AgentStreamDelta(delta),
                StreamOutbound::Snapshot(snapshot) => WsMessage::AgentStreamSync(snapshot),
            })
            .collect()
    }

    /// Notify handle for the WS forward task. The task `select!`s on this
    /// alongside the regular `WsReceiver` and, on wakeup, calls
    /// `drain_stream_messages` to forward queued stream messages to the WS client.
    pub fn stream_sync_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.stream_notify)
    }

    pub fn set_sender(&self, sender: Option<WsSender>) {
        let mut guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        let cleared = sender.is_none();
        if sender.is_some() {
            crate::other::telemetry::increment_ws_reconnects();
        }
        *guard = sender;
        if cleared {
            // No active WS session — drop queued stream messages so a future
            // session does not receive data intended for the prior client.
            self.stream_queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        }
    }

    pub fn has_subscriber(&self) -> bool {
        self.sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    pub fn create_channel() -> (WsSender, WsReceiver) {
        mpsc::unbounded_channel()
    }
}

fn stream_queue_exceeds_limit(queue: &VecDeque<StreamOutbound>) -> bool {
    queue.len() > STREAM_DELTA_QUEUE_LIMIT
        || queue
            .iter()
            .map(StreamOutbound::estimated_bytes)
            .sum::<usize>()
            > STREAM_DELTA_QUEUE_BYTE_LIMIT
}

fn collapse_stream_queue_to_snapshot(
    queue: &mut VecDeque<StreamOutbound>,
    key: (String, String),
    snapshot: AgentStreamSync,
) {
    let before = queue.len();
    queue.retain(|item| item.key() != key);
    if before != queue.len() {
        crate::other::telemetry::increment_dropped_stream_frames();
    }
    queue.push_back(StreamOutbound::Snapshot(snapshot));
}

fn estimate_stream_parts_bytes(parts: &[AgentStreamPartMsg]) -> usize {
    parts
        .iter()
        .map(|part| match part {
            AgentStreamPartMsg::Thinking { content, .. }
            | AgentStreamPartMsg::Text { content, .. }
            | AgentStreamPartMsg::ToolResult { content, .. }
            | AgentStreamPartMsg::Error { content, .. } => content.len(),
            AgentStreamPartMsg::ToolUse {
                tool, input, id, ..
            } => tool.len() + id.len() + serde_json::to_string(input).map_or(0, |s| s.len()),
            AgentStreamPartMsg::Permission {
                request,
                status,
                answers,
                ..
            } => {
                status.len()
                    + serde_json::to_string(request).map_or(0, |s| s.len())
                    + answers
                        .as_ref()
                        .and_then(|value| serde_json::to_string(value).ok())
                        .map_or(0, |s| s.len())
            }
            AgentStreamPartMsg::TaskStatus {
                task_tool_use_id,
                status,
                description,
                summary,
            } => {
                task_tool_use_id.len()
                    + status.len()
                    + description.as_ref().map_or(0, String::len)
                    + summary.as_ref().map_or(0, String::len)
            }
            AgentStreamPartMsg::TodoListSnapshot { items } => {
                items.iter().map(|item| item.text.len() + 1).sum()
            }
            AgentStreamPartMsg::SystemNotification {
                notification_type,
                status,
                label,
                detail,
                hook_id,
            } => {
                notification_type.len()
                    + status.len()
                    + label.len()
                    + detail.as_ref().map_or(0, String::len)
                    + hook_id.as_ref().map_or(0, String::len)
            }
            AgentStreamPartMsg::Image { data, media_type } => data.len() + media_type.len(),
            AgentStreamPartMsg::ImageRef { attachment } => {
                attachment.id.len() + attachment.media_type.len() + std::mem::size_of::<u64>()
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::protocol::pty::PtyOutputMsg;

    #[test]
    fn has_subscriber_tracks_registered_sender() {
        let broadcaster = WsBroadcaster::default();
        assert!(!broadcaster.has_subscriber());

        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        assert!(broadcaster.has_subscriber());

        broadcaster.set_sender(None);
        assert!(!broadcaster.has_subscriber());
    }

    #[test]
    fn try_send_drops_pty_output_when_no_sender() {
        let broadcaster = WsBroadcaster::default();
        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "hello".to_string(),
            sequence: 1,
        }));
        assert!(!broadcaster.has_subscriber());
    }

    #[test]
    fn try_send_forwards_pty_output_to_registered_sender_without_buffering() {
        let broadcaster = WsBroadcaster::default();
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "aaa".to_string(),
            sequence: 1,
        }));

        match rx.try_recv().unwrap() {
            WsMessage::PtyOutput(msg) => {
                assert_eq!(msg.pty_id, 1);
                assert_eq!(msg.data, "aaa");
            }
            other => panic!("unexpected message: {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn send_without_buffer_sends_without_side_effects() {
        let broadcaster = WsBroadcaster::default();
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        let sent = broadcaster.send_without_buffer(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "original".to_string(),
            sequence: 1,
        }));
        assert!(sent, "send must succeed when a receiver is registered");

        let mut received = vec![];
        while let Ok(msg) = rx.try_recv() {
            received.push(msg);
        }
        assert_eq!(received.len(), 1, "message should be sent to channel");
    }

    #[test]
    fn send_without_buffer_returns_false_when_receiver_dropped() {
        // PTY replay (`handle_pty_output_request`) calls `send_without_buffer`
        // to deliver the ring-buffer contents on subscribe without re-buffering
        // them. `false` indicates the sender is registered but its receiver was
        // dropped (client gone), letting callers decide whether to retry.
        let broadcaster = WsBroadcaster::default();
        let (tx, rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        drop(rx);

        let sent = broadcaster.send_without_buffer(WsMessage::Error(crate::protocol::ErrorMsg {
            code: "X".to_string(),
            message: "x".to_string(),
        }));
        assert!(
            !sent,
            "send must return false when the receiver has been dropped"
        );
    }

    #[test]
    fn send_without_buffer_returns_true_when_no_sender_registered() {
        // No WS client to satisfy → not a delivery failure. `true` lets the
        // caller (PTY replay path) treat the absence of a subscriber as a
        // benign no-op instead of an error.
        let broadcaster = WsBroadcaster::default();
        let sent = broadcaster.send_without_buffer(WsMessage::Error(crate::protocol::ErrorMsg {
            code: "X".to_string(),
            message: "x".to_string(),
        }));
        assert!(
            sent,
            "send must return true when no sender is registered (no client to satisfy)"
        );
    }

    fn dummy_stream_sync(session: &str, message: &str, seq: u64, n: usize) -> AgentStreamSync {
        AgentStreamSync {
            session_id: session.to_string(),
            message_id: message.to_string(),
            seq,
            parts: (0..n)
                .map(|i| crate::protocol::AgentStreamPartMsg::Text {
                    content: format!("p{i}"),
                    parent_tool_use_id: None,
                })
                .collect(),
        }
    }

    fn dummy_stream_delta(
        session: &str,
        message: &str,
        seq: u64,
        content: &str,
    ) -> AgentStreamDeltaMsg {
        AgentStreamDeltaMsg {
            session_id: session.to_string(),
            message_id: message.to_string(),
            seq,
            parts: vec![crate::protocol::AgentStreamPartMsg::Text {
                content: content.to_string(),
                parent_tool_use_id: None,
            }],
        }
    }

    fn queue_stream_delta(broadcaster: &WsBroadcaster, delta: AgentStreamDeltaMsg) {
        let snapshot = AgentStreamSync {
            session_id: delta.session_id.clone(),
            message_id: delta.message_id.clone(),
            seq: delta.seq,
            parts: delta.parts.clone(),
        };
        broadcaster.send_stream_delta(delta, || snapshot);
    }

    #[test]
    fn send_stream_delta_queues_ordered_deltas() {
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "a"));
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 2, "b"));

        let drained = broadcaster.drain_stream_messages();
        assert_eq!(drained.len(), 2);
        assert!(matches!(
            &drained[0],
            WsMessage::AgentStreamDelta(msg) if msg.seq == 1
        ));
        assert!(matches!(
            &drained[1],
            WsMessage::AgentStreamDelta(msg) if msg.seq == 2
        ));
        assert!(broadcaster.drain_stream_messages().is_empty());
    }

    #[test]
    fn send_stream_delta_preserves_tool_result_ref_without_full_tail() {
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        let full_tail = "USER_SECRET_TAIL";
        let output_id = "c".repeat(64);
        let delta = AgentStreamDeltaMsg {
            session_id: "S".to_string(),
            message_id: "M".to_string(),
            seq: 1,
            parts: vec![AgentStreamPartMsg::ToolResult {
                content: "preview only".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: Some(crate::protocol::AgentToolOutputRefMsg {
                    id: output_id.clone(),
                    byte_size: 4096,
                }),
                summary: Some(crate::protocol::AgentToolOutputSummaryMsg {
                    line_count: 1200,
                    byte_size: 4096,
                    is_error: false,
                    truncated: true,
                }),
            }],
        };

        broadcaster.send_stream_delta(delta, || panic!("snapshot should not be built"));

        let drained = broadcaster.drain_stream_messages();
        assert_eq!(drained.len(), 1);
        let json = serde_json::to_string(&drained[0]).unwrap();
        assert!(json.contains("\"contentRef\""));
        assert!(json.contains(&output_id));
        assert!(json.contains("preview only"));
        assert!(!json.contains(full_tail));
        match &drained[0] {
            WsMessage::AgentStreamDelta(msg) => {
                assert!(matches!(
                    &msg.parts[0],
                    AgentStreamPartMsg::ToolResult {
                        content,
                        content_ref: Some(content_ref),
                        summary: Some(summary),
                        ..
                    } if content == "preview only"
                        && content_ref.id == output_id
                        && summary.truncated
                ));
            }
            other => panic!("expected stream delta, got {other:?}"),
        }
    }

    #[test]
    fn send_stream_delta_does_not_build_snapshot_under_queue_limits() {
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        let mut snapshot_built = false;
        broadcaster.send_stream_delta(dummy_stream_delta("S", "M", 1, "x"), || {
            snapshot_built = true;
            dummy_stream_sync("S", "M", 1, 2)
        });

        assert!(
            !snapshot_built,
            "normal delta enqueue must not build the overflow snapshot"
        );
        assert_eq!(broadcaster.drain_stream_messages().len(), 1);
    }

    #[test]
    fn send_stream_snapshot_replaces_queued_deltas_for_same_message() {
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "a"));
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M-other", 1, "x"));
        broadcaster.send_stream_snapshot(dummy_stream_sync("S", "M", 2, 2));

        let drained = broadcaster.drain_stream_messages();
        assert_eq!(drained.len(), 2);
        assert!(matches!(
            &drained[0],
            WsMessage::AgentStreamDelta(msg) if msg.message_id == "M-other"
        ));
        assert!(matches!(
            &drained[1],
            WsMessage::AgentStreamSync(msg) if msg.message_id == "M" && msg.seq == 2
        ));
    }

    #[test]
    fn send_stream_delta_collapses_to_snapshot_when_queue_is_over_limit() {
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        for seq in 1..=(STREAM_DELTA_QUEUE_LIMIT as u64 + 1) {
            broadcaster.send_stream_delta(dummy_stream_delta("S", "M", seq, "x"), || {
                dummy_stream_sync("S", "M", seq, 2)
            });
        }

        let drained = broadcaster.drain_stream_messages();
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            WsMessage::AgentStreamSync(msg) => {
                assert_eq!(msg.seq, STREAM_DELTA_QUEUE_LIMIT as u64 + 1);
                assert_eq!(msg.parts.len(), 2);
                match &msg.parts[0] {
                    AgentStreamPartMsg::Text { content, .. } => {
                        assert_eq!(content, "p0");
                    }
                    other => panic!("expected text delta, got {other:?}"),
                }
            }
            other => panic!("expected snapshot collapse, got {other:?}"),
        }
    }

    #[test]
    fn send_stream_delta_drops_when_no_sender() {
        // Desktop-only / no WS client: the delta is dropped on the floor
        // so the producer's coalescing buffer cannot grow waiting for a
        // non-existent receiver.
        let broadcaster = WsBroadcaster::default();
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "x"));
        assert!(broadcaster.drain_stream_messages().is_empty());
    }

    #[test]
    fn set_sender_none_clears_queued_stream_messages() {
        // Disconnect must drop stale stream messages so reconnect doesn't see them.
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "x"));
        broadcaster.set_sender(None);
        assert!(broadcaster.drain_stream_messages().is_empty());
    }

    #[test]
    fn send_stream_delta_does_not_resurrect_queue_after_concurrent_disconnect() {
        // Regression: prior implementation released the sender lock before
        // inserting into the stream buffer, so a `set_sender(None)` racing
        // between the sender check and insert could leave a stale
        // message in the queue. With the sender lock held across the whole
        // critical section, the two operations are serialised: either the
        // disconnect wins (enqueue observes `sender_guard.is_none()` and
        // returns early) or the send wins (queue has the message and is
        // cleared by the subsequent disconnect). Either ordering must leave
        // the queue empty after disconnect.
        for _ in 0..200 {
            let broadcaster = Arc::new(WsBroadcaster::default());
            let (tx, _rx) = WsBroadcaster::create_channel();
            broadcaster.set_sender(Some(tx));

            let producer = {
                let broadcaster = Arc::clone(&broadcaster);
                std::thread::spawn(move || {
                    queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "x"));
                })
            };
            let disconnector = {
                let broadcaster = Arc::clone(&broadcaster);
                std::thread::spawn(move || {
                    broadcaster.set_sender(None);
                })
            };
            producer.join().unwrap();
            disconnector.join().unwrap();

            assert!(
                broadcaster.drain_stream_messages().is_empty(),
                "queue must not retain a stream message after disconnect"
            );
        }
    }

    #[tokio::test]
    async fn send_stream_delta_notifies_waiter() {
        // Consumer waits on `stream_sync_notify`; producer sends; consumer is
        // woken and drains the queue. Ensures the notify pipeline is connected.
        let broadcaster = Arc::new(WsBroadcaster::default());
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        let notify = broadcaster.stream_sync_notify();
        let consumer = {
            let broadcaster = Arc::clone(&broadcaster);
            tokio::spawn(async move {
                notify.notified().await;
                broadcaster.drain_stream_messages()
            })
        };
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 3, "x"));
        let drained = consumer.await.unwrap();
        assert_eq!(drained.len(), 1);
        assert!(matches!(
            &drained[0],
            WsMessage::AgentStreamDelta(msg) if msg.seq == 3
        ));
    }
}
