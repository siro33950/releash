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
            let mut buf = self.pty_output_buffer.lock().unwrap_or_else(|e| e.into_inner());
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

    pub fn take_pty_output_buffer(&self) -> String {
        let buf = self.pty_output_buffer.lock().unwrap_or_else(|e| e.into_inner());
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
