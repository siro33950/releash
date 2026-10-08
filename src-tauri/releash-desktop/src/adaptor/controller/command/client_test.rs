use crate::adaptor::controller::command as commands;
use releashd::desktop_api::test_support as wire;
#[test]
fn test_クライアントdispatch_proto全commandの登録と引数検証() {
    for command in commands::tests::registered_command_names() {
        if command != "set_menu_items_enabled"
            && !commands::client::COMMAND_NAMES.contains(&command)
            && !commands::desktop_lifecycle::COMMAND_NAMES.contains(&command)
        {
            assert!(wire::COMMAND_NAMES.contains(&command), "{command}");
        }
    }
    for command in [
        "menu",
        "set_menu_items_enabled",
        "start_watching",
        "start_git_dir_watching",
        "stop_watching",
    ]
    .into_iter()
    .chain(commands::desktop_lifecycle::COMMAND_NAMES.iter().copied())
    {
        assert!(!wire::COMMAND_NAMES.contains(&command), "{command}");
    }
}
