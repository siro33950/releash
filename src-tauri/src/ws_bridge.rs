use crate::protocol::{AgentStreamSync, WsMessage};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, Notify};

pub type WsSender = mpsc::UnboundedSender<WsMessage>;
pub type WsReceiver = mpsc::UnboundedReceiver<WsMessage>;

pub struct WsBroadcaster {
    sender: Mutex<Option<WsSender>>,
    /// Latest cumulative `AgentStreamSync` snapshot per
    /// `(chat_session_id, message_id)`. The producer writes a fresh snapshot
    /// here on every flush; the consumer drains the slot when woken. This
    /// keeps the WS queue from accumulating O(N) cumulative payloads when
    /// the receiver is slow — only the most recent snapshot is retained per
    /// message, so memory is bounded by (number of live streaming messages
    /// × one snapshot).
    latest_stream_sync: Mutex<HashMap<(String, String), AgentStreamSync>>,
    /// Wakeup for the consumer to drain `latest_stream_sync`. `Notify`
    /// collapses multiple producer signals into a single permit, so a burst
    /// of producer updates yields at most one drain pass.
    stream_sync_notify: Arc<Notify>,
}

impl Default for WsBroadcaster {
    fn default() -> Self {
        Self {
            sender: Mutex::new(None),
            latest_stream_sync: Mutex::new(HashMap::new()),
            stream_sync_notify: Arc::new(Notify::new()),
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

    /// Best-effort enqueue of an `AgentStreamSync` cumulative snapshot.
    ///
    /// The new snapshot replaces any prior unsent snapshot for the same
    /// `(chat_session_id, message_id)` so a slow WS receiver cannot
    /// accumulate stacks of full-content payloads in the channel — only the
    /// most recent cumulative is retained per message. When no sender is
    /// registered the snapshot is simply dropped (no client to satisfy);
    /// reconnect must refetch fresh state via the agent session query path
    /// rather than replay stale slot data.
    ///
    /// No return value: the slot write itself cannot fail, and downstream
    /// transport failure (WS receiver gone) is recovered by the next flush
    /// re-sending the cumulative `streaming_parts` — which is what the
    /// producer's coalescer relies on. Reporting a "ws_ok" boolean here
    /// would be misleading because there is no production path that can
    /// observe it as `false`.
    pub fn send_stream_sync(&self, msg: AgentStreamSync) {
        // Hold the sender lock for the entire critical section so a concurrent
        // `set_sender(None)` cannot clear `latest_stream_sync` between our
        // sender-present check and the slot insert (which would leave a stale
        // snapshot in the slot after disconnect). Lock order is sender → slot,
        // matching `set_sender`.
        let sender_guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        if sender_guard.is_none() {
            return;
        }
        {
            let mut slot = self
                .latest_stream_sync
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if slot
                .insert((msg.session_id.clone(), msg.message_id.clone()), msg)
                .is_some()
            {
                crate::other::telemetry::increment_dropped_stream_frames();
            }
        }
        drop(sender_guard);
        self.stream_sync_notify.notify_one();
    }

    /// Drain every queued `AgentStreamSync` snapshot and reset the slot to
    /// empty. The returned vector contains at most one cumulative snapshot
    /// per `(chat_session_id, message_id)` (latest write wins) — order
    /// across distinct messages is not preserved, which is safe because each
    /// snapshot is itself a complete replacement payload that the receiver
    /// applies independently per message.
    ///
    /// Called by the WS forward task on each `stream_sync_notify` wakeup.
    pub fn drain_stream_sync(&self) -> Vec<AgentStreamSync> {
        let mut slot = self
            .latest_stream_sync
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        slot.drain().map(|(_, v)| v).collect()
    }

    /// Notify handle for the WS forward task. The task `select!`s on this
    /// alongside the regular `WsReceiver` and, on wakeup, calls
    /// `drain_stream_sync` to forward queued snapshots to the WS client.
    pub fn stream_sync_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.stream_sync_notify)
    }

    pub fn set_sender(&self, sender: Option<WsSender>) {
        let mut guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        let cleared = sender.is_none();
        if sender.is_some() {
            crate::other::telemetry::increment_ws_reconnects();
        }
        *guard = sender;
        if cleared {
            // No active WS session — drop the queued cumulative snapshots so
            // a future session does not start by receiving a snapshot the
            // prior client was the intended recipient of.
            self.latest_stream_sync
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

    fn dummy_stream_sync(session: &str, message: &str, n: usize) -> AgentStreamSync {
        AgentStreamSync {
            session_id: session.to_string(),
            message_id: message.to_string(),
            parts: (0..n)
                .map(|i| crate::protocol::AgentStreamPartMsg::Text {
                    content: format!("p{i}"),
                    parent_tool_use_id: None,
                })
                .collect(),
        }
    }

    #[test]
    fn send_stream_sync_replaces_prior_snapshot_for_same_message() {
        // Slow consumer scenario: producer pushes many cumulative snapshots
        // before the consumer drains. Only the newest snapshot must remain.
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        for n in 1..=10 {
            broadcaster.send_stream_sync(dummy_stream_sync("S", "M", n));
        }

        let drained = broadcaster.drain_stream_sync();
        assert_eq!(drained.len(), 1, "only the latest snapshot is retained");
        assert_eq!(drained[0].parts.len(), 10);

        // After drain, the slot is empty.
        assert!(broadcaster.drain_stream_sync().is_empty());
    }

    #[test]
    fn send_stream_sync_keeps_distinct_messages_separately() {
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        broadcaster.send_stream_sync(dummy_stream_sync("S", "M1", 1));
        broadcaster.send_stream_sync(dummy_stream_sync("S", "M2", 2));

        let mut drained = broadcaster.drain_stream_sync();
        drained.sort_by(|a, b| a.message_id.cmp(&b.message_id));
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].message_id, "M1");
        assert_eq!(drained[1].message_id, "M2");
    }

    #[test]
    fn send_stream_sync_drops_snapshot_when_no_sender() {
        // Desktop-only / no WS client: the snapshot is dropped on the floor
        // (no slot retention) so the producer's coalescing buffer cannot grow
        // unbounded waiting for a non-existent receiver.
        let broadcaster = WsBroadcaster::default();
        broadcaster.send_stream_sync(dummy_stream_sync("S", "M", 1));
        // No client → no snapshot retained.
        assert!(broadcaster.drain_stream_sync().is_empty());
    }

    #[test]
    fn set_sender_none_clears_queued_stream_sync_snapshots() {
        // Disconnect must drop stale snapshots so reconnect doesn't see them.
        let broadcaster = WsBroadcaster::default();
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        broadcaster.send_stream_sync(dummy_stream_sync("S", "M", 1));
        broadcaster.set_sender(None);
        assert!(broadcaster.drain_stream_sync().is_empty());
    }

    #[test]
    fn send_stream_sync_does_not_resurrect_slot_after_concurrent_disconnect() {
        // Regression: prior implementation released the sender lock before
        // inserting into `latest_stream_sync`, so a `set_sender(None)` racing
        // between the sender check and the slot insert could leave a stale
        // snapshot in the slot. With the sender lock held across the whole
        // critical section, the two operations are serialised: either the
        // disconnect wins (insert observes `sender_guard.is_none()` and
        // returns early) or the send wins (slot has the snapshot and is
        // cleared by the subsequent disconnect). Either ordering must leave
        // the slot empty after disconnect.
        for _ in 0..200 {
            let broadcaster = Arc::new(WsBroadcaster::default());
            let (tx, _rx) = WsBroadcaster::create_channel();
            broadcaster.set_sender(Some(tx));

            let producer = {
                let broadcaster = Arc::clone(&broadcaster);
                std::thread::spawn(move || {
                    broadcaster.send_stream_sync(dummy_stream_sync("S", "M", 1));
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
                broadcaster.drain_stream_sync().is_empty(),
                "slot must not retain a snapshot after disconnect"
            );
        }
    }

    #[tokio::test]
    async fn send_stream_sync_notifies_waiter() {
        // Consumer waits on `stream_sync_notify`; producer sends; consumer is
        // woken and drains the slot. Ensures the notify pipeline is connected.
        let broadcaster = Arc::new(WsBroadcaster::default());
        let (tx, _rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        let notify = broadcaster.stream_sync_notify();
        let consumer = {
            let broadcaster = Arc::clone(&broadcaster);
            tokio::spawn(async move {
                notify.notified().await;
                broadcaster.drain_stream_sync()
            })
        };
        broadcaster.send_stream_sync(dummy_stream_sync("S", "M", 3));
        let drained = consumer.await.unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].parts.len(), 3);
    }
}
