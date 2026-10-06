use super::*;

fn report(value: serde_json::Value) -> DiagnosticReport {
    serde_json::from_value(value).unwrap()
}

#[test]
fn test_診断_相対pathだけをcwd基準の絶対pathへ解決する() {
    // Given
    let cwd = Path::new("/tmp/repository");

    // When
    let relative = absolutize(PathBuf::from("workflows"), cwd);
    let absolute = absolutize(PathBuf::from("/tmp/workflows"), cwd);

    // Then
    assert_eq!(relative, cwd.join("workflows"));
    assert_eq!(absolute, PathBuf::from("/tmp/workflows"),);
}

#[cfg(unix)]
#[test]
fn test_診断_utf8ではない対象directoryをinvalid_inputにする() {
    use std::os::unix::ffi::OsStringExt;

    // Given
    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));

    // When
    let result = target_dir_str(&path);

    // Then
    assert!(matches!(result, Err(CliError::InvalidInput(_))));
}

#[test]
fn test_診断_error_itemがある場合だけ終了コード3にする() {
    // Given
    let error = report(serde_json::json!({
        "items": [{"code":"E1","severity":"error","stage":"parse_shape","message":"m"}],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {}
    }));
    let info = report(serde_json::json!({
        "items": [{"code":"I1","severity":"info","stage":"resolve","message":"m"}],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {}
    }));
    let empty = report(serde_json::json!({
        "items": [],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {}
    }));

    // When
    let error_exit = diagnostics_exit_code(&error);
    let info_exit = diagnostics_exit_code(&info);
    let empty_exit = diagnostics_exit_code(&empty);

    // Then
    assert_eq!(error_exit, 3);
    assert_eq!(info_exit, 0);
    assert_eq!(empty_exit, 0);
}

#[test]
fn test_診断_human_readable出力がitem順と位置と対象を保持する() {
    // Given
    let report = report(serde_json::json!({
        "items": [
            {
                "code":"E1","severity":"error","stage":"parse_shape","message":"first",
                "span":{"source":"workflow.yml","start_line":2,"start_col":3,"end_line":2,"end_col":4},
                "workflow_name":"alpha","node_name":"main","field":"permission"
            },
            {"code":"I1","severity":"info","stage":"resolve","message":"second"},
            {"code":"I2","severity":"info","stage":"resolve","message":"third","workflow_name":"beta"},
            {
                "code":"I3","severity":"info","stage":"resolve","message":"fourth",
                "span":{"start_line":4,"start_col":5,"end_line":4,"end_col":6},
                "facet_kind":"policy","facet_key":"coding"
            }
        ],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {}
    }));

    // When
    let output = format_human_readable(&report);

    // Then
    assert_eq!(output.lines().count(), 6);
    assert_eq!(
        output,
        "error E1 workflow.yml:2:3 [workflow=alpha, node=main, field=permission]: first\n\
info I1: second\n\
info I2 [workflow=beta]: third\n\
info I3 4:5 [facet=policy/coding]: fourth\n\
\n\
1 error, 3 info\n"
    );
}

#[test]
fn test_診断_json出力が受信valueを再構成しない() {
    // Given
    let value = serde_json::json!({
        "items": [],
        "workflow_summaries": {},
        "facet_summaries": {},
        "facet_usage": {},
        "future_field": {"kept": true}
    });

    // When
    let output = format_json(&value).unwrap();

    // Then
    assert_eq!(
        output,
        format!("{}\n", serde_json::to_string_pretty(&value).unwrap())
    );
}

#[test]
fn test_診断_cli_sourceにdiagnostic_code_literalを持たない() {
    // Given
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("diagnostics.rs", include_str!("diagnostics.rs")),
        ("api_client.rs", include_str!("api_client.rs")),
        ("workflow.rs", include_str!("workflow.rs")),
        ("common.rs", include_str!("common.rs")),
    ];

    for (file, source) in sources {
        // When / Then
        for prefix in ["\"WF", "\"FAC"] {
            assert!(
                !source.contains(prefix),
                "diagnostic code prefix '{prefix}' must not appear in {file}"
            );
        }
    }
}
