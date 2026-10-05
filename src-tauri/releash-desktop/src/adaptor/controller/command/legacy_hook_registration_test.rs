#[test]
fn test_legacy_hook登録_廃止済み設定commandを登録しない() {
    let registered = super::tests::registered_command_names();

    for removed in [
        "generate_hooks_config",
        "get_hooks_status",
        "apply_hooks_config",
    ] {
        assert!(
            !registered.contains(&removed),
            "still registered: {removed}"
        );
    }
}
