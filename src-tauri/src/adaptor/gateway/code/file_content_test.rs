use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::{create_initial_commit, create_test_repo};
#[test]
fn test_ファイル参照_各操作の停止を欠損に変えず探索を終了する() {
    // Given
    let (_dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let oid =
        crate::test_support::git::add_and_commit(&repo, "file", "one\ntwo\n", "file").to_string();
    let file = repo.workdir().unwrap().join("file");
    let file = file.to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| review_blob_at_ref(file, "HEAD"));
    assert_stops_at_each_checkpoint(|| review_blob_at_branch_base(file, Some(&oid)));
    assert_stops_at_each_checkpoint(|| review_blob_staged(file));
    assert_stops_at_each_checkpoint(|| binary_by_attributes(file));
    let missing = repo.workdir().unwrap().join("deleted/child/file");
    assert_stops_at_each_checkpoint(|| review_blob_at_ref(missing.to_str().unwrap(), "HEAD"));
}
