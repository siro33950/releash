use releash_desktop::test_support::integration::desktop::*;

#[test]
fn b071_pre_admission_window_grants_no_plugin_ipc_capability() {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    assert_eq!(config["app"]["windows"][0]["label"], NORMAL_WINDOW_LABEL);
    assert_eq!(config["app"]["windows"][0]["create"], false);

    fn capability_files(root: &std::path::Path, output: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                capability_files(&path, output);
            } else if path.extension().and_then(|value| value.to_str()) == Some("json") {
                output.push(path);
            }
        }
    }

    let mut automatically_loaded = Vec::new();
    capability_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities"),
        &mut automatically_loaded,
    );
    assert!(!automatically_loaded.is_empty());
    let mut startup_failure_capability_seen = false;
    let mut normal_capability = None;
    for path in automatically_loaded {
        let capability: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let windows = capability["windows"]
            .as_array()
            .expect("capability windows");
        let permissions = capability["permissions"]
            .as_array()
            .expect("capability permissions");
        if !permissions.is_empty() {
            assert_eq!(
                capability["windows"],
                serde_json::json!([NORMAL_WINDOW_LABEL]),
                "a plugin-capable capability can target only the post-admission window: {}",
                path.display()
            );
        }
        if windows
            .iter()
            .any(|window| window == STARTUP_FAILURE_WINDOW_LABEL)
        {
            startup_failure_capability_seen = true;
            assert_eq!(
                capability["permissions"],
                serde_json::json!([]),
                "startup-failure window received plugin IPC permissions from {}",
                path.display()
            );
        }
        if windows.iter().any(|window| window == NORMAL_WINDOW_LABEL) {
            normal_capability = Some(capability);
        }
    }
    assert!(startup_failure_capability_seen);

    let normal = normal_capability.expect("normal workbench capability");
    assert_eq!(normal["windows"], serde_json::json!(["main"]));
    let permissions = normal["permissions"]
        .as_array()
        .expect("normal workbench capability permissions");
    for plugin in ["fs:default"] {
        assert!(
            permissions.iter().any(|permission| permission == plugin),
            "Ready-only capability lost {plugin}"
        );
    }
}
