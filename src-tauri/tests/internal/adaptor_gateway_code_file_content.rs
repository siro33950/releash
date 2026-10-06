use releash_lib::test_support::integration::code::binary_by_attributes;
use releash_lib::test_support::integration::code::review_blob_at_branch_base;
use releash_lib::test_support::integration::code::review_blob_at_ref;
use releash_lib::test_support::integration::code::review_blob_staged;

use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;

#[test]
pub fn test_ファイル参照_各操作の停止を欠損に変えず探索を終了する() {
    // Given
    let (_dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let oid =
        crate::test_support_git::add_and_commit(&repo, "file", "one\ntwo\n", "file").to_string();
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
pub(crate) mod file_content_gateway_tests {
    use super::*;

    use crate::test_support_git::*;
    use git2::Repository;
    use releash_lib::test_support::integration::code::FileContentGateway;
    use releash_lib::test_support::integration::repository::FileContentRepository;
    use releash_lib::test_support::integration::repository::ReviewSideBytes;
    use releash_lib::test_support::integration::repository::ReviewSideMetadata;

    /// macOS では TempDir::path() が /var/... を返すが workdir() は /private/var/... を返す。
    /// strip_prefix の不一致を防ぐため workdir() ベースでファイルパスを構築する。
    fn workdir_file(repo: &Repository, name: &str) -> String {
        repo.workdir()
            .unwrap()
            .join(name)
            .to_str()
            .unwrap()
            .to_string()
    }

    /// base ブランチの commit OID(hex) を返す（gateway は解決済み OID を受け取る契約）。
    fn base_commit_oid(repo: &Repository) -> String {
        repo.head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .id()
            .to_string()
    }

    #[test]
    pub fn review_metadata_at_ref_missing_blob_returns_missing() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "tracked.txt", "content\n", "add tracked");

        let gateway = FileContentGateway;
        let result = gateway
            .review_file_metadata_at_ref(&workdir_file(&repo, "missing.txt"), "HEAD")
            .unwrap();

        assert_eq!(result, ReviewSideMetadata::Missing);
    }

    #[test]
    pub fn review_at_head_on_unborn_branch_returns_missing() {
        let (_dir, repo) = create_test_repo();
        let file_path = workdir_file(&repo, "new.txt");
        std::fs::write(&file_path, "new\n").unwrap();

        let gateway = FileContentGateway;
        let metadata = gateway
            .review_file_metadata_at_ref(&file_path, "HEAD")
            .unwrap();
        let bytes = gateway
            .review_file_bytes_at_ref(&file_path, "HEAD")
            .unwrap();

        assert_eq!(metadata, ReviewSideMetadata::Missing);
        assert_eq!(bytes, ReviewSideBytes::Missing);
    }

    #[test]
    pub fn review_working_tree_metadata_uses_metadata_size() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let file_path = workdir_file(&repo, "large.txt");
        std::fs::write(&file_path, vec![b'x'; 1_048_577]).unwrap();

        let gateway = FileContentGateway;
        let result = gateway.review_working_tree_metadata(&file_path).unwrap();

        assert_eq!(
            result,
            ReviewSideMetadata::Present {
                size_bytes: 1_048_577
            }
        );
    }

    #[test]
    pub fn review_at_ref_reads_deleted_file_after_parent_directories_are_removed() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        std::fs::create_dir_all(repo.workdir().unwrap().join("src/nested")).unwrap();
        add_and_commit(
            &repo,
            "src/nested/file.txt",
            "original content\n",
            "add nested file",
        );
        let base_oid = base_commit_oid(&repo);
        std::fs::remove_dir_all(repo.workdir().unwrap().join("src")).unwrap();
        let file_path = workdir_file(&repo, "src/nested/file.txt");

        let gateway = FileContentGateway;
        let metadata = gateway
            .review_file_metadata_at_ref(&file_path, "HEAD")
            .unwrap();
        let bytes = gateway
            .review_file_bytes_at_ref(&file_path, "HEAD")
            .unwrap();
        let base_metadata = gateway
            .review_file_metadata_at_branch_base(&file_path, Some(&base_oid))
            .unwrap();
        let base_bytes = gateway
            .review_file_bytes_at_branch_base(&file_path, Some(&base_oid))
            .unwrap();

        assert_eq!(metadata, ReviewSideMetadata::Present { size_bytes: 17 });
        assert_eq!(
            bytes,
            ReviewSideBytes::Present(b"original content\n".to_vec())
        );
        assert_eq!(
            base_metadata,
            ReviewSideMetadata::Present { size_bytes: 17 }
        );
        assert_eq!(
            base_bytes,
            ReviewSideBytes::Present(b"original content\n".to_vec())
        );
    }

    #[test]
    pub fn review_working_tree_regular_file_still_reads_metadata_and_bytes() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let file_path = workdir_file(&repo, "regular.txt");
        std::fs::write(&file_path, "regular content").unwrap();

        let gateway = FileContentGateway;
        let metadata = gateway.review_working_tree_metadata(&file_path).unwrap();
        let bytes = gateway.review_working_tree_bytes(&file_path).unwrap();

        assert_eq!(metadata, ReviewSideMetadata::Present { size_bytes: 15 });
        assert_eq!(bytes, ReviewSideBytes::Present(b"regular content".to_vec()));
    }

    #[cfg(unix)]
    #[test]
    pub fn review_working_tree_symlink_does_not_return_target_metadata_or_bytes() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let outside_dir = tempfile::tempdir().unwrap();
        let outside_file = outside_dir.path().join("outside.txt");
        std::fs::write(&outside_file, "outside secret").unwrap();
        let link_path = repo.workdir().unwrap().join("linked.txt");
        std::os::unix::fs::symlink(&outside_file, &link_path).unwrap();

        let gateway = FileContentGateway;
        let metadata = gateway
            .review_working_tree_metadata(link_path.to_str().unwrap())
            .unwrap();
        let bytes = gateway
            .review_working_tree_bytes(link_path.to_str().unwrap())
            .unwrap();

        assert_eq!(metadata, ReviewSideMetadata::Missing);
        assert_eq!(bytes, ReviewSideBytes::Missing);
    }

    #[test]
    pub fn review_binary_by_attributes_detects_minus_diff_text_file() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        std::fs::write(workdir_file(&repo, ".gitattributes"), "*.txt -diff\n").unwrap();
        let file_path = workdir_file(&repo, "data.txt");
        std::fs::write(&file_path, "plain utf8\n").unwrap();

        let gateway = FileContentGateway;
        assert!(gateway.review_binary_by_attributes(&file_path).unwrap());
    }
}
