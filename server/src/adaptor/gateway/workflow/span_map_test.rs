pub(crate) mod tests {
    use super::super::*;

    const SOURCE: &str = r#"name: span-test
schemas:
  review:
    type: object
nodes:
  main:
    session:
      provider: claude
      facets:
        instruction: entry
    rules:
      - when:
          on: passed
          then: done
        next: done
  done:
    session:
      provider: claude
"#;

    #[test]
    fn field_span_uses_exact_key_coordinates_with_one_based_columns() {
        let map = YamlSpanMap::parse(SOURCE).unwrap();

        assert_eq!(
            map.field_span("name"),
            Some(DiagnosticSpan {
                source: None,
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 5,
            })
        );
        assert_eq!(
            map.field_span("nodes.main.rules[0].when.on"),
            Some(DiagnosticSpan {
                source: None,
                start_line: 13,
                start_col: 11,
                end_line: 13,
                end_col: 13,
            })
        );
    }

    #[test]
    fn sequence_index_paths_are_recorded_for_nested_items() {
        let map = YamlSpanMap::parse(SOURCE).unwrap();

        assert_eq!(
            map.key_span("nodes.done"),
            Some(DiagnosticSpan {
                source: None,
                start_line: 16,
                start_col: 3,
                end_line: 16,
                end_col: 7,
            })
        );
        assert!(map.value_span("nodes.main.rules[0]").is_some());
        assert!(map.value_span("nodes.done").is_some());
    }

    #[test]
    fn nearest_span_falls_back_to_nearest_parent_not_root() {
        let map = YamlSpanMap::parse(SOURCE).unwrap();
        let nearest_parent = map.nearest_span("nodes.main.rules[0].when").unwrap();
        let missing_child = map
            .nearest_span("nodes.main.rules[0].when.missing_field")
            .unwrap();

        assert_eq!(missing_child, nearest_parent);
        assert_ne!(missing_child, map.value_span("").unwrap());
    }

    #[test]
    fn alias_paths_fall_back_to_the_alias_token() {
        let source = "defaults: &defaults\n  provider: claude\nsession: *defaults\n";
        let map = YamlSpanMap::parse(source).unwrap();
        let alias_span = DiagnosticSpan {
            source: None,
            start_line: 3,
            start_col: 10,
            end_line: 3,
            end_col: 19,
        };

        assert_eq!(map.value_span("session"), Some(alias_span.clone()));
        assert_eq!(map.nearest_span("session.provider"), Some(alias_span));
    }

    #[test]
    fn parent_path_handles_sequence_indices_and_dotted_keys() {
        assert_eq!(parent_path("rules[0]").as_deref(), Some("rules"));
        assert_eq!(
            parent_path("nodes.main.rules[0]").as_deref(),
            Some("nodes.main.rules")
        );
        assert_eq!(
            parent_path("nodes.main.rules[0].when.on").as_deref(),
            Some("nodes.main.rules[0].when")
        );
        assert_eq!(
            parent_path("schemas.review.properties.status").as_deref(),
            Some("schemas.review.properties")
        );
        assert_eq!(parent_path("name").as_deref(), Some(""));
        assert_eq!(parent_path("").as_deref(), None);
    }
}
