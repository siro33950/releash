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
        assert!(
            class("ReleashModule").contains(&format!(
                "---@field {} fun(options: Releash{builder}Options): ReleashNode\n",
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
