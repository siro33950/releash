use super::*;
use std::collections::BTreeSet;

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    git2::Repository::init(directory.path()).unwrap();
    directory
}

fn write(repo: &Path, path: &str) {
    let path = repo.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "acceptance fixture\n").unwrap();
}

fn commit(repo: &Path) {
    let git = git2::Repository::open(repo).unwrap();
    let mut index = git.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = git.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("acceptance", "test@example.invalid").unwrap();
    git.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .unwrap();
}

async fn filtered(
    socket: &Socket,
    repo: &Path,
    query: Option<&str>,
    changed: bool,
) -> wire::FilteredFilesSnapshot {
    let mut args = vec![repo.to_str().unwrap().into()];
    if let Some(query) = query {
        args.push(format!("name={query}"));
    }
    if changed {
        args.push("changed=true".into());
    }
    let mut stream = subscribe_state(socket, "filtered-files", args).await;
    let value = tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .expect("filtered-files did not deliver a snapshot")
        .unwrap();
    let wire::state_payload::Value::FilteredFiles(snapshot) = value else {
        panic!("expected filtered-files, got {value:?}");
    };
    snapshot
}

fn paths(snapshot: &wire::FilteredFilesSnapshot) -> BTreeSet<&str> {
    snapshot
        .directories
        .iter()
        .flat_map(|directory| &directory.entries)
        .map(|entry| entry.path.as_str())
        .collect()
}

fn assert_tree(snapshot: &wire::FilteredFilesSnapshot, expected: &[(&str, &[&str])]) {
    let actual = snapshot
        .directories
        .iter()
        .map(|directory| {
            (
                directory.path.as_str(),
                directory
                    .entries
                    .iter()
                    .map(|entry| entry.path.as_str())
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let expected = expected
        .iter()
        .map(|(path, entries)| (*path, entries.iter().copied().collect::<BTreeSet<_>>()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn test_ファイル検索_b018_b019_未展開の階層から名前を大小文字と文字間を問わず絞る() {
    // Given
    let repo = repository();
    write(repo.path(), "closed/nested/AlphaBeta.rs");
    write(repo.path(), "closed/nested/Other.rs");
    commit(repo.path());
    let data = tempfile::tempdir().unwrap();
    let (_daemon, discovery) = start(data.path());
    let (socket, _) = connect(&discovery).await;
    // When
    let snapshot = filtered(&socket, repo.path(), Some("ab"), false).await;
    // Then
    assert_tree(
        &snapshot,
        &[
            (".", &["closed"]),
            ("closed", &["closed/nested"]),
            ("closed/nested", &["closed/nested/AlphaBeta.rs"]),
        ],
    );
}

#[tokio::test]
async fn test_ファイル検索_b020_一致したディレクトリだけを残して不一致の中身を除く() {
    // Given
    let repo = repository();
    write(repo.path(), "matches/other.rs");
    commit(repo.path());
    let data = tempfile::tempdir().unwrap();
    let (_daemon, discovery) = start(data.path());
    let (socket, _) = connect(&discovery).await;
    // When
    let snapshot = filtered(&socket, repo.path(), Some("matches"), false).await;
    // Then
    assert_tree(&snapshot, &[(".", &["matches"])]);
    assert!(snapshot.directories[0].entries[0].directory);
}

#[tokio::test]
async fn test_ファイル検索_b021_gitignoreの対象を含めgitディレクトリを除く() {
    // Given
    let repo = repository();
    std::fs::write(repo.path().join(".gitignore"), "ignored/\n").unwrap();
    write(repo.path(), "ignored/needle.txt");
    write(repo.path(), ".git/needle.txt");
    commit(repo.path());
    let data = tempfile::tempdir().unwrap();
    let (_daemon, discovery) = start(data.path());
    let (socket, _) = connect(&discovery).await;
    // When
    let snapshot = filtered(&socket, repo.path(), Some("needle"), false).await;
    // Then
    assert_tree(
        &snapshot,
        &[(".", &["ignored"]), ("ignored", &["ignored/needle.txt"])],
    );
}

#[tokio::test]
async fn test_ファイル検索_b030_ディレクトリ名とファイル名をつなげて一致させない() {
    // Given
    let repo = repository();
    write(repo.path(), "alpha/beta.rs");
    commit(repo.path());
    let data = tempfile::tempdir().unwrap();
    let (_daemon, discovery) = start(data.path());
    let (socket, _) = connect(&discovery).await;
    // When
    let snapshot = filtered(&socket, repo.path(), Some("alphabeta"), false).await;
    // Then
    assert!(snapshot.directories.is_empty());
}

#[tokio::test]
async fn test_差分のみ_b023_b024_b026_未展開の変更を残し削除を除いて検索と併用する() {
    // Given
    let repo = repository();
    for path in [
        "closed/match-changed.rs",
        "closed/match-clean.rs",
        "closed/other.rs",
        "closed/match-deleted.rs",
    ] {
        write(repo.path(), path);
    }
    commit(repo.path());
    std::fs::write(repo.path().join("closed/match-changed.rs"), "changed\n").unwrap();
    std::fs::write(repo.path().join("closed/other.rs"), "changed\n").unwrap();
    std::fs::remove_file(repo.path().join("closed/match-deleted.rs")).unwrap();
    let data = tempfile::tempdir().unwrap();
    let (_daemon, discovery) = start(data.path());
    let (socket, _) = connect(&discovery).await;
    // When
    let changed = filtered(&socket, repo.path(), None, true).await;
    // Then
    assert_tree(
        &changed,
        &[
            (".", &["closed"]),
            ("closed", &["closed/match-changed.rs", "closed/other.rs"]),
        ],
    );
    // When
    let combined = filtered(&socket, repo.path(), Some("match"), true).await;
    // Then
    assert_tree(
        &combined,
        &[(".", &["closed"]), ("closed", &["closed/match-changed.rs"])],
    );
}

#[tokio::test]
async fn test_ファイル検索_b032_b033_512件以下は全件で超過時は512件と一部の通知を返す() {
    for count in [3, 512, 513] {
        // Given
        let repo = repository();
        for index in 0..count {
            write(repo.path(), &format!("closed/needle-{index}.rs"));
        }
        commit(repo.path());
        let data = tempfile::tempdir().unwrap();
        let (_daemon, discovery) = start(data.path());
        let (socket, _) = connect(&discovery).await;
        // When
        let snapshot = filtered(&socket, repo.path(), Some("needle"), false).await;
        // Then
        let files = snapshot
            .directories
            .iter()
            .flat_map(|directory| &directory.entries)
            .filter(|entry| !entry.directory)
            .map(|entry| entry.path.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(files.len(), count.min(512));
        assert!(files.iter().all(|path| repo.path().join(path).is_file()));
        assert_eq!(snapshot.truncated, count > 512);
        assert!(paths(&snapshot).contains("closed"));
        if count <= 512 {
            assert_eq!(
                files,
                (0..count)
                    .map(|index| format!("closed/needle-{index}.rs"))
                    .collect::<Vec<_>>()
                    .iter()
                    .map(String::as_str)
                    .collect()
            );
        }
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_外部エディタ_b027_b035_選択したアプリへworktreeとファイルを一度に渡す() {
    // Given: a real macOS application records the open-document event it receives.
    let repo = repository();
    write(repo.path(), "file.rs");
    commit(repo.path());
    let fixture = tempfile::tempdir().unwrap();
    let record = fixture.path().join("received.txt");
    let app = fixture.path().join("Acceptance Editor.app");
    let script = fixture.path().join("editor.applescript");
    let escaped_record = record
        .to_str()
        .unwrap()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    std::fs::write(
        &script,
        format!(
            r#"
on open receivedItems
    set output to open for access POSIX file "{escaped_record}" with write permission
    try
        write "BEGIN" & linefeed to output starting at eof
        repeat with receivedItem in receivedItems
            write (POSIX path of receivedItem) & linefeed to output starting at eof
        end repeat
        write "END" & linefeed to output starting at eof
        close access output
    on error messageText
        close access output
        error messageText
    end try
end open
"#
        ),
    )
    .unwrap();
    let compiled = Command::new("/usr/bin/osacompile")
        .arg("-o")
        .arg(&app)
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let data = tempfile::tempdir().unwrap();
    let (_daemon, discovery) = start(data.path());
    let (mut socket, _) = connect(&discovery).await;
    request(
        &mut socket,
        "select-editor",
        wire::command_request::Command::UpdateExternalEditor(wire::UpdateExternalEditorRequest {
            editor: Some(app.to_str().unwrap().into()),
        }),
    )
    .await;
    assert_eq!(read_external_editor(&socket).await, app.to_str().unwrap());
    for file in [None, Some("file.rs")] {
        std::fs::write(&record, "").unwrap();
        // When
        request(
            &mut socket,
            "open-editor",
            wire::command_request::Command::OpenInEditor(wire::OpenInEditorRequest {
                worktree_path: Some(repo.path().to_str().unwrap().into()),
                file: file.map(str::to_string),
            }),
        )
        .await;
        // Then
        let deadline = Instant::now() + Duration::from_secs(15);
        let received = loop {
            let contents = std::fs::read_to_string(&record).unwrap();
            if contents.ends_with("END\n") {
                break contents;
            }
            assert!(
                Instant::now() < deadline,
                "selected editor did not receive the open event: {contents}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(received.lines().filter(|line| *line == "BEGIN").count(), 1);
        assert_eq!(received.lines().filter(|line| *line == "END").count(), 1);
        let actual = received
            .lines()
            .filter(|line| !["BEGIN", "END"].contains(line))
            .map(|path| Path::new(path).canonicalize().unwrap())
            .collect::<BTreeSet<_>>();
        let mut expected = BTreeSet::from([repo.path().canonicalize().unwrap()]);
        if let Some(file) = file {
            expected.insert(repo.path().join(file).canonicalize().unwrap());
        }
        assert_eq!(actual, expected);
    }
}
