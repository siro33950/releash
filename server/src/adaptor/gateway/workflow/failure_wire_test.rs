pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn failure_kind_wire_serde_uses_stable_strings() {
        let json = serde_json::to_string(&NodeExecutionFailureKind::ModelRefusal).unwrap();
        assert_eq!(json, "\"model_refusal\"");
        let back: NodeExecutionFailureKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, NodeExecutionFailureKind::ModelRefusal);
    }

    #[test]
    fn failure_disposition_wire_serde_uses_stable_strings() {
        let json = serde_json::to_string(&FailureDisposition::UserActionRequired).unwrap();
        assert_eq!(json, "\"user-action-required\"");
        let back: FailureDisposition = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FailureDisposition::UserActionRequired);
    }
}
