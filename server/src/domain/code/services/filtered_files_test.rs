use super::*;
use crate::domain::code::value_objects::file_list::{FileNode, NameQuery};

fn entry(path: &str, directory: bool) -> WorktreeEntry {
    WorktreeEntry {
        path: path.into(),
        name: path.rsplit('/').next().unwrap().into(),
        directory,
        ignored: false,
    }
}

fn node(path: &str, directory: bool, status: GitStatus, children: Vec<FileNode>) -> FileNode {
    FileNode {
        entry: entry(path, directory),
        status,
        children,
    }
}

fn name_filter() -> FileFilter {
    FileFilter::Name(NameQuery::new("hit").unwrap())
}

#[test]
fn test_絞込一覧_一致項目と祖先だけを階層順と名前順で残す() {
    // Given
    let statuses = [
        ("z/deep/hit-b", GitStatus::New, GitStatus::Modified),
        ("z/other", GitStatus::None, GitStatus::Conflicted),
        ("hit-dir/hit-child", GitStatus::New, GitStatus::Modified),
        ("hit-dir/unrelated", GitStatus::None, GitStatus::Conflicted),
    ];
    let mut collector = FilteredFilesCollector::new(name_filter(), statuses.into_iter());
    // When
    for (path, directory) in [
        ("z", true),
        ("z/deep", true),
        ("z/deep/hit-b", false),
        ("z/deep/hit-a", false),
        ("z/other", false),
        ("hit-dir", true),
        ("hit-dir/hit-child", false),
        ("hit-dir/unrelated", false),
        ("hit-empty", true),
        ("hit-root", false),
        ("absent", true),
    ] {
        assert_eq!(collector.offer(entry(path, directory)), WalkStep::Continue);
    }
    let result = collector.finish();
    // Then
    assert_eq!(
        result,
        FilteredFiles {
            roots: vec![
                node(
                    "hit-dir",
                    true,
                    GitStatus::Conflicted,
                    vec![node(
                        "hit-dir/hit-child",
                        false,
                        GitStatus::Modified,
                        vec![]
                    )],
                ),
                node("hit-empty", true, GitStatus::None, vec![]),
                node(
                    "z",
                    true,
                    GitStatus::Conflicted,
                    vec![node(
                        "z/deep",
                        true,
                        GitStatus::Modified,
                        vec![
                            node("z/deep/hit-a", false, GitStatus::None, vec![]),
                            node("z/deep/hit-b", false, GitStatus::Modified, vec![]),
                        ]
                    )]
                ),
                node("hit-root", false, GitStatus::None, vec![]),
            ],
            truncated: false
        }
    );
}

#[test]
fn test_絞込一覧_名前は各項目単独で照合し除外属性も保持する() {
    // Given
    let mut collector = FilteredFilesCollector::new(name_filter(), std::iter::empty());
    let mut ignored = entry("hit.txt", false);
    ignored.ignored = true;
    // When
    for item in [entry("hi", true), entry("hi/t", false), ignored.clone()] {
        assert_eq!(collector.offer(item), WalkStep::Continue);
    }
    // Then
    assert_eq!(
        collector.finish().roots,
        vec![FileNode {
            entry: ignored,
            status: GitStatus::None,
            children: vec![]
        }]
    );
}

#[test]
fn test_絞込一覧_差分条件は状態のないディレクトリを降りずファイルだけ残す() {
    // Given / When / Then
    for filter in [
        FileFilter::Changed,
        FileFilter::NameAndChanged(NameQuery::new("hit").unwrap()),
    ] {
        let statuses = [
            ("dir/hit", GitStatus::New, GitStatus::None),
            ("dir/other", GitStatus::None, GitStatus::Modified),
            ("hit-deleted/gone", GitStatus::None, GitStatus::Deleted),
            ("hit-empty", GitStatus::None, GitStatus::None),
            ("directory/hit", GitStatus::New, GitStatus::None),
        ];
        let combined = matches!(filter, FileFilter::NameAndChanged(_));
        let mut collector = FilteredFilesCollector::new(filter, statuses.into_iter());
        assert_eq!(
            collector.offer(entry("hit-empty", true)),
            WalkStep::SkipChildren
        );
        assert_eq!(
            collector.offer(entry("direct", true)),
            WalkStep::SkipChildren
        );
        for (path, directory) in [
            ("dir", true),
            ("dir/hit", false),
            ("dir/other", false),
            ("dir/hit-clean", false),
            ("hit-deleted", true),
        ] {
            assert_eq!(collector.offer(entry(path, directory)), WalkStep::Continue);
        }
        let mut children = vec![node("dir/hit", false, GitStatus::New, vec![])];
        if !combined {
            children.push(node("dir/other", false, GitStatus::Modified, vec![]));
        }
        assert_eq!(
            collector.finish(),
            FilteredFiles {
                roots: vec![node("dir", true, GitStatus::Modified, children)],
                truncated: false
            }
        );
    }
}

#[test]
fn test_絞込一覧_git状態はindexと作業ツリーを集約する() {
    // Given
    let statuses = [
        ("hit-a", GitStatus::Renamed, GitStatus::None),
        ("hit-b", GitStatus::Modified, GitStatus::Conflicted),
        ("hit-c", GitStatus::Deleted, GitStatus::New),
    ];
    let mut collector = FilteredFilesCollector::new(FileFilter::Changed, statuses.into_iter());
    // When
    for path in ["hit-c", "hit-b", "hit-a"] {
        assert_eq!(collector.offer(entry(path, false)), WalkStep::Continue);
    }
    // Then
    assert_eq!(
        collector.finish().roots,
        vec![
            node("hit-a", false, GitStatus::Modified, vec![]),
            node("hit-b", false, GitStatus::Conflicted, vec![]),
            node("hit-c", false, GitStatus::Deleted, vec![])
        ]
    );
}

#[test]
fn test_絞込一覧_検索は512件まで残し513件目で停止する() {
    // Given / When / Then
    for changed in [false, true] {
        for count in [0, 511, 512, 513] {
            let paths: Vec<_> = (0..count).map(|i| format!("hit-{i:04}")).collect();
            let filter = FileFilter::new(Some(NameQuery::new("hit").unwrap()), changed).unwrap();
            let mut collector = FilteredFilesCollector::new(
                filter,
                paths
                    .iter()
                    .map(|p| (p.as_str(), GitStatus::New, GitStatus::None)),
            );
            for (i, path) in paths.iter().enumerate() {
                assert_eq!(
                    collector.offer(entry(path, false)),
                    if i == 512 {
                        WalkStep::Stop
                    } else {
                        WalkStep::Continue
                    }
                );
            }
            let result = collector.finish();
            assert_eq!(result.truncated, count > 512);
            assert_eq!(
                result
                    .roots
                    .iter()
                    .map(|n| n.entry.path.as_str())
                    .collect::<Vec<_>>(),
                paths
                    .iter()
                    .take(512)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn test_絞込一覧_上限後も不一致ファイルとディレクトリでは打ち切らない() {
    // Given
    let mut collector = FilteredFilesCollector::new(name_filter(), std::iter::empty());
    for i in 0..512 {
        assert_eq!(
            collector.offer(entry(&format!("hit-{i}"), false)),
            WalkStep::Continue
        );
    }
    // When
    assert_eq!(
        collector.offer(entry("unrelated", false)),
        WalkStep::Continue
    );
    assert_eq!(collector.offer(entry("hit-dir", true)), WalkStep::Continue);
    let result = collector.finish();
    // Then
    assert!(!result.truncated);
    assert_eq!(result.roots.len(), 513);
    assert_eq!(
        result.roots[0],
        node("hit-dir", true, GitStatus::None, vec![])
    );
}

#[test]
fn test_絞込一覧_差分のみでは512件を超えても続ける() {
    // Given
    let paths: Vec<_> = (0..513).map(|i| format!("file-{i:04}")).collect();
    let mut collector = FilteredFilesCollector::new(
        FileFilter::Changed,
        paths
            .iter()
            .map(|p| (p.as_str(), GitStatus::None, GitStatus::Modified)),
    );
    // When
    for path in &paths {
        assert_eq!(collector.offer(entry(path, false)), WalkStep::Continue);
    }
    let result = collector.finish();
    // Then
    assert_eq!(result.roots.len(), 513);
    assert!(!result.truncated);
}
