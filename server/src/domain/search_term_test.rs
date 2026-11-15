use super::*;

#[test]
fn test_検索語_前後の空白を除き空なら検索語にならない() {
    // Given / When / Then
    assert_eq!(SearchTerm::new(""), None);
    assert_eq!(SearchTerm::new("  "), None);
    assert_eq!(SearchTerm::new(" fix ").unwrap().as_str(), "fix");
}

#[test]
fn test_検索語_大文字と小文字を区別せず含むものに一致する() {
    // Given
    let term = SearchTerm::new("Sidebar").unwrap();
    // When / Then
    assert!(term.matches("fix/sidebar-badge"));
    assert!(term.matches("SIDEBAR"));
    assert!(!term.matches("fix/side-bar"));
}
