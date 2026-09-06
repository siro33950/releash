use crate::adaptor::protocol::workflow::DiagnosticSpan;

// 64bit の Token は32 byte。Vec の容量増加と table・宣言の補助領域も含め64MiB以内に収める。
const MAX_SPAN_SOURCE_BYTES: usize = 128 * 1024;

#[derive(Debug, Default)]
pub(super) struct ArtifactSpanMap {
    declarations: Vec<(usize, Option<String>, DiagnosticSpan)>,
}

impl ArtifactSpanMap {
    pub(super) fn parse(source: &str) -> Self {
        if source.len() > MAX_SPAN_SOURCE_BYTES {
            return Self::default();
        }
        let tokens = tokens(source);
        let mut tables = Vec::new();
        let mut declarations = Vec::new();
        for (index, token) in tokens.iter().enumerate() {
            match token.text {
                "{" => tables.push((index, None, None)),
                "}" => {
                    let Some((start, name, Some(span))) = tables.pop() else {
                        continue;
                    };
                    let Some(mut prefix) = start.checked_sub(1) else {
                        continue;
                    };
                    if tokens[prefix].text == "(" {
                        let Some(previous) = prefix.checked_sub(1) else {
                            continue;
                        };
                        prefix = previous;
                    }
                    declarations.push((tokens[prefix].line, name, span));
                }
                _ => {
                    let Some((_, name, span)) = tables.last_mut() else {
                        continue;
                    };
                    let Some((key, value_index)) = table_field(&tokens, index) else {
                        continue;
                    };
                    if key == "name" {
                        *name = tokens
                            .get(value_index)
                            .and_then(|value| value.string())
                            .map(str::to_string);
                    }
                    if key == "artifact" {
                        let key_token = if token.text == "[" {
                            &tokens[index + 1]
                        } else {
                            token
                        };
                        *span = Some(key_token.span());
                    }
                }
            }
        }
        Self { declarations }
    }

    pub(super) fn node_span(&self, call_line: usize, node_name: &str) -> Option<DiagnosticSpan> {
        let mut candidates = self.declarations.iter().filter(|(line, name, _)| {
            *line == call_line && name.as_deref().is_none_or(|name| name == node_name)
        });
        let (_, _, span) = candidates.next()?;
        candidates.next().is_none().then(|| span.clone())
    }
}

fn table_field<'a>(tokens: &[Token<'a>], index: usize) -> Option<(&'a str, usize)> {
    if !matches!(tokens.get(index.checked_sub(1)?)?.text, "{" | "," | ";") {
        return None;
    }
    let token = &tokens[index];
    if token.text == "[" {
        let key = tokens.get(index + 1)?.string()?;
        if tokens.get(index + 2)?.text == "]" && tokens.get(index + 3)?.text == "=" {
            return Some((key, index + 4));
        }
    } else if tokens.get(index + 1)?.text == "=" {
        return Some((token.text, index + 2));
    }
    None
}

struct Token<'a> {
    text: &'a str,
    line: usize,
    col: usize,
}

impl<'a> Token<'a> {
    fn string(&self) -> Option<&'a str> {
        let quote = self.text.chars().next()?;
        if matches!(quote, '\'' | '"') {
            self.text.strip_prefix(quote)?.strip_suffix(quote)
        } else {
            None
        }
    }

    fn span(&self) -> DiagnosticSpan {
        DiagnosticSpan {
            source: None,
            start_line: self.line,
            start_col: self.col,
            end_line: self.line,
            end_col: self.col + self.text.chars().count(),
        }
    }
}

fn tokens(source: &str) -> Vec<Token<'_>> {
    let bytes = source.as_bytes();
    let mut result = Vec::new();
    let (mut index, mut line, mut col) = (0, 1, 1);
    while index < bytes.len() {
        let start = index;
        let token_line = line;
        let token_col = col;
        let comment = source[start..].starts_with("--");
        let content_start = start + if comment { 2 } else { 0 };
        if let Some(end) = long_bracket_end(source, content_start) {
            index = end;
        } else if comment {
            index = source[start..]
                .find(['\r', '\n'])
                .map_or(bytes.len(), |offset| start + offset);
        } else if matches!(bytes[start], b'\'' | b'"') {
            index += 1;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if byte == b'\\' {
                    index = (index + 1).min(bytes.len());
                } else if byte == bytes[start] {
                    break;
                }
            }
        } else if bytes[start].is_ascii_alphanumeric() || bytes[start] == b'_' {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
        } else {
            index += source[start..].chars().next().unwrap().len_utf8();
            if matches!(bytes[start], b'\r' | b'\n')
                && index < bytes.len()
                && matches!(bytes[index], b'\r' | b'\n')
                && bytes[index] != bytes[start]
            {
                index += 1;
            }
        }
        if !comment && !bytes[start].is_ascii_whitespace() {
            result.push(Token {
                text: &source[start..index],
                line: token_line,
                col: token_col,
            });
        }
        let mut cursor = start;
        while cursor < index {
            if matches!(bytes[cursor], b'\r' | b'\n') {
                let newline = bytes[cursor];
                cursor += 1;
                if cursor < index
                    && matches!(bytes[cursor], b'\r' | b'\n')
                    && bytes[cursor] != newline
                {
                    cursor += 1;
                }
                line += 1;
                col = 1;
            } else {
                cursor += source[cursor..].chars().next().unwrap().len_utf8();
                col += 1;
            }
        }
    }
    result
}

fn long_bracket_end(source: &str, start: usize) -> Option<usize> {
    let rest = source.get(start..)?.strip_prefix('[')?;
    let level = rest.bytes().take_while(|byte| *byte == b'=').count();
    let body = rest.get(level..)?.strip_prefix('[')?;
    let closing = format!("]{}]", "=".repeat(level));
    Some(body.find(&closing).map_or(source.len(), |offset| {
        offset + start + level + 2 + closing.len()
    }))
}

#[cfg(test)]
#[path = "field_span_test.rs"]
mod field_span_tests;
