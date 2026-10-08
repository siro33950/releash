pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn usage_events_are_allowlisted() {
        assert!(usage_event_allowed("settings_saved"));
        assert!(!usage_event_allowed("repo_path_added"));
    }
}
