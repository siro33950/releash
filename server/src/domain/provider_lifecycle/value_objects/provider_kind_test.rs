use super::ProviderKind;

#[test]
fn test_provider表示名_各providerの利用者向けの名前を返す() {
    // Given
    let providers = [ProviderKind::Claude, ProviderKind::Codex];

    // When
    let names = providers.map(ProviderKind::display_name);

    // Then
    assert_eq!(names, ["Claude", "Codex"]);
}
