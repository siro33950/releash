use std::collections::BTreeMap;

use serde::Serialize;
use serde_saphyr::granit_parser::{Event, Parser, ScanError, Span as ParserSpan};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub(crate) struct DiagnosticSpan {
    pub(crate) start_line: usize,
    pub(crate) start_col: usize,
    pub(crate) end_line: usize,
    pub(crate) end_col: usize,
}

impl DiagnosticSpan {
    pub(crate) fn from_location(location: serde_saphyr::Location) -> Self {
        let line = usize::try_from(location.line()).unwrap_or(usize::MAX);
        let col = usize::try_from(location.column()).unwrap_or(usize::MAX);
        Self {
            start_line: line,
            start_col: col,
            end_line: line,
            end_col: col.saturating_add(1),
        }
    }

    pub(crate) fn from_scan_error(error: &ScanError) -> Self {
        let marker = error.marker();
        Self {
            start_line: marker.line(),
            start_col: marker.col() + 1,
            end_line: marker.line(),
            end_col: marker.col() + 2,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct YamlSpanMap {
    value_spans: BTreeMap<String, DiagnosticSpan>,
    key_spans: BTreeMap<String, DiagnosticSpan>,
}

impl YamlSpanMap {
    pub(crate) fn parse(source: &str) -> Result<Self, ScanError> {
        let mut parser = Parser::new_from_str(source);
        let mut events = Vec::new();
        while let Some(event) = parser.next_event() {
            events.push(event?);
        }
        let mut cursor = EventCursor { events, index: 0 };
        let mut map = Self::default();
        while let Some((event, span)) = cursor.peek().cloned() {
            match event {
                Event::StreamStart
                | Event::StreamEnd
                | Event::DocumentStart(_, _)
                | Event::DocumentEnd
                | Event::Comment(_, _) => {
                    cursor.bump();
                }
                _ => {
                    parse_node(&mut cursor, String::new(), &mut map)?;
                    if let Some(span) = diagnostic_span(&span) {
                        map.value_spans.entry(String::new()).or_insert(span);
                    }
                }
            }
        }
        Ok(map)
    }

    pub(crate) fn value_span(&self, path: &str) -> Option<DiagnosticSpan> {
        self.value_spans.get(path).copied()
    }

    pub(crate) fn key_span(&self, path: &str) -> Option<DiagnosticSpan> {
        self.key_spans.get(path).copied()
    }

    pub(crate) fn nearest_span(&self, path: &str) -> Option<DiagnosticSpan> {
        let mut current = path.to_string();
        loop {
            if let Some(span) = self
                .value_span(&current)
                .or_else(|| self.key_span(&current))
            {
                return Some(span);
            }
            let Some(parent) = parent_path(&current) else {
                break;
            };
            current = parent;
        }
        self.value_span("")
    }

    pub(crate) fn field_span(&self, path: &str) -> Option<DiagnosticSpan> {
        self.key_span(path)
            .or_else(|| self.value_span(path))
            .or_else(|| self.nearest_span(path))
    }
}

struct EventCursor<'input> {
    events: Vec<(Event<'input>, ParserSpan)>,
    index: usize,
}

impl<'input> EventCursor<'input> {
    fn peek(&self) -> Option<&(Event<'input>, ParserSpan)> {
        self.events.get(self.index)
    }

    fn bump(&mut self) -> Option<(Event<'input>, ParserSpan)> {
        let event = self.events.get(self.index).cloned();
        self.index += usize::from(event.is_some());
        event
    }
}

fn parse_node(
    cursor: &mut EventCursor<'_>,
    path: String,
    map: &mut YamlSpanMap,
) -> Result<(), ScanError> {
    let Some((event, span)) = cursor.bump() else {
        return Ok(());
    };
    if let Some(span) = diagnostic_span(&span) {
        map.value_spans.entry(path.clone()).or_insert(span);
    }
    match event {
        Event::MappingStart(_, _, _) => parse_mapping(cursor, path, map),
        Event::SequenceStart(_, _, _) => parse_sequence(cursor, path, map),
        Event::Alias(_)
        | Event::Scalar(_, _, _, _)
        | Event::Nothing
        | Event::StreamStart
        | Event::StreamEnd
        | Event::DocumentStart(_, _)
        | Event::DocumentEnd
        | Event::SequenceEnd
        | Event::MappingEnd
        | Event::Comment(_, _) => Ok(()),
    }
}

fn parse_mapping(
    cursor: &mut EventCursor<'_>,
    path: String,
    map: &mut YamlSpanMap,
) -> Result<(), ScanError> {
    loop {
        skip_presentation_events(cursor);
        let Some((event, _)) = cursor.peek() else {
            return Ok(());
        };
        if matches!(event, Event::MappingEnd) {
            cursor.bump();
            return Ok(());
        }
        let key = match cursor.bump() {
            Some((Event::Scalar(value, _, _, _), span)) => {
                let span = diagnostic_span(&span).expect("parser spans always have coordinates");
                let child = child_path(&path, &value);
                map.key_spans.insert(child.clone(), span);
                (child, span)
            }
            Some((_, _)) => continue,
            None => return Ok(()),
        };
        parse_node(cursor, key.0, map)?;
    }
}

fn parse_sequence(
    cursor: &mut EventCursor<'_>,
    path: String,
    map: &mut YamlSpanMap,
) -> Result<(), ScanError> {
    let mut index = 0usize;
    loop {
        skip_presentation_events(cursor);
        let Some((event, _)) = cursor.peek() else {
            return Ok(());
        };
        if matches!(event, Event::SequenceEnd) {
            cursor.bump();
            return Ok(());
        }
        parse_node(cursor, format!("{path}[{index}]"), map)?;
        index += 1;
    }
}

fn skip_presentation_events(cursor: &mut EventCursor<'_>) {
    while let Some((event, _)) = cursor.peek() {
        match event {
            Event::Comment(_, _)
            | Event::DocumentStart(_, _)
            | Event::DocumentEnd
            | Event::StreamStart
            | Event::StreamEnd => {
                cursor.bump();
            }
            _ => break,
        }
    }
}

fn child_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_string()
    } else {
        format!("{parent}.{key}")
    }
}

fn parent_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    if let Some(prefix) = path.strip_suffix(']') {
        let start = prefix.rfind('[')?;
        return Some(prefix[..start].to_string());
    }
    path.rsplit_once('.')
        .map(|(parent, _)| parent.to_string())
        .or_else(|| Some(String::new()))
}

fn diagnostic_span(span: &ParserSpan) -> Option<DiagnosticSpan> {
    Some(DiagnosticSpan {
        start_line: span.start.line(),
        start_col: span.start.col() + 1,
        end_line: span.end.line(),
        end_col: span.end.col() + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = r#"name: span-test
schemas:
  review:
    type: object
nodes:
  - name: entry
    session:
      provider: claude
      facets:
        instruction: entry
    rules:
      - when:
          on: passed
          then: done
        next: done
  - name: done
    session:
      provider: claude
"#;

    #[test]
    fn field_span_uses_exact_key_coordinates_with_one_based_columns() {
        let map = YamlSpanMap::parse(SOURCE).unwrap();

        assert_eq!(
            map.field_span("name"),
            Some(DiagnosticSpan {
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 5,
            })
        );
        assert_eq!(
            map.field_span("nodes[0].rules[0].when.on"),
            Some(DiagnosticSpan {
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
            map.key_span("nodes[1].name"),
            Some(DiagnosticSpan {
                start_line: 16,
                start_col: 5,
                end_line: 16,
                end_col: 9,
            })
        );
        assert!(map.value_span("nodes[0].rules[0]").is_some());
        assert!(map.value_span("nodes[1]").is_some());
    }

    #[test]
    fn nearest_span_falls_back_to_nearest_parent_not_root() {
        let map = YamlSpanMap::parse(SOURCE).unwrap();
        let nearest_parent = map.nearest_span("nodes[0].rules[0].when").unwrap();
        let missing_child = map
            .nearest_span("nodes[0].rules[0].when.missing_field")
            .unwrap();

        assert_eq!(missing_child, nearest_parent);
        assert_ne!(missing_child, map.value_span("").unwrap());
    }

    #[test]
    fn parent_path_handles_sequence_indices_and_dotted_keys() {
        assert_eq!(parent_path("nodes[0]").as_deref(), Some("nodes"));
        assert_eq!(
            parent_path("nodes[0].rules[0]").as_deref(),
            Some("nodes[0].rules")
        );
        assert_eq!(
            parent_path("nodes[0].rules[0].when.on").as_deref(),
            Some("nodes[0].rules[0].when")
        );
        assert_eq!(
            parent_path("schemas.review.properties.status").as_deref(),
            Some("schemas.review.properties")
        );
        assert_eq!(parent_path("name").as_deref(), Some(""));
        assert_eq!(parent_path("").as_deref(), None);
    }
}
