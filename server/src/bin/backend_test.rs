use super::*;

#[test]
fn test_起動引数_通常起動と内部起動を解決する() {
    // Given
    let cases = [
        (vec![], Mode::Daemon(None)),
        (
            vec!["--data-dir", "data"],
            Mode::Daemon(Some("data".into())),
        ),
        (vec!["--internal-background-worker"], Mode::BackgroundWorker),
    ];
    for (arguments, expected) in cases {
        // When
        let arguments: Vec<_> = arguments.into_iter().map(OsString::from).collect();
        let mode = parse_arguments(&arguments);
        // Then
        assert_eq!(mode, Some(expected));
    }
}

#[test]
fn test_起動引数_値の欠落と未知の引数と余分な引数を拒否する() {
    // Given
    for arguments in [
        vec!["--data-dir"],
        vec!["--unknown"],
        vec!["--data-dir", "data", "extra"],
        vec!["--internal-background-worker", "data"],
    ] {
        // When
        let arguments: Vec<_> = arguments.into_iter().map(OsString::from).collect();
        let mode = parse_arguments(&arguments);
        // Then
        assert_eq!(mode, None);
    }
}
