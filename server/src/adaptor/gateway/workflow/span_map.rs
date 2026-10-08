use std::collections::BTreeMap;

use serde_saphyr::granit_parser::{Event, Parser, ScanError, Span as ParserSpan};

use crate::usecase::workflow::diagnostic_dto::DiagnosticSpan;

#[derive(Debug, Clone, Default)]
pub struct YamlSpanMap {
    value_spans: BTreeMap<String, DiagnosticSpan>,
    key_spans: BTreeMap<String, DiagnosticSpan>,
}

impl YamlSpanMap {
    pub fn parse(source: &str) -> Result<Self, ScanError> {
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
        self.value_spans.get(path).cloned()
    }

    pub(crate) fn key_span(&self, path: &str) -> Option<DiagnosticSpan> {
        self.key_spans.get(path).cloned()
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

    pub fn field_span(&self, path: &str) -> Option<DiagnosticSpan> {
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
        Event::Alias(_) => Ok(()),
        _ => Ok(()),
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
                map.key_spans.entry(child.clone()).or_insert(span.clone());
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
        source: None,
        start_line: span.start.line(),
        start_col: span.start.col() + 1,
        end_line: span.end.line(),
        end_col: span.end.col() + 1,
    })
}

#[cfg(test)]
#[path = "span_map_test.rs"]
mod span_map_tests;
