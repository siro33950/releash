use super::*;

#[test]
fn test_ターミナル文字復号_未完了連番を次断片まで保持する() {
    let mut pending = Vec::new();

    assert_eq!(decode_utf8_chunk(&[0xE3, 0x81], &mut pending), None);
    assert_eq!(pending.len(), 2);
    assert_eq!(
        decode_utf8_chunk(&[0x82], &mut pending),
        Some("あ".to_string())
    );
    assert!(pending.is_empty());
}

#[test]
fn test_ターミナル出力文字復号_不正バイトの後に続く正常な出力を失わない() {
    let mut pending = Vec::new();

    assert_eq!(
        decode_utf8_chunk(&[0xFF], &mut pending),
        Some("�".to_string())
    );
    assert!(pending.is_empty());
    assert_eq!(
        decode_utf8_chunk(b"visible-output", &mut pending),
        Some("visible-output".to_string())
    );
}
