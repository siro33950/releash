use super::*;

#[test]
fn test_実行木読取保持_未使用のfoldを期限で破棄する() {
    // Given
    let records =
        crate::domain::workflow::services::test_helpers::isolated_session_timestamp_facts();
    let (fold, folded) = TreeFold::from_records(&records[0].meta.tree_id, &records)
        .unwrap()
        .unwrap();
    let mut tree = HeldTree {
        fold: Some(HeldFold {
            fold,
            read_through: records.last().unwrap().seq,
            terminal: false,
            view: folded_execution(folded),
            used_at: std::time::Instant::now(),
        }),
        ..Default::default()
    };
    // When / Then
    tree.expire(FOLD_IDLE_LIMIT);
    assert!(tree.fold.is_some());
    tree.fold.as_mut().unwrap().used_at -= std::time::Duration::from_secs(601);
    tree.expire(FOLD_IDLE_LIMIT);
    assert!(tree.fold.is_none());
}
