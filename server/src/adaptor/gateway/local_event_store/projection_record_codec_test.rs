pub(crate) mod tests {
    use super::super::{
        decode_provider_hook_health_projection_record_v1,
        encode_provider_hook_health_projection_record_v1, merge_additive_payload,
    };
    use crate::domain::local_event::{
        AgentSessionProviderRecord, ProviderHookHealthProjectionRecord,
    };

    #[test]
    fn nested_additive_fields_follow_a_unique_semantic_array_identity() {
        let merged = merge_additive_payload(
            r#"{"items":[{"id":"a","known":"old","future":{"flag":true}}]}"#,
            r#"{"items":[{"id":"a","known":"old"}]}"#,
            r#"{"items":[{"id":"a","known":"new"}]}"#,
        )
        .expect("merge");
        let merged: serde_json::Value = serde_json::from_str(&merged).expect("JSON");
        assert_eq!(merged["items"][0]["known"], "new");
        assert_eq!(merged["items"][0]["future"]["flag"], true);
    }

    #[test]
    fn duplicate_kind_keys_never_graft_additive_fields_between_array_entries() {
        let merged = merge_additive_payload(
            r#"{"events":[{"kind":"text","content":"one","future":"first"},{"kind":"text","content":"two","future":"second"}]}"#,
            r#"{"events":[{"kind":"text","content":"one"},{"kind":"text","content":"two"}]}"#,
            r#"{"events":[{"kind":"text","content":"two changed"},{"kind":"text","content":"one changed"}]}"#,
        )
        .expect("merge");
        let merged: serde_json::Value = serde_json::from_str(&merged).expect("JSON");
        assert!(merged["events"][0].get("future").is_none());
        assert!(merged["events"][1].get("future").is_none());
    }

    #[test]
    fn test_provider_hook_health_projection_session_start後の配送失敗warningを保持する() {
        let record = ProviderHookHealthProjectionRecord {
            provider: AgentSessionProviderRecord::Claude,
            latest_launch_id: "launch-1".to_string(),
            latest_launch_session_started: true,
            warning_launch_id: Some("launch-1".to_string()),
            warning_reason: Some("local_api_unavailable".to_string()),
        };

        let encoded = encode_provider_hook_health_projection_record_v1(&record).unwrap();

        assert_eq!(
            decode_provider_hook_health_projection_record_v1(&encoded).unwrap(),
            record
        );
    }
}
