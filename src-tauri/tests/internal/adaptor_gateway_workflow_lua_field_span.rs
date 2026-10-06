use releash_lib::test_support::integration::workflow::DiagnosticSpan;
use releash_lib::test_support::integration::workflow::MAX_SPAN_SOURCE_BYTES;

#[test]
pub fn test_lua宣言位置_解析上限を超える定義もloadと一覧に成功する() {
    // Given
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("example.lua");
    let source = format!(
        r#"local r = require('releash')
return r.workflow{{ name = 'example', description = 'large definition', main = r.sequence{{
  children = {{ r.child{{ node = r.command{{ name = 'check', command = 'true' }} }} }},
}} }}
--{}
"#,
        " ".repeat(MAX_SPAN_SOURCE_BYTES)
    );
    std::fs::write(&path, &source).unwrap();

    // When
    let loaded =
        releash_lib::test_support::integration::workflow::load_workflow(&path, tmp.path()).unwrap();
    let summaries = releash_lib::test_support::integration::workflow::list_workflows_with_facets(
        tmp.path(),
        tmp.path(),
    )
    .unwrap();

    // Then
    assert!(source.len() > MAX_SPAN_SOURCE_BYTES);
    assert_eq!(loaded.name, "example");
    let custom: Vec<_> = summaries
        .iter()
        .filter(|summary| !summary.builtin)
        .collect();
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].name, "example");
    assert_eq!(custom[0].description, "large definition");
}

#[test]
pub fn test_lua宣言位置_解析上限を超えるartifact宣言は呼び出し行で診断する() {
    // Given
    use releash_lib::test_support::integration::workflow::DiagnosticStage;
    use releash_lib::test_support::integration::workflow::Severity;
    let tmp = tempfile::tempdir().unwrap();
    let source = format!(
        r#"local r = require('releash')
local result = r.schema.object{{ properties = {{}} }}
local main = r.sequence{{
    artifact = result,
    children = {{ r.child{{ node = r.command{{ command = 'true' }} }} }},
}}
return r.workflow{{ name = 'example', description = 'example', main = main }}
--{}
"#,
        " ".repeat(MAX_SPAN_SOURCE_BYTES)
    );

    // When
    let diagnosis = releash_lib::test_support::integration::workflow::diagnose_lua_workflow_source(
        "example.lua",
        &source,
        tmp.path(),
        tmp.path(),
        None,
    );

    // Then
    assert!(source.len() > MAX_SPAN_SOURCE_BYTES);
    assert_eq!(
        diagnosis.diagnostics.len(),
        1,
        "{:?}",
        diagnosis.diagnostics
    );
    let diagnostic = &diagnosis.diagnostics[0];
    assert_eq!(diagnostic.code, "WFS008");
    assert_eq!(diagnostic.stage, DiagnosticStage::ParseShape);
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(diagnostic.field.as_deref(), Some("artifact"));
    assert_eq!(
        diagnostic.span,
        Some(DiagnosticSpan {
            source: Some("example.lua".to_string()),
            start_line: 3,
            start_col: 1,
            end_line: 3,
            end_col: 2,
        })
    );
}
