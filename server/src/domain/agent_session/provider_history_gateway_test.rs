use super::*;
#[test]
fn test_履歴検索_題と最初の指示を照合しidは含めない() {
    // Given
    let entry = ProviderSessionTitleEntry {
        provider_session_id: "secret".into(),
        session_title: Some("Thread".into()),
        first_user_prompt: Some("Commit menu".into()),
    };
    // When / Then
    for (text, expected) in [("thread", true), ("COMMIT", true), ("secret", false)] {
        assert_eq!(
            entry.matches(&crate::domain::search_term::SearchTerm::new(text).unwrap()),
            expected
        );
    }
}
