use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyOutputMsg {
    pub pty_id: u64,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyExitMsg {
    pub pty_id: u64,
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_output_msg_roundtrip() {
        let msg = PtyOutputMsg {
            pty_id: 42,
            data: "hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyOutputMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pty_id, 42);
        assert_eq!(deserialized.data, "hello");
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
}
