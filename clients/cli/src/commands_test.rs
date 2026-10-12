use super::*;

#[test]
fn test_出力設定_起動だけに案内を付けjsonは引数に従う() {
    // Given / When / Then
    for json in [false, true] {
        assert_eq!(
            output_options(&TopCommand::Server {
                command: crate::server::ServerSubcommand::Start { json },
            }),
            (json, Some(crate::client::startup_guidance()))
        );
        assert_eq!(output_options(&TopCommand::Status { json }), (json, None));
    }
    assert_eq!(
        output_options(&TopCommand::Server {
            command: crate::server::ServerSubcommand::Stop,
        }),
        (false, None)
    );
}
#[test]
fn test_購読フィルタ_順番とthreadの重複除去() {
    assert_eq!(
        review_filters(
            Some("a".into()),
            Some("bad-state".into()),
            Some("self".into()),
            Some("true".into()),
            vec!["z".into(), "a".into(), "z".into()]
        ),
        [
            "file=a",
            "state=bad-state",
            "author=self",
            "unread=true",
            "thread=a",
            "thread=z"
        ]
    );
}
#[test]
fn test_hook_payload_境界値と不透明な内容を維持する() {
    assert_eq!(read_payload(&b"not json"[..]).unwrap(), b"not json");
    assert_eq!(read_payload(&vec![0; 65536][..]).unwrap().len(), 65536);
    assert_eq!(
        read_payload(&vec![0; 65537][..]).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
}
