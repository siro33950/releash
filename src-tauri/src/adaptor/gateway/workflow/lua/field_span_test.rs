use super::*;

#[test]
fn test_lua宣言位置_上限ちょうどは解析し超過すると空mapを返す() {
    // Given
    let mut source = "local main = sequence{ artifact = result }".to_string();
    source.push_str(&";".repeat(MAX_SPAN_SOURCE_BYTES - source.len()));
    assert_eq!(source.len(), MAX_SPAN_SOURCE_BYTES);

    // When
    let within_limit = ArtifactSpanMap::parse(&source);
    source.push(';');
    let over_limit = ArtifactSpanMap::parse(&source);

    // Then
    let span = within_limit.node_span(1, "main").unwrap();
    assert_eq!((span.start_col, span.end_col), (24, 32));
    assert!(over_limit.declarations.is_empty());
    assert_eq!(over_limit.declarations.capacity(), 0);
}

#[test]
fn test_lua宣言位置_補助領域を含む確保見積りがluaのメモリ予算内に収まる() {
    // Given
    let token_bytes = std::mem::size_of::<Token<'_>>();
    let table_bytes = std::mem::size_of::<(usize, Option<String>, Option<DiagnosticSpan>)>();
    let declaration_bytes = std::mem::size_of::<(usize, Option<String>, DiagnosticSpan)>();

    // When
    let vector_capacity_bytes =
        2 * MAX_SPAN_SOURCE_BYTES * (token_bytes + table_bytes + declaration_bytes);
    let string_bytes = 4 * MAX_SPAN_SOURCE_BYTES;

    // Then
    #[cfg(target_pointer_width = "64")]
    assert_eq!(token_bytes, 32);
    assert!(
        vector_capacity_bytes + string_bytes
            <= crate::infrastructure::lua::LuaLimits::default().memory_bytes
    );
}

#[test]
fn test_lua宣言位置_解析上限を超える定義もloadと一覧に成功する() {
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
        crate::adaptor::gateway::workflow::storage::load_workflow(&path, tmp.path()).unwrap();
    let summaries = crate::adaptor::gateway::workflow::storage::list_workflows_with_facets(
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
fn test_lua宣言位置_解析上限を超えるartifact宣言は呼び出し行で診断する() {
    // Given
    use crate::adaptor::protocol::workflow::{DiagnosticStage, Severity};
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
    let diagnosis = crate::adaptor::gateway::workflow::diagnostics::diagnose_lua_workflow_source(
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

#[test]
fn test_lua宣言位置_コメントと文字列と入れ子のfieldを区別する() {
    // Given
    let source = r#"local main = sequence({
  -- artifact = ignored, }
  children = { child{ node = command{
    command = "artifact = ignored, } \"",
    artifact = child_result,
  } } },
  --[==[ artifact = ignored,
  } ]==]
  artifact = result,
})"#;

    // When
    let span = ArtifactSpanMap::parse(source).node_span(1, "main").unwrap();

    // Then
    assert_eq!((span.start_line, span.start_col), (9, 3));
    assert_eq!((span.end_line, span.end_col), (9, 11));
}

#[test]
fn test_lua宣言位置_同じ行の別nodeのfieldを混同しない() {
    // Given
    let source = "local a = r.sequence{ name = 'a', artifact = a_result }; local b = r.sequence{ name = 'b', artifact = b_result }";

    // When
    let a = ArtifactSpanMap::parse(source).node_span(1, "a").unwrap();
    let b = ArtifactSpanMap::parse(source).node_span(1, "b").unwrap();

    // Then
    assert_eq!(a.start_col, source.find("artifact = a_result").unwrap() + 1);
    assert_eq!(b.start_col, source.find("artifact = b_result").unwrap() + 1);
    assert!(ArtifactSpanMap::parse(source)
        .node_span(1, "missing")
        .is_none());
}

#[test]
fn test_lua宣言位置_改行形式と長文字列に依存しない() {
    // Given
    let source = "local main = r.sequence{\n  children = { r.command{ command = [=[\nartifact = ignored }\n]=] } },\n  artifact = result,\n}";
    for newline in ["\n", "\r\n", "\r", "\n\r"] {
        let source = source.replace('\n', newline);

        // When
        let span = ArtifactSpanMap::parse(&source)
            .node_span(1, "main")
            .unwrap();

        // Then
        assert_eq!((span.start_line, span.start_col), (5, 3));
        assert_eq!((span.end_line, span.end_col), (5, 11));
    }
}

#[test]
fn test_lua宣言位置_該当する宣言がなければ他のtableを返さない() {
    // Given
    let source =
        "local child = command{ artifact = result }\nlocal main = sequence{ children = { child } }";

    // When / Then
    assert!(ArtifactSpanMap::parse(source)
        .node_span(2, "main")
        .is_none());
}

#[test]
fn test_lua宣言位置_引用したfield名の位置を返す() {
    // Given
    let source = "local main = sequence{\n  [\"artifact\"] = result,\n}";

    // When
    let span = ArtifactSpanMap::parse(source).node_span(1, "main").unwrap();

    // Then
    assert_eq!((span.start_line, span.start_col), (2, 4));
    assert_eq!((span.end_line, span.end_col), (2, 14));
}

#[test]
fn test_lua宣言位置_未完成の構文でも停止する() {
    // Given
    for source in [
        "}",
        "{ artifact = result",
        "--[=[",
        "local main = r.sequence{ [[",
    ] {
        // When / Then
        assert!(ArtifactSpanMap::parse(source)
            .node_span(1, "main")
            .is_none());
    }
}
