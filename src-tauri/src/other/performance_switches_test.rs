use super::TerminalPerformanceSwitches;

fn reader<'a>(entries: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        entries
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    }
}

#[test]
fn test_env未設定なら全switchが無効でdefaultと一致する() {
    let switches = TerminalPerformanceSwitches::from_env_reader(reader(&[]));
    assert_eq!(switches, TerminalPerformanceSwitches::default());
    assert!(!switches.disable_output_flow_control);
    assert!(!switches.disable_terminal_journal);
    assert!(!switches.disable_renderer_write_serialization);
}

#[test]
fn test_各envの真値表現でswitchが個別に有効になる() {
    let switches = TerminalPerformanceSwitches::from_env_reader(reader(&[(
        "RELEASH_PERF_DISABLE_OUTPUT_FLOW_CONTROL",
        "1",
    )]));
    assert!(switches.disable_output_flow_control);

    let switches = TerminalPerformanceSwitches::from_env_reader(reader(&[(
        "RELEASH_PERF_DISABLE_TERMINAL_JOURNAL",
        "TRUE",
    )]));
    assert!(switches.disable_terminal_journal);

    let switches = TerminalPerformanceSwitches::from_env_reader(reader(&[(
        "RELEASH_PERF_DISABLE_RENDERER_WRITE_SERIALIZATION",
        " 1 ",
    )]));
    assert!(switches.disable_renderer_write_serialization);
}

#[test]
fn test_偽値や空文字ではswitchが有効にならない() {
    for value in ["0", "false", "", "no", "yes", "on"] {
        let switches = TerminalPerformanceSwitches::from_env_reader(reader(&[(
            "RELEASH_PERF_DISABLE_OUTPUT_FLOW_CONTROL",
            value,
        )]));
        assert!(
            !switches.disable_output_flow_control,
            "value {value:?} must not enable the switch"
        );
    }
}

#[test]
fn test_disable_webgl_renderer_switchはenvで有効になりdefaultは無効() {
    let switches = TerminalPerformanceSwitches::from_env_reader(reader(&[]));
    assert!(!switches.disable_webgl_renderer);

    let switches = TerminalPerformanceSwitches::from_env_reader(reader(&[(
        "RELEASH_PERF_DISABLE_WEBGL_RENDERER",
        "1",
    )]));
    assert!(switches.disable_webgl_renderer);

    let switches = TerminalPerformanceSwitches::from_env_reader(reader(&[(
        "RELEASH_PERF_DISABLE_TERMINAL_WEBSOCKET",
        "1",
    )]));
    assert!(switches.disable_terminal_websocket);
}
