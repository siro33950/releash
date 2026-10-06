pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn test_provider活動request_json往復で活動状態とsession参照を保持する() {
        for (activity, expected) in [
            (ProviderActivityRequest::Working, "working"),
            (ProviderActivityRequest::AwaitingAnswer, "awaiting_answer"),
            (
                ProviderActivityRequest::AwaitingInstruction,
                "awaiting_instruction",
            ),
        ] {
            let request = ProviderLifecycleSignalRequest::ActivityObserved {
                provider_session_id: "provider-session-1".to_string(),
                transcript_ref: Some("provider://transcript".to_string()),
                activity,
            };
            let encoded = serde_json::to_value(&request).unwrap();

            assert_eq!(
                encoded,
                serde_json::json!({
                    "event": "activity_observed",
                    "provider_session_id": "provider-session-1",
                    "transcript_ref": "provider://transcript",
                    "activity": expected,
                })
            );
            assert_eq!(
                serde_json::from_value::<ProviderLifecycleSignalRequest>(encoded).unwrap(),
                request
            );
        }
    }
}
