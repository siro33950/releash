use crate::protocol::WsMessage;
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::mpsc;

pub type WsSender = mpsc::UnboundedSender<WsMessage>;
pub type WsReceiver = mpsc::UnboundedReceiver<WsMessage>;

const PTY_OUTPUT_BUFFER_SIZE: usize = 64 * 1024;

pub struct WsBroadcaster {
    sender: Mutex<Option<WsSender>>,
    pty_output_buffer: Mutex<VecDeque<u8>>,
}

impl Default for WsBroadcaster {
    fn default() -> Self {
        Self {
            sender: Mutex::new(None),
            pty_output_buffer: Mutex::new(VecDeque::new()),
        }
    }
}

impl WsBroadcaster {
    pub fn try_send(&self, msg: WsMessage) {
        if let WsMessage::PtyOutput(ref pty) = msg {
            let mut buf = self
                .pty_output_buffer
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            for byte in pty.data.as_bytes() {
                if buf.len() >= PTY_OUTPUT_BUFFER_SIZE {
                    buf.pop_front();
                }
                buf.push_back(*byte);
            }
        }

        let guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(msg);
        }
    }

    pub fn get_pty_output_buffer(&self) -> String {
        let buf = self
            .pty_output_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let bytes: Vec<u8> = buf.iter().copied().collect();
        String::from_utf8_lossy(&bytes).to_string()
    }

    pub fn set_sender(&self, sender: Option<WsSender>) {
        let mut guard = self.sender.lock().unwrap_or_else(|e| e.into_inner());
        *guard = sender;
    }

    pub fn create_channel() -> (WsSender, WsReceiver) {
        mpsc::unbounded_channel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PtyOutputMsg;

    #[test]
    fn empty_buffer_returns_empty_string() {
        let broadcaster = WsBroadcaster::default();
        assert_eq!(broadcaster.get_pty_output_buffer(), "");
    }

    #[test]
    fn buffer_accumulates_pty_output() {
        let broadcaster = WsBroadcaster::default();
        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "hello".to_string(),
        }));
        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: " world".to_string(),
        }));
        assert_eq!(broadcaster.get_pty_output_buffer(), "hello world");
    }

    #[test]
    fn buffer_ring_evicts_oldest_bytes() {
        let broadcaster = WsBroadcaster::default();
        let chunk = "A".repeat(PTY_OUTPUT_BUFFER_SIZE);
        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: chunk,
        }));
        broadcaster.try_send(WsMessage::PtyOutput(PtyOutputMsg {
            pty_id: 1,
            data: "B".to_string(),
        }));
        let buf = broadcaster.get_pty_output_buffer();
        assert_eq!(buf.len(), PTY_OUTPUT_BUFFER_SIZE);
        assert!(buf.starts_with('A'));
        assert!(buf.ends_with('B'));
    }

    #[test]
    fn non_pty_output_does_not_affect_buffer() {
        let broadcaster = WsBroadcaster::default();
        broadcaster.try_send(WsMessage::Error(crate::protocol::ErrorMsg {
            code: "TEST".to_string(),
            message: "test".to_string(),
        }));
        assert_eq!(broadcaster.get_pty_output_buffer(), "");
    }
}
