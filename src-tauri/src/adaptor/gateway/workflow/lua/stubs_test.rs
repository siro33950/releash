use super::*;

#[test]
fn test_worktreeのstub_生成物が型とhandleと全builderの公開契約を持つ() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();
    // Then
    assert!(stub.lines().any(|line| line == "---@class ReleashWorktree"));
    let class = |name: &str| {
        stub.split(&format!("---@class {name}\n"))
            .nth(1)
            .unwrap()
            .split("---@class ")
            .next()
            .unwrap()
    };
    for handle in ["shared", "isolated"] {
        assert!(class("ReleashWorktreeModule")
            .contains(&format!("---@field {handle} ReleashWorktree\n")));
    }
    assert!(class("ReleashModule").contains("---@field worktree ReleashWorktreeModule\n"));
    for builder in ["Command", "Session", "Fanout", "Sequence"] {
        assert!(
            class(&format!("Releash{builder}Options"))
                .contains("---@field worktree? ReleashWorktree\n"),
            "{builder}"
        );
        let result = if builder == "Session" {
            "ReleashSession"
        } else {
            "ReleashNode"
        };
        assert!(
            class("ReleashModule").contains(&format!(
                "---@field {} fun(options: Releash{builder}Options): {result}\n",
                builder.to_lowercase()
            )),
            "{builder}"
        );
    }
}

#[test]
fn test_lua述語stub_生成物の型とbuilderが公開apiと一致する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();
    // Then
    assert!(stub.contains("---@class ReleashPredicate"));
    assert!(
        stub.contains("---@class ReleashWhenOptions\n---@field on ReleashSource|ReleashPredicate")
    );
    for builder in ["all", "any"] {
        assert!(stub.contains(&format!("---@field {builder} fun(elements: (ReleashSource|ReleashPredicate)[]): ReleashPredicate")));
    }
    assert!(stub.contains("---@class ReleashSwitchOptions\n---@field on ReleashSource\n"));
}

#[test]
fn test_completionのstub_要求tableと承認handleの型を区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();
    // Then
    assert!(stub
        .contains("---@class ReleashCompletion\n---@field require ReleashCompletionRequirement\n"));
    assert!(stub.contains("---@field approval ReleashCompletionRequirement\n"));
    assert_eq!(
        stub.matches("---@field completion? ReleashCompletion\n")
            .count(),
        4
    );
}

#[test]
fn test_completionのstub_delegateメソッドとchild段を再生成する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(".releash/releash.lua");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "old stub").unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(path).unwrap();
    // Then
    assert!(stub
        .contains("---@field delegate fun(options: ReleashDelegateOptions) Session handles only;"));
    assert!(stub.contains("---@field child ReleashSource Only on a Session with delegate."));
    assert!(stub.contains("---@class ReleashDelegateOptions\n---@field child ReleashNode\n"));
    assert!(stub.contains("---@field inputs? table<string, ReleashSource> Parent Input"));
    assert!(stub.contains("---@field when ReleashSource|ReleashPredicate Parent Artifact"));
    assert!(stub.contains("---@field max_iterations integer At least 1."));
    let class = |name: &str| {
        stub.split(&format!("---@class {name}\n"))
            .nth(1)
            .unwrap()
            .split("---@class ")
            .next()
            .unwrap()
    };
    let node = class("ReleashNode: ReleashSource");
    assert!(!node.contains("---@field delegate "));
    assert!(!node.contains("---@field child "));
    let session = class("ReleashSession: ReleashNode");
    assert!(session.contains("---@field delegate fun(options: ReleashDelegateOptions)"));
    assert!(session.contains("---@field child ReleashSource"));
    assert!(class("ReleashModule")
        .contains("---@field session fun(options: ReleashSessionOptions): ReleashSession\n"));
    for builder in ["Command", "Sequence", "Fanout"] {
        assert!(class("ReleashModule").contains(&format!(
            "---@field {} fun(options: Releash{builder}Options): ReleashNode\n",
            builder.to_lowercase()
        )));
    }
}
