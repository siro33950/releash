use super::*;
#[test]
fn test_メッセージ_空白を除き空入力を拒否する() {
    // Given / When / Then
    assert_eq!(
        CommitMessage::new("  title\n\nbody  ").unwrap().as_str(),
        "title\n\nbody"
    );
    assert!(CommitMessage::new(" \n\t").is_err());
}
