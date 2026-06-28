use crate::adaptor::protocol::AgentStreamSync;
use crate::adaptor::protocol::{AgentStreamDeltaMsg, AgentStreamPartMsg, WsMessage};
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::mpsc;

pub type WsSender = mpsc::UnboundedSender<()>;
pub type WsReceiver = mpsc::UnboundedReceiver<()>;

const STREAM_DELTA_QUEUE_LIMIT: usize = 1024;
const STREAM_DELTA_QUEUE_BYTE_LIMIT: usize = 512 * 1024;

pub struct WsBroadcaster {
    sender: Mutex<Option<WsSender>>,
    /// Ordered outbound queue for every WS push. Stream entries use
    /// `AgentStreamDeltaMsg`; snapshot replacement is represented by the same
    /// DTO with `snapshot: true`, so the forwarder has a single event log.
    outbound_queue: Mutex<VecDeque<WsMessage>>,
}

impl Default for WsBroadcaster {
    fn default() -> Self {
        Self {
            sender: Mutex::new(None),
            outbound_queue: Mutex::new(VecDeque::new()),
        }
    }
}

impl WsBroadcaster {
    pub fn try_send(&self, msg: WsMessage) {
        let _ = self.enqueue_if_connected(|queue| {
            queue.push_back(msg);
        });
    }

    /// Enqueue a message into the outbound queue and wake the active forwarder.
    /// Returns `true` when the message was queued, `true` when no sender is
    /// registered (no WS client to satisfy), and `false` only when a sender is
    /// registered but its receiver was dropped.
    pub fn enqueue_or_report_disconnect(&self, msg: WsMessage) -> bool {
        self.enqueue_if_connected(|queue| {
            queue.push_back(msg);
        })
        .unwrap_or(true)
    }

    /// Best-effort enqueue of a normal streaming delta.
    pub fn send_stream_delta<F>(&self, msg: AgentStreamDeltaMsg, overflow_snapshot: F)
    where
        F: FnOnce() -> AgentStreamSync,
    {
        let _ = self.enqueue_if_connected(|queue| {
            queue.push_back(WsMessage::AgentStreamDelta(msg));
            if stream_queue_exceeds_limit(queue) {
                let overflow_snapshot = stream_delta_from_sync(overflow_snapshot());
                let key = (
                    overflow_snapshot.session_id.clone(),
                    overflow_snapshot.message_id.clone(),
                );
                collapse_stream_queue_to_snapshot(queue, key, overflow_snapshot);
            }
        });
    }

    /// Best-effort enqueue of a resync snapshot.
    pub fn send_stream_snapshot(&self, msg: AgentStreamSync) {
        let snapshot = stream_delta_from_sync(msg);
        let key = (snapshot.session_id.clone(), snapshot.message_id.clone());
        let _ = self.enqueue_if_connected(|queue| {
            collapse_stream_queue_to_snapshot(queue, key, snapshot);
        });
    }

    /// Drain every queued outbound message in backend enqueue order.
    pub fn drain_messages(&self) -> Vec<WsMessage> {
        self.outbound_queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    pub fn set_sender(&self, sender: Option<WsSender>) {
        let mut guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        let cleared = sender.is_none();
        if sender.is_some() {
            crate::other::telemetry::increment_ws_reconnects();
        }
        *guard = sender;
        if cleared {
            // No active WS session — drop queued messages so a future session
            // does not receive data intended for the prior client.
            self.outbound_queue
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

    fn enqueue_if_connected<F>(&self, enqueue: F) -> Option<bool>
    where
        F: FnOnce(&mut VecDeque<WsMessage>),
    {
        // Hold the sender lock for the entire critical section so a concurrent
        // `set_sender(None)` cannot clear buffers between our sender-present
        // check and queue insert, which would leave stale messages after
        // disconnect.
        let mut sender_guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        let sender = sender_guard.as_ref().cloned()?;
        let mut queue = self
            .outbound_queue
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        enqueue(&mut queue);
        if sender.send(()).is_ok() {
            Some(true)
        } else {
            queue.clear();
            *sender_guard = None;
            Some(false)
        }
    }
}

fn stream_queue_exceeds_limit(queue: &VecDeque<WsMessage>) -> bool {
    let mut stream_count = 0;
    let mut stream_bytes = 0;
    for item in queue {
        if let WsMessage::AgentStreamDelta(msg) = item {
            stream_count += 1;
            stream_bytes += estimate_stream_parts_bytes(&msg.parts);
        }
    }
    stream_count > STREAM_DELTA_QUEUE_LIMIT || stream_bytes > STREAM_DELTA_QUEUE_BYTE_LIMIT
}

fn collapse_stream_queue_to_snapshot(
    queue: &mut VecDeque<WsMessage>,
    key: (String, String),
    snapshot: AgentStreamDeltaMsg,
) {
    let mut collapsed = VecDeque::with_capacity(queue.len());
    let mut first_removed_index = None;

    while let Some(item) = queue.pop_front() {
        if stream_message_matches_key(&item, &key) {
            first_removed_index.get_or_insert(collapsed.len());
        } else {
            collapsed.push_back(item);
        }
    }

    if let Some(index) = first_removed_index {
        crate::other::telemetry::increment_dropped_stream_frames();
        collapsed.insert(index, WsMessage::AgentStreamDelta(snapshot));
    } else {
        collapsed.push_back(WsMessage::AgentStreamDelta(snapshot));
    }

    *queue = collapsed;
}

fn stream_message_matches_key(item: &WsMessage, key: &(String, String)) -> bool {
    matches!(
        item,
        WsMessage::AgentStreamDelta(msg)
            if msg.session_id == key.0 && msg.message_id == key.1
    )
}

fn stream_delta_from_sync(sync: AgentStreamSync) -> AgentStreamDeltaMsg {
    AgentStreamDeltaMsg {
        session_id: sync.session_id,
        message_id: sync.message_id,
        seq: sync.seq,
        snapshot: true,
        parts: sync.parts,
    }
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
    use std::sync::Arc;

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
    fn try_send_queues_pty_output_and_wakes_registered_sender() {
        let broadcaster = WsBroadcaster::default();
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "aaa".to_string(),
            sequence: 1,
        }));

        rx.try_recv().unwrap();
        match &broadcaster.drain_messages()[..] {
            [WsMessage::PtyOutput(msg)] => {
                assert_eq!(msg.pty_id, 1);
                assert_eq!(msg.data, "aaa");
            }
            other => panic!("unexpected messages: {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn outbound_queue_preserves_stream_before_status_order() {
        let broadcaster = WsBroadcaster::default();
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "tail"));
        broadcaster.try_send(WsMessage::AgentStateSync(
            crate::adaptor::protocol::AgentStateSync {
                worktree_path: "/repo".to_string(),
                state: crate::adaptor::protocol::AgentState::Done,
                exit_code: Some(0),
                timestamp: 1.0,
                session_id: Some("S".to_string()),
                pty_id: None,
            },
        ));

        rx.try_recv().unwrap();
        rx.try_recv().unwrap();
        let drained = broadcaster.drain_messages();
        assert!(matches!(
            &drained[0],
            WsMessage::AgentStreamDelta(msg) if msg.seq == 1 && !msg.snapshot
        ));
        assert!(matches!(&drained[1], WsMessage::AgentStateSync(_)));
    }

    #[test]
    fn outbound_queue_preserves_normal_stream_normal_order() {
        let broadcaster = WsBroadcaster::default();
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "before".to_string(),
            sequence: 1,
        }));
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "tail"));
        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "after".to_string(),
            sequence: 2,
        }));

        rx.try_recv().unwrap();
        rx.try_recv().unwrap();
        rx.try_recv().unwrap();
        let drained = broadcaster.drain_messages();
        match &drained[0] {
            WsMessage::PtyOutput(msg) => {
                assert_eq!(msg.pty_id, 1);
                assert_eq!(msg.data, "before");
            }
            other => panic!("unexpected first message: {other:?}"),
        }
        assert!(matches!(
            &drained[1],
            WsMessage::AgentStreamDelta(msg) if msg.seq == 1
        ));
        assert!(matches!(
            &drained[2],
            WsMessage::PtyOutput(msg) if msg.data == "after"
        ));
    }

    #[test]
    fn enqueue_or_report_disconnect_queues_and_wakes_forwarder() {
        let broadcaster = WsBroadcaster::default();
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        let sent = broadcaster.enqueue_or_report_disconnect(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "original".to_string(),
            sequence: 1,
        }));
        assert!(sent, "send must succeed when a receiver is registered");

        rx.try_recv().unwrap();
        assert_eq!(
            broadcaster.drain_messages().len(),
            1,
            "message should be queued for the forwarder"
        );
    }

    #[test]
    fn enqueue_or_report_disconnect_returns_false_when_receiver_dropped() {
        // PTY replay needs a disconnect signal so it can stop replaying when
        // the active WS receiver has already gone away.
        let broadcaster = WsBroadcaster::default();
        let (tx, rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        drop(rx);

        let sent = broadcaster.enqueue_or_report_disconnect(WsMessage::Error(
            crate::adaptor::protocol::ErrorMsg {
                code: "X".to_string(),
                message: "x".to_string(),
            },
        ));
        assert!(
            !sent,
            "send must return false when the receiver has been dropped"
        );
        assert!(
            broadcaster.drain_messages().is_empty(),
            "failed wakeup must not leave stale outbound messages"
        );
    }

    #[test]
    fn enqueue_or_report_disconnect_returns_true_when_no_sender_registered() {
        // No WS client to satisfy → not a delivery failure. `true` lets the
        // caller (PTY replay path) treat the absence of a subscriber as a
        // benign no-op instead of an error.
        let broadcaster = WsBroadcaster::default();
        let sent = broadcaster.enqueue_or_report_disconnect(WsMessage::Error(
            crate::adaptor::protocol::ErrorMsg {
                code: "X".to_string(),
                message: "x".to_string(),
            },
        ));
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
                .map(|i| crate::adaptor::protocol::AgentStreamPartMsg::Text {
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
            snapshot: false,
            parts: vec![crate::adaptor::protocol::AgentStreamPartMsg::Text {
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

        let drained = broadcaster.drain_messages();
        assert_eq!(drained.len(), 2);
        assert!(matches!(
            &drained[0],
            WsMessage::AgentStreamDelta(msg) if msg.seq == 1
        ));
        assert!(matches!(
            &drained[1],
            WsMessage::AgentStreamDelta(msg) if msg.seq == 2
        ));
        assert!(broadcaster.drain_messages().is_empty());
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
            snapshot: false,
            parts: vec![AgentStreamPartMsg::ToolResult {
                content: "preview only".to_string(),
                is_error: false,
                tool_use_id: Some("tool-1".to_string()),
                parent_tool_use_id: None,
                content_ref: Some(crate::adaptor::protocol::AgentToolOutputRefMsg {
                    id: output_id.clone(),
                    byte_size: 4096,
                }),
                summary: Some(crate::adaptor::protocol::AgentToolOutputSummaryMsg {
                    line_count: 1200,
                    byte_size: 4096,
                    is_error: false,
                    truncated: true,
                }),
            }],
        };

        broadcaster.send_stream_delta(delta, || panic!("snapshot should not be built"));

        let drained = broadcaster.drain_messages();
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
        assert_eq!(broadcaster.drain_messages().len(), 1);
    }

    #[test]
    fn send_stream_snapshot_replaces_queued_deltas_at_first_stream_position() {
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "a"));
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M-other", 1, "x"));
        broadcaster.send_stream_snapshot(dummy_stream_sync("S", "M", 2, 2));

        let drained = broadcaster.drain_messages();
        assert_eq!(drained.len(), 2);
        assert!(matches!(
            &drained[0],
            WsMessage::AgentStreamDelta(msg)
                if msg.message_id == "M" && msg.seq == 2 && msg.snapshot
        ));
        assert!(matches!(
            &drained[1],
            WsMessage::AgentStreamDelta(msg) if msg.message_id == "M-other"
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

        let drained = broadcaster.drain_messages();
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            WsMessage::AgentStreamDelta(msg) => {
                assert_eq!(msg.seq, STREAM_DELTA_QUEUE_LIMIT as u64 + 1);
                assert!(msg.snapshot);
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
    fn stream_overflow_snapshot_keeps_position_before_interleaved_status() {
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "a"));
        broadcaster.try_send(WsMessage::AgentStateSync(
            crate::adaptor::protocol::AgentStateSync {
                worktree_path: "/repo".to_string(),
                state: crate::adaptor::protocol::AgentState::Done,
                exit_code: Some(0),
                timestamp: 1.0,
                session_id: Some("S".to_string()),
                pty_id: None,
            },
        ));
        for seq in 2..=(STREAM_DELTA_QUEUE_LIMIT as u64 + 1) {
            broadcaster.send_stream_delta(dummy_stream_delta("S", "M", seq, "x"), || {
                dummy_stream_sync("S", "M", seq, 2)
            });
        }

        let drained = broadcaster.drain_messages();
        assert_eq!(drained.len(), 2);
        assert!(matches!(
            &drained[0],
            WsMessage::AgentStreamDelta(msg)
                if msg.message_id == "M"
                    && msg.seq == STREAM_DELTA_QUEUE_LIMIT as u64 + 1
                    && msg.snapshot
        ));
        assert!(matches!(&drained[1], WsMessage::AgentStateSync(_)));
    }

    #[test]
    fn send_stream_delta_drops_when_no_sender() {
        // Desktop-only / no WS client: the delta is dropped on the floor
        // so the producer's coalescing buffer cannot grow waiting for a
        // non-existent receiver.
        let broadcaster = WsBroadcaster::default();
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "x"));
        assert!(broadcaster.drain_messages().is_empty());
    }

    #[test]
    fn set_sender_none_clears_queued_outbound_messages() {
        // Disconnect must drop stale messages so reconnect doesn't see them.
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        queue_stream_delta(&broadcaster, dummy_stream_delta("S", "M", 1, "x"));
        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "stale".to_string(),
            sequence: 1,
        }));
        broadcaster.set_sender(None);
        assert!(broadcaster.drain_messages().is_empty());
    }

    #[test]
    fn send_stream_delta_does_not_resurrect_queue_after_concurrent_disconnect() {
        // Regression: prior implementation released the sender lock before
        // inserting into the outbound buffer, so a `set_sender(None)` racing
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
                broadcaster.drain_messages().is_empty(),
                "queue must not retain a stream message after disconnect"
            );
        }
    }

    #[tokio::test]
    async fn send_stream_delta_notifies_waiter() {
        // Consumer waits on the wake receiver; producer sends; consumer is woken
        // and drains the queue. Ensures the wake pipeline is connected.
        let broadcaster = Arc::new(WsBroadcaster::default());
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        let consumer = {
            let broadcaster = Arc::clone(&broadcaster);
            tokio::spawn(async move {
                rx.recv().await;
                broadcaster.drain_messages()
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
