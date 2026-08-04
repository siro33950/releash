use super::{TerminalSurfaceAttachment, TerminalSurfaceSequenceDecision};

fn attachment(snapshot_sequence: u64) -> TerminalSurfaceAttachment {
    TerminalSurfaceAttachment::new("attachment-1".to_string(), snapshot_sequence)
}

#[test]
fn test_ターミナル画面接続_接続idで識別する() {
    let attachment = attachment(4);

    assert_eq!(attachment.attachment_id(), "attachment-1");
}

#[test]
fn test_ターミナル画面接続順序_連続した連番だけを配信する() {
    let mut attachment = attachment(4);

    assert_eq!(
        attachment.observe(5, false),
        TerminalSurfaceSequenceDecision::Deliver
    );
    assert_eq!(
        attachment.observe(6, false),
        TerminalSurfaceSequenceDecision::Deliver
    );
}

#[test]
fn test_ターミナル画面接続順序_重複と逆行を無視して欠落を再同期する() {
    let mut attachment = attachment(4);

    assert_eq!(
        attachment.observe(4, false),
        TerminalSurfaceSequenceDecision::Ignore
    );
    assert_eq!(
        attachment.observe(3, false),
        TerminalSurfaceSequenceDecision::Ignore
    );
    assert_eq!(
        attachment.observe(6, false),
        TerminalSurfaceSequenceDecision::Resynchronize
    );
}

#[test]
fn test_ターミナル画面再同期_画面写像が欠落連番を包含しなければ接続を閉じる() {
    let mut attachment = attachment(4);

    assert!(!attachment.apply_snapshot(5, Some(6), false));
    assert!(attachment.is_closed());
    assert_eq!(
        attachment.observe(6, false),
        TerminalSurfaceSequenceDecision::Closed
    );
}

#[test]
fn test_ターミナル画面接続終了_終了を配信した後は接続を閉じる() {
    let mut attachment = attachment(4);

    assert_eq!(
        attachment.observe(5, true),
        TerminalSurfaceSequenceDecision::Deliver
    );
    assert!(attachment.is_closed());
    assert_eq!(
        attachment.observe(6, false),
        TerminalSurfaceSequenceDecision::Closed
    );
}

#[test]
fn test_ターミナル画面再同期_現在より古い画面写像へ連番を逆行させない() {
    let mut attachment = attachment(8);

    assert!(!attachment.apply_snapshot(7, None, false));
    assert_eq!(
        attachment.observe(9, false),
        TerminalSurfaceSequenceDecision::Deliver
    );
}

#[test]
fn test_ターミナル画面接続終了_閉じた接続を画面写像で再開しない() {
    let mut attachment = attachment(4);
    attachment.close();

    assert!(!attachment.apply_snapshot(5, None, false));
    assert!(attachment.is_closed());
    assert_eq!(
        attachment.observe(6, false),
        TerminalSurfaceSequenceDecision::Closed
    );
}
