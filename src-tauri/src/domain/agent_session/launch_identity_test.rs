pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn launch_resource_id_is_deterministic_and_request_scoped() {
        let first = launch_resource_id("session", "request-1").unwrap();
        assert_eq!(launch_resource_id("session", "request-1"), Some(first));
        assert_ne!(
            launch_resource_id("session", "request-1"),
            launch_resource_id("session", "request-2")
        );
    }

    #[test]
    fn launch_resource_id_rejects_blank_identity_parts() {
        assert_eq!(launch_resource_id("   ", "request-1"), None);
        assert_eq!(launch_resource_id("session", "   "), None);
    }
}
