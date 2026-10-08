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
