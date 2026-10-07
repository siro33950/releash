use super::Compatibility;

#[test]
fn test_互換判定_protocolの大小で古い側を返す() {
    assert_eq!(Compatibility::assess(1, 1), Compatibility::Compatible);
    assert_eq!(Compatibility::assess(2, 1), Compatibility::ServerOlder);
    assert_eq!(Compatibility::assess(1, 2), Compatibility::ClientOlder);
}
