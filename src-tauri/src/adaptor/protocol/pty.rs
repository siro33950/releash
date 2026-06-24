use serde::{Deserialize, Serialize};

use crate::domain::pty_session::PtyEvictReason;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyOutputMsg {
    pub pty_id: u64,
    pub data: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyExitMsg {
    pub pty_id: u64,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PtyEvictReasonMsg {
    Idle,
    CapExceeded,
}

impl From<PtyEvictReason> for PtyEvictReasonMsg {
    fn from(reason: PtyEvictReason) -> Self {
        match reason {
            PtyEvictReason::Idle => Self::Idle,
            PtyEvictReason::CapExceeded => Self::CapExceeded,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyEvictedMsg {
    pub pty_id: u64,
    pub session_key: String,
    pub reason: PtyEvictReasonMsg,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_output_msg_roundtrip() {
        let msg = PtyOutputMsg {
            pty_id: 42,
            data: "hello".to_string(),
            sequence: 7,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyOutputMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pty_id, 42);
        assert_eq!(deserialized.data, "hello");
        assert_eq!(deserialized.sequence, 7);
    }

    #[test]
    fn test_pty_exit_msg_roundtrip() {
        let msg = PtyExitMsg {
            pty_id: 1,
            exit_code: Some(0),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyExitMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exit_code, Some(0));
    }

    #[test]
    fn test_pty_exit_msg_none_exit_code() {
        let msg = PtyExitMsg {
            pty_id: 1,
            exit_code: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyExitMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exit_code, None);
    }

    #[test]
    fn test_pty_evicted_msg_roundtrip() {
        let msg = PtyEvictedMsg {
            pty_id: 1,
            session_key: "key".to_string(),
            reason: PtyEvictReasonMsg::CapExceeded,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"reason\":\"cap_exceeded\""));
        let deserialized: PtyEvictedMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pty_id, 1);
        assert_eq!(deserialized.session_key, "key");
        assert_eq!(deserialized.reason, PtyEvictReasonMsg::CapExceeded);
    }

    #[test]
    fn pty_evict_reason_serializes_as_snake_case_wire_value() {
        assert_eq!(
            serde_json::to_value(PtyEvictReasonMsg::from(PtyEvictReason::Idle)).unwrap(),
            "idle"
        );
        assert_eq!(
            serde_json::to_value(PtyEvictReasonMsg::from(PtyEvictReason::CapExceeded)).unwrap(),
            "cap_exceeded"
        );
    }
}
