use releash_lib::test_support::integration::code::CodeError;
use releash_lib::test_support::integration::code::StagingGateway;
use releash_lib::test_support::integration::platform::Deadline;
use releash_lib::test_support::integration::platform::OperationContext;
use releash_lib::test_support::integration::platform::OperationStopped;
use releash_lib::test_support::integration::repository::StagingRepository;
use std::time::Duration;
use std::time::Instant;

use releash_lib::test_support::integration::code::set_git_program;

#[cfg(unix)]
#[tokio::test]
pub async fn test_hunk変更_実行中のprocessを期限と取消で回収し停止分類を返す() {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            set_git_program(None);
        }
    }
    // Given
    for reverse in [false, true] {
        for expire in [false, true] {
            let (dir, _repo) = crate::test_support_git::create_test_repo();
            let program = dir.path().join("fake-git");
            std::fs::write(
                &program,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > args\necho $$ > pid\nexec sleep 30\n",
            )
            .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            set_git_program(Some(program));
            let _reset = Reset;
            let token = tokio_util::sync::CancellationToken::new();
            let context = OperationContext::new(
                expire.then(|| Deadline::new(Instant::now() + Duration::from_secs(1))),
                Arc::new(token.clone()),
            );
            let pid_file = dir.path().join("pid");
            let ready = pid_file.clone();
            let signal = std::thread::spawn(move || {
                let start = Instant::now();
                while std::fs::read_to_string(&ready)
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    assert!(
                        start.elapsed() < Duration::from_secs(5),
                        "git did not start"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                if !expire {
                    token.cancel();
                }
            });
            // When
            let result = releash_lib::test_support::integration::platform::scope(context, async {
                if reverse {
                    StagingGateway
                        .unstage_hunk(dir.path().to_str().unwrap(), "patch")
                        .await
                } else {
                    StagingGateway
                        .stage_hunk(dir.path().to_str().unwrap(), "patch")
                        .await
                }
            })
            .await;
            signal.join().unwrap();
            // Then
            assert!(
                matches!(result, Err(CodeError::Technical(error)) if error == if expire { OperationStopped::Expired.into() } else { OperationStopped::Cancelled.into() })
            );
            let pid: i32 = std::fs::read_to_string(pid_file)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join("args")).unwrap(),
                if reverse {
                    "apply\n--cached\n--reverse\n"
                } else {
                    "apply\n--cached\n"
                }
            );
        }
    }
}
pub(crate) mod staging_gateway_tests {

    use git2::Repository;
    use releash_lib::test_support::integration::code::git_stage;
    use releash_lib::test_support::integration::code::git_stage_hunk;
    use releash_lib::test_support::integration::code::git_unstage;
    use releash_lib::test_support::integration::code::git_unstage_hunk;

    use releash_lib::test_support::integration::repository::get_git_status;

    use crate::test_support_git::*;
    use releash_lib::test_support::integration::code::ChangeGroup;
    use releash_lib::test_support::integration::code::Hunk;
    use std::fs;
    use std::path::Path;

    fn index_file_content(repo: &Repository, path: &str) -> String {
        let mut index = repo.index().unwrap();
        index.read(true).unwrap();
        let entry = index.get_path(Path::new(path), 0).unwrap();
        let blob = repo.find_blob(entry.id).unwrap();
        std::str::from_utf8(blob.content()).unwrap().to_string()
    }

    fn diff_hunks_and_groups(original: &str, modified: &str) -> (Vec<Hunk>, Vec<ChangeGroup>) {
        let raw_hunks = releash_lib::test_support::integration::code::diff_buffers(
            original,
            modified,
            Some("file.txt"),
        )
        .unwrap();
        let hunks = releash_lib::test_support::integration::code::assign_hunk_ids(&raw_hunks);
        let groups = releash_lib::test_support::integration::code::compute_change_groups(&hunks);
        (hunks, groups)
    }

    fn group_patch(file_path: &str, hunks: &[Hunk], group: &ChangeGroup) -> String {
        let hunk = hunks
            .iter()
            .find(|hunk| hunk.index == group.hunk_index)
            .unwrap();
        releash_lib::test_support::integration::code::generate_group_patch(file_path, hunk, group)
    }

    #[tokio::test]
    pub async fn test_stage_特定ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("new.txt"), "hello").unwrap();

        git_stage(dir.path().to_str().unwrap(), vec!["new.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "new");
        assert_eq!(statuses[0].worktree_status, "none");
    }

    #[tokio::test]
    pub async fn test_stage_全ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();

        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 2);
        for s in &statuses {
            assert_eq!(s.index_status, "new");
            assert_eq!(s.worktree_status, "none");
        }
    }

    #[tokio::test]
    pub async fn test_stage_削除ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");
        fs::remove_file(dir.path().join("file.txt")).unwrap();

        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "deleted");
        assert_eq!(statuses[0].worktree_status, "none");
    }

    #[tokio::test]
    pub async fn test_stage_未追跡ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("untracked.txt"), "data").unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(before[0].worktree_status, "new");

        git_stage(
            dir.path().to_str().unwrap(),
            vec!["untracked.txt".to_string()],
        )
        .unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(after[0].index_status, "new");
        assert_eq!(after[0].worktree_status, "none");
    }

    #[tokio::test]
    pub async fn test_unstage_特定ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        git_unstage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].worktree_status, "new");
        assert_eq!(statuses[0].index_status, "none");
    }

    #[tokio::test]
    pub async fn test_unstage_全ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();

        git_unstage(dir.path().to_str().unwrap(), vec![]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        for s in &statuses {
            assert_eq!(s.index_status, "none");
            assert_eq!(s.worktree_status, "new");
        }
    }

    #[tokio::test]
    pub async fn test_unstage_unborn_branch() {
        let (dir, _repo) = create_test_repo();
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(before[0].index_status, "new");

        git_unstage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(after[0].index_status, "none");
        assert_eq!(after[0].worktree_status, "new");
    }

    #[tokio::test]
    pub async fn test_stage_hunk() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        fs::write(dir.path().join("file.txt"), "line1\nmodified\nline3\n").unwrap();

        let patch =
            "--- a/file.txt\n+++ b/file.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+modified\n line3\n";

        git_stage_hunk(dir.path().to_str().unwrap(), patch)
            .await
            .unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert!(statuses.iter().any(|s| s.index_status == "modified"));
    }

    #[tokio::test]
    pub async fn test_stage_hunk_連続適用はstaged内容で再計算したpatchなら成功する() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let original = "line1\nline2\nline3\nline4\n";
        let modified = "line1\nchanged2\nline3\nchanged4\n";
        add_and_commit(&repo, "file.txt", original, "add file");
        fs::write(dir.path().join("file.txt"), modified).unwrap();

        let (hunks, groups) =
            diff_hunks_and_groups(&index_file_content(&repo, "file.txt"), modified);
        assert_eq!(groups.len(), 2);
        let first_group = groups[0].clone();
        let second_group_id = groups[1].group_id.clone();

        let first_patch = group_patch("file.txt", &hunks, &first_group);
        git_stage_hunk(dir.path().to_str().unwrap(), &first_patch)
            .await
            .unwrap();
        assert_eq!(
            index_file_content(&repo, "file.txt"),
            "line1\nchanged2\nline3\nline4\n"
        );

        let staged = index_file_content(&repo, "file.txt");
        let (hunks_after_stage, groups_after_stage) = diff_hunks_and_groups(&staged, modified);
        let second_group = groups_after_stage
            .iter()
            .find(|group| group.group_id == second_group_id)
            .unwrap();
        let second_patch = group_patch("file.txt", &hunks_after_stage, second_group);
        git_stage_hunk(dir.path().to_str().unwrap(), &second_patch)
            .await
            .unwrap();

        assert_eq!(index_file_content(&repo, "file.txt"), modified);
    }

    #[tokio::test]
    pub async fn test_unstage_hunk() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        fs::write(dir.path().join("file.txt"), "line1\nmodified\nline3\n").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert!(before.iter().any(|s| s.index_status == "modified"));

        let patch =
            "--- a/file.txt\n+++ b/file.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+modified\n line3\n";
        git_unstage_hunk(dir.path().to_str().unwrap(), patch)
            .await
            .unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        let file_status = after.iter().find(|s| s.path == "file.txt").unwrap();
        assert_eq!(file_status.worktree_status, "modified");
        assert_eq!(file_status.index_status, "none");
    }
}
