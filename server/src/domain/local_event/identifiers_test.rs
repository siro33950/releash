pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn stream_id_namespaces() {
        let provider_lifecycle = StreamId::provider_lifecycle("s-1").unwrap();
        assert_eq!(provider_lifecycle.as_str(), "provider-lifecycle:s-1");
        assert_eq!(StreamId::application().as_str(), "application");
        let ownership = StreamId::provider_session_ownership("codex", "ownership-digest").unwrap();
        assert_eq!(
            ownership.as_str(),
            "provider-session-ownership:codex:ownership-digest"
        );
        assert!(StreamId::parse("agent-session:").is_err());
        assert!(StreamId::parse("other:x").is_err());
        assert!(StreamId::parse("agent-session:a b").is_err());
    }

    #[test]
    fn identity_bounds() {
        assert!(CommitIdentity::parse("").is_err());
        assert!(CommitIdentity::parse(&"a".repeat(128)).is_ok());
        assert!(CommitIdentity::parse(&"a".repeat(129)).is_err());
        assert!(CommitIdentity::parse("ok._:-09AZ").is_ok());
        assert!(CommitIdentity::parse("no/slash").is_err());
    }

    #[test]
    fn sequence_bounds() {
        assert!(GlobalSequence::new(0).is_err());
        assert!(GlobalSequence::new(1).is_ok());
        assert!(StreamVersion::new(-1).is_err());
        assert_eq!(StreamVersion::zero().value(), 0);
    }
}
