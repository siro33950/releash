use super::*;
fn comment(time: u64, body: &str) -> PrComment {
    PrComment {
        author: PrCommentAuthor {
            login: "author".into(),
            is_bot: false,
        },
        written_at: time,
        body: body.into(),
    }
}
#[test]
fn test_comment一覧_空レビューを除いて時刻順に混ぜる() {
    // Given / When
    let conversation = PrConversation::new(
        vec![comment(3, "last"), comment(1, "")],
        vec![comment(2, "review"), comment(0, "")],
    );
    // Then
    assert_eq!(
        conversation
            .entries()
            .iter()
            .map(|c| c.written_at)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}
#[test]
fn test_comment入力_空を拒み本文を保持する() {
    // Given / When / Then
    assert_eq!(PrCommentText::new("".into()), Err(PrCommentError::Empty));
    assert_eq!(
        PrCommentText::new(" body ".into()).unwrap().as_str(),
        " body "
    );
}
