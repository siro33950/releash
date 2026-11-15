use releashd::test_support::client_api_acceptance::{GitGateway, GitService};
use releashd::test_support::integration::settings::{AppConfig, ReleashConfig};
use std::sync::Arc;

fn gateway() -> GitGateway {
    GitGateway::new(Arc::new(AppConfig::new(
        ReleashConfig::default(),
        Default::default(),
    )))
}

#[tokio::test(flavor = "multi_thread")]
async fn test_破棄_削除したディレクトリ内の追跡ファイルを復元する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    std::fs::create_dir(root.path().join("dir")).unwrap();
    std::fs::write(root.path().join("dir/a"), "staged\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("dir/a")).unwrap();
    index.write().unwrap();
    std::fs::remove_dir_all(root.path().join("dir")).unwrap();
    let path = root.path().to_str().unwrap().to_owned();
    // When
    tokio::task::spawn_blocking(move || gateway().restore_worktree_files(&path, &["dir/a".into()]))
        .await
        .unwrap()
        .unwrap();
    // Then
    assert_eq!(
        std::fs::read_to_string(root.path().join("dir/a")).unwrap(),
        "staged\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_破棄_worktree外のパスとsymlink経由を拒否する() {
    // Given
    let root = tempfile::tempdir().unwrap();
    git2::Repository::init(root.path()).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("keep"), "keep").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
    let path = root.path().to_str().unwrap().to_owned();
    // When / Then
    tokio::task::spawn_blocking(move || {
        for file in ["../keep", "/keep", "link/keep"] {
            assert!(gateway()
                .restore_worktree_files(&path, &[file.into()])
                .is_err());
            assert!(gateway().trash_files(&path, &[file.into()]).is_err());
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(outside.path().join("keep")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn test_commit_無出力のhook失敗でも空でない理由を返す() {
    use releashd::test_support::client_api_acceptance::CommitMessage;
    use std::os::unix::fs::PermissionsExt;
    // Given
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "test").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();
    config.set_bool("commit.gpgSign", false).unwrap();
    let hooks = repo.path().join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    config
        .set_str("core.hooksPath", hooks.to_str().unwrap())
        .unwrap();
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(root.path().join("a"), "a").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("a")).unwrap();
    index.write().unwrap();
    let path = root.path().to_str().unwrap().to_owned();
    // When
    let error = tokio::task::spawn_blocking(move || {
        gateway().commit(&path, &CommitMessage::new("commit").unwrap())
    })
    .await
    .unwrap()
    .unwrap_err();
    // Then
    assert!(!error.to_string().is_empty());
    assert!(repo.head().is_err());
}

fn git(path: &std::path::Path, args: &[&str], input: &str) {
    use std::io::Write;
    let mut command = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    command
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let result = command.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn test_conflict分類_indexの全7種類を返す() {
    // Given
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    let blob = repo.blob(b"content\n").unwrap();
    let cases = [
        ([true, true, true], "both_modified"),
        ([false, true, true], "both_added"),
        ([true, false, false], "both_deleted"),
        ([false, true, false], "added_by_us"),
        ([false, false, true], "added_by_them"),
        ([true, false, true], "deleted_by_us"),
        ([true, true, false], "deleted_by_them"),
    ];
    let mut input = String::new();
    for (stages, name) in cases {
        for (stage, present) in stages.into_iter().enumerate() {
            if present {
                input += &format!("100644 {blob} {}\t{name}\n", stage + 1);
            }
        }
    }
    git(root.path(), &["update-index", "--index-info"], &input);
    // When
    let scan = gateway()
        .status_scan(root.path().to_str().unwrap())
        .unwrap();
    // Then
    assert_eq!(scan.status.len(), cases.len());
    for (_, name) in cases {
        let file = scan.status.iter().find(|file| file.path == name).unwrap();
        assert_eq!(file.conflict.unwrap().as_str(), name);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ブランチ状態_unbornとdetachedはcurrent表示で未公開を返す() {
    use releashd::test_support::client_api_acceptance::BranchState;
    // Given
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    repo.set_head("refs/heads/main").unwrap();
    let path = root.path().to_str().unwrap().to_owned();
    tokio::task::spawn_blocking(move || {
        // When / Then
        let branch = gateway().worktree_branch(&path).unwrap();
        assert_eq!(branch.name, gateway().current(&path).unwrap());
        assert_eq!(branch.state, BranchState::Unpublished { unpushed: 0 });
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Test", "test@example.invalid").unwrap();
        let commit = repo
            .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        repo.set_head_detached(commit).unwrap();
        let branch = gateway().worktree_branch(&path).unwrap();
        assert_eq!(branch.name, gateway().current(&path).unwrap());
        assert_eq!(branch.state, BranchState::Unpublished { unpushed: 1 });
    })
    .await
    .unwrap();
}

#[test]
fn test_ブランチ状態_merge中はmerge_msgを返す() {
    use releashd::test_support::client_api_acceptance::{BranchState, OperationKind};
    // Given
    let root = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(root.path()).unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("Test", "test@example.invalid").unwrap();
    let commit = repo
        .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .unwrap();
    std::fs::write(repo.path().join("MERGE_HEAD"), format!("{commit}\n")).unwrap();
    std::fs::write(
        repo.path().join("MERGE_MSG"),
        "Merge feature\n\nMerge details\n",
    )
    .unwrap();
    // When
    let branch = gateway()
        .worktree_branch(root.path().to_str().unwrap())
        .unwrap();
    // Then
    let BranchState::Operating(operation) = branch.state else {
        panic!("expected merge");
    };
    assert_eq!(operation.kind, OperationKind::Merge);
    assert_eq!(
        operation.message.as_deref(),
        Some("Merge feature\n\nMerge details")
    );
}

#[test]
fn test_git操作_取消と期限切れをリポジトリへのアクセス前に返す() {
    use releashd::test_support::client_api_acceptance::{
        CommitMessage, OperationKind, Push, RebaseProgress,
    };
    use releashd::test_support::integration::platform::{sync_scope, Deadline, OperationContext};
    // Given
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("missing");
    let path = path.to_str().unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    let contexts = [
        (
            OperationContext::new(None, Arc::new(token)),
            "Operation cancelled",
        ),
        (
            OperationContext::new(
                Some(Deadline::new(std::time::Instant::now())),
                Arc::new(tokio_util::sync::CancellationToken::new()),
            ),
            "Operation deadline exceeded",
        ),
    ];
    for (context, expected) in contexts {
        // When
        let results = sync_scope(context, || {
            let gateway = gateway();
            [
                gateway.worktree_branch(path).map(|_| ()),
                gateway.line_changes(path).map(|_| ()),
                gateway.push(path, Push::ToUpstream),
                gateway.fetch(path),
                gateway.continue_operation(
                    path,
                    &OperationKind::Rebase(RebaseProgress {
                        onto: "main".into(),
                        step: 1,
                        total: 2,
                    }),
                    Some(&CommitMessage::new("message").unwrap()),
                ),
            ]
        });
        // Then
        for result in results {
            assert_eq!(result.unwrap_err().to_string(), expected);
        }
        assert!(!std::path::Path::new(path).exists());
    }
}

#[cfg(unix)]
#[test]
fn test_変更行数_通常ファイルをsymlinkに置換した追加削除を両方返す() {
    for staged in [false, true] {
        // Given
        let root = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(root.path()).unwrap();
        std::fs::write(root.path().join("a"), "first\nsecond\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let signature = git2::Signature::now("Test", "test@example.invalid").unwrap();
        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        std::fs::remove_file(root.path().join("a")).unwrap();
        std::os::unix::fs::symlink("target", root.path().join("a")).unwrap();
        if staged {
            index.add_path(std::path::Path::new("a")).unwrap();
            index.write().unwrap();
        }
        // When
        let changes = gateway()
            .line_changes(root.path().to_str().unwrap())
            .unwrap();
        // Then
        let lines = if staged {
            &changes.staged["a"]
        } else {
            &changes.unstaged["a"]
        };
        assert_eq!(lines.additions, 1);
        assert_eq!(lines.deletions, 2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rebase停止_breakとexec失敗はメッセージ無しで続行する() {
    use releashd::test_support::client_api_acceptance::BranchState;
    for stop in ["break", "exec false"] {
        let stop = stop.to_owned();
        tokio::task::spawn_blocking(move || {
            // Given
            let root = tempfile::tempdir().unwrap();
            git(root.path(), &["init", "-b", "main"], "");
            git(root.path(), &["config", "user.name", "Test"], "");
            git(
                root.path(),
                &["config", "user.email", "test@example.invalid"],
                "",
            );
            git(root.path(), &["config", "commit.gpgSign", "false"], "");
            for message in ["first", "second", "third"] {
                git(root.path(), &["commit", "--allow-empty", "-m", message], "");
            }
            let result = std::process::Command::new("git")
                .arg("-C")
                .arg(root.path())
                .args(["rebase", "-i", "HEAD~2"])
                .env(
                    "GIT_SEQUENCE_EDITOR",
                    format!("sed -i.bak '1a\\\n{stop}\\\n'"),
                )
                .output()
                .unwrap();
            if stop == "break" {
                assert!(result.status.success(), "{result:?}");
            } else {
                assert!(!result.status.success());
            }
            let repo = git2::Repository::open(root.path()).unwrap();
            assert!(repo.path().join("rebase-merge").is_dir());
            let path = root.path().to_str().unwrap();
            // When
            let branch = gateway().worktree_branch(path).unwrap();
            let BranchState::Operating(operation) = branch.state else {
                panic!("expected rebase")
            };
            // Then
            assert_eq!(operation.message, None);
            gateway()
                .continue_operation(path, &operation.kind, None)
                .unwrap();
            assert!(!repo.path().join("rebase-merge").exists());
            assert_eq!(
                repo.head()
                    .unwrap()
                    .peel_to_commit()
                    .unwrap()
                    .message()
                    .unwrap(),
                "third\n"
            );
        })
        .await
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_remote操作_origin以外のupstreamへpushとfetchする() {
    use releashd::test_support::client_api_acceptance::Push;
    tokio::task::spawn_blocking(|| {
        // Given
        let root = tempfile::tempdir().unwrap();
        let origin = tempfile::tempdir().unwrap();
        let upstream = tempfile::tempdir().unwrap();
        git(origin.path(), &["init", "--bare"], "");
        git(upstream.path(), &["init", "--bare"], "");
        git(root.path(), &["init", "-b", "main"], "");
        git(root.path(), &["config", "user.name", "Test"], "");
        git(
            root.path(),
            &["config", "user.email", "test@example.invalid"],
            "",
        );
        git(root.path(), &["config", "commit.gpgSign", "false"], "");
        git(
            root.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
            "",
        );
        git(
            root.path(),
            &["remote", "add", "other", upstream.path().to_str().unwrap()],
            "",
        );
        git(root.path(), &["commit", "--allow-empty", "-m", "first"], "");
        git(root.path(), &["push", "-u", "other", "HEAD:target"], "");
        let path = root.path().to_str().unwrap();
        let remote = git2::Repository::open_bare(upstream.path()).unwrap();
        let repo = git2::Repository::open(root.path()).unwrap();
        for push in [Push::ToUpstream, Push::ForceWithLease] {
            git(root.path(), &["commit", "--allow-empty", "-m", "next"], "");
            // When
            gateway().push(path, push).unwrap();
            // Then
            assert_eq!(
                remote.refname_to_id("refs/heads/target").unwrap(),
                repo.head().unwrap().target().unwrap()
            );
            assert!(git2::Repository::open_bare(origin.path())
                .unwrap()
                .find_reference("refs/heads/target")
                .is_err());
        }
        git(
            root.path(),
            &["update-ref", "refs/remotes/other/target", "HEAD~1"],
            "",
        );
        // When
        gateway().fetch(path).unwrap();
        // Then
        assert_eq!(
            repo.refname_to_id("refs/remotes/other/target").unwrap(),
            remote.refname_to_id("refs/heads/target").unwrap()
        );
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fetch_upstreamが無い場合はremote設定があってもoriginから取得する() {
    tokio::task::spawn_blocking(|| {
        for state in ["missing_merge", "missing_ref", "unborn", "detached"] {
            // Given
            let root = tempfile::tempdir().unwrap();
            let origin = tempfile::tempdir().unwrap();
            let other = tempfile::tempdir().unwrap();
            git(origin.path(), &["init", "--bare"], "");
            git(other.path(), &["init", "--bare"], "");
            git(root.path(), &["init", "-b", "main"], "");
            git(root.path(), &["config", "user.name", "Test"], "");
            git(
                root.path(),
                &["config", "user.email", "test@example.invalid"],
                "",
            );
            git(root.path(), &["config", "commit.gpgSign", "false"], "");
            git(
                root.path(),
                &["remote", "add", "origin", origin.path().to_str().unwrap()],
                "",
            );
            git(
                root.path(),
                &["remote", "add", "other", other.path().to_str().unwrap()],
                "",
            );
            git(
                root.path(),
                &["commit", "--allow-empty", "-m", "initial"],
                "",
            );
            git(root.path(), &["push", "origin", "HEAD:target"], "");
            git(
                root.path(),
                &["update-ref", "-d", "refs/remotes/origin/target"],
                "",
            );
            git(root.path(), &["config", "branch.main.remote", "other"], "");
            match state {
                "missing_ref" => git(
                    root.path(),
                    &["config", "branch.main.merge", "refs/heads/missing"],
                    "",
                ),
                "unborn" => git(root.path(), &["update-ref", "-d", "refs/heads/main"], ""),
                "detached" => git(root.path(), &["checkout", "--detach"], ""),
                _ => {}
            }
            // When
            gateway().fetch(root.path().to_str().unwrap()).unwrap();
            // Then
            let repo = git2::Repository::open(root.path()).unwrap();
            let remote = git2::Repository::open_bare(origin.path()).unwrap();
            assert_eq!(
                repo.refname_to_id("refs/remotes/origin/target").unwrap(),
                remote.refname_to_id("refs/heads/target").unwrap(),
                "{state}"
            );
        }
    })
    .await
    .unwrap();
}
