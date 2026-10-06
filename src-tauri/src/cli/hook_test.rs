use super::*;
use std::io::Cursor;

#[test]
fn test_hook受信_上限を超えるstdinを拒否する() {
    let payload = vec![b'x'; 65_537];

    let error = receive_from(Cursor::new(payload), HookProvider::Claude).unwrap_err();

    assert_eq!(
        error,
        CliError::InvalidInput(
            "Provider lifecycle payload exceeds the 65536 byte limit".to_string()
        )
    );
}

#[test]
fn test_hook受信_cli入口はpayload不正や配送失敗でも空jsonを返す() {
    assert_eq!(
        complete_receive(Err(CliError::InvalidInput("invalid payload".to_string()))).unwrap(),
        "{}"
    );
    assert_eq!(
        complete_receive(Err(CliError::Other("rejected".to_string()))).unwrap(),
        "{}"
    );
    assert_eq!(complete_receive(Ok("{}".to_string())).unwrap(), "{}");
}
