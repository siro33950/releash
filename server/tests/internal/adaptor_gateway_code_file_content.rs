use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;

pub(crate) mod file_content_gateway_tests {
    use super::*;

    use crate::test_support_git::*;
    use git2::Repository;
    use releashd::test_support::integration::code::FileContentGateway;
    use releashd::test_support::integration::repository::FileContentRepository;
    use releashd::test_support::integration::repository::ReviewSideBytes;
    use releashd::test_support::integration::repository::ReviewSideMetadata;

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

    #[test]
    fn test_内容同一性_初回commit前のhead欠損は作業ツリーとの不一致を返す() {
        // Given
        let (_dir, repo) = create_test_repo();
        let file = workdir_file(&repo, "new.bin");
        std::fs::write(&file, "new").unwrap();
        // When
        let matches = FileContentGateway
            .review_working_tree_matches_ref(&file, "HEAD")
            .unwrap();
        // Then
        assert!(!matches);
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

    #[cfg(unix)]
    #[test]
    fn test_作業ツリー読取_fifoを開かず欠損として返す() {
        // Given
        let (_dir, repo) = create_test_repo();
        let fifo = repo.workdir().unwrap().join("pipe");
        let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        let gateway = FileContentGateway;
        // When / Then
        assert_eq!(
            gateway
                .review_working_tree_metadata(fifo.to_str().unwrap())
                .unwrap(),
            ReviewSideMetadata::Missing
        );
        assert_eq!(
            gateway
                .review_working_tree_bytes(fifo.to_str().unwrap())
                .unwrap(),
            ReviewSideBytes::Missing
        );
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
