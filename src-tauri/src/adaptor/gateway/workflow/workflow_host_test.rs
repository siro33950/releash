use super::*;

#[test]
fn command_env_includes_worktree_path() {
    let input = CommandExecutionInput {
        execution_id: "execution-1".to_string(),
        node_execution_id: "node-execution-1".to_string(),
        node_name: "check".to_string(),
        attempt: 1,
        worktree_path: "/repo/worktree".to_string(),
        raw_command: Some("true".to_string()),
        definition_env: Vec::new(),
        contract: None,
        schemas: BTreeMap::new(),
        session_id: None,
    };

    let env = command_env(
        &input,
        vec![
            ("DOC".to_string(), "document".to_string()),
            (
                "RELEASH_WORKTREE_PATH".to_string(),
                "/definition/attempted-override".to_string(),
            ),
        ],
    );

    assert!(env.contains(&("DOC".to_string(), "document".to_string())));
    assert!(env.contains(&(
        "RELEASH_WORKTREE_PATH".to_string(),
        "/repo/worktree".to_string()
    )));
    assert_eq!(
        env.iter()
            .rev()
            .find(|(name, _)| name == "RELEASH_WORKTREE_PATH")
            .map(|(_, value)| value.as_str()),
        Some("/repo/worktree")
    );
}
