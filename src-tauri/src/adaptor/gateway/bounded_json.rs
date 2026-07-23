//! Allocation-bounded streaming helpers for legacy JSON sources.
//!
//! These readers validate JSON syntax while bytes are still coming from the
//! source. A record that crosses its byte ceiling is rejected before another
//! byte is appended to an owned buffer.

use std::collections::BTreeMap;
use std::io::BufRead;

const MAX_JSON_DEPTH: usize = 128;
const DECODED_NODE_RESERVE_BYTES: usize = 1024;
const DECODED_BYTE_EXPANSION: usize = 2;

struct DecodedBudget {
    remaining: usize,
}

impl DecodedBudget {
    fn new(max_bytes: usize) -> Self {
        Self {
            remaining: max_bytes,
        }
    }

    fn charge(&mut self, bytes: usize) -> Result<(), String> {
        self.remaining = self.remaining.checked_sub(bytes).ok_or_else(|| {
            "legacy JSON decoded allocation estimate exceeds its bound".to_string()
        })?;
        Ok(())
    }
}

enum Capture<'a> {
    Discard,
    Bounded {
        bytes: &'a mut Vec<u8>,
        max_bytes: usize,
        decoded_budget: &'a mut DecodedBudget,
    },
    Validate {
        decoded_budget: &'a mut DecodedBudget,
    },
}

struct JsonStream<R> {
    reader: R,
}

impl<R: BufRead> JsonStream<R> {
    fn new(reader: R) -> Self {
        Self { reader }
    }

    fn into_inner(self) -> R {
        self.reader
    }

    fn peek(&mut self) -> Result<Option<u8>, String> {
        self.reader
            .fill_buf()
            .map_err(|error| error.to_string())
            .map(|available| available.first().copied())
    }

    fn take(&mut self, capture: &mut Capture<'_>) -> Result<u8, String> {
        let byte = self
            .peek()?
            .ok_or_else(|| "unexpected end of JSON input".to_string())?;
        match capture {
            Capture::Discard => {}
            Capture::Bounded {
                bytes,
                max_bytes,
                decoded_budget,
            } => {
                if bytes.len() >= *max_bytes {
                    return Err(format!(
                        "one legacy JSON record exceeds {} bytes",
                        max_bytes
                    ));
                }
                decoded_budget.charge(DECODED_BYTE_EXPANSION)?;
                bytes.push(byte);
            }
            Capture::Validate { decoded_budget } => {
                decoded_budget.charge(DECODED_BYTE_EXPANSION)?;
            }
        }
        self.reader.consume(1);
        Ok(byte)
    }

    fn consume_whitespace(&mut self, capture: &mut Capture<'_>) -> Result<(), String> {
        while self.peek()?.is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.take(capture)?;
        }
        Ok(())
    }

    fn expect(&mut self, expected: u8, capture: &mut Capture<'_>) -> Result<(), String> {
        let actual = self.take(capture)?;
        if actual != expected {
            return Err(format!(
                "invalid JSON: expected byte {expected:#04x}, found {actual:#04x}"
            ));
        }
        Ok(())
    }

    fn parse_value(&mut self, capture: &mut Capture<'_>, depth: usize) -> Result<(), String> {
        if depth > MAX_JSON_DEPTH {
            return Err("legacy JSON nesting exceeds the supported depth".to_string());
        }
        match capture {
            Capture::Discard => {}
            Capture::Bounded { decoded_budget, .. } | Capture::Validate { decoded_budget } => {
                decoded_budget.charge(DECODED_NODE_RESERVE_BYTES)?;
            }
        }
        match self
            .peek()?
            .ok_or_else(|| "unexpected end of JSON value".to_string())?
        {
            b'"' => self.parse_string(capture),
            b'{' => self.parse_object(capture, depth),
            b'[' => self.parse_array(capture, depth),
            b't' => self.parse_literal(b"true", capture),
            b'f' => self.parse_literal(b"false", capture),
            b'n' => self.parse_literal(b"null", capture),
            b'-' | b'0'..=b'9' => self.parse_number(capture),
            byte => Err(format!("invalid JSON value prefix {byte:#04x}")),
        }
    }

    fn parse_literal(&mut self, expected: &[u8], capture: &mut Capture<'_>) -> Result<(), String> {
        for expected_byte in expected {
            self.expect(*expected_byte, capture)?;
        }
        Ok(())
    }

    fn parse_number(&mut self, capture: &mut Capture<'_>) -> Result<(), String> {
        if self.peek()? == Some(b'-') {
            self.take(capture)?;
        }
        match self.peek()? {
            Some(b'0') => {
                self.take(capture)?;
            }
            Some(b'1'..=b'9') => {
                while self.peek()?.is_some_and(|byte| byte.is_ascii_digit()) {
                    self.take(capture)?;
                }
            }
            _ => return Err("invalid JSON number integer component".to_string()),
        }
        if self.peek()? == Some(b'.') {
            self.take(capture)?;
            if !self.peek()?.is_some_and(|byte| byte.is_ascii_digit()) {
                return Err("invalid JSON number fraction".to_string());
            }
            while self.peek()?.is_some_and(|byte| byte.is_ascii_digit()) {
                self.take(capture)?;
            }
        }
        if self.peek()?.is_some_and(|byte| matches!(byte, b'e' | b'E')) {
            self.take(capture)?;
            if self.peek()?.is_some_and(|byte| matches!(byte, b'+' | b'-')) {
                self.take(capture)?;
            }
            if !self.peek()?.is_some_and(|byte| byte.is_ascii_digit()) {
                return Err("invalid JSON number exponent".to_string());
            }
            while self.peek()?.is_some_and(|byte| byte.is_ascii_digit()) {
                self.take(capture)?;
            }
        }
        Ok(())
    }

    fn parse_string(&mut self, capture: &mut Capture<'_>) -> Result<(), String> {
        self.expect(b'"', capture)?;
        loop {
            let byte = self.take(capture)?;
            match byte {
                b'"' => return Ok(()),
                b'\\' => match self.take(capture)? {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                    b'u' => {
                        for _ in 0..4 {
                            if !self.take(capture)?.is_ascii_hexdigit() {
                                return Err("invalid JSON unicode escape".to_string());
                            }
                        }
                    }
                    _ => return Err("invalid JSON string escape".to_string()),
                },
                0x00..=0x1f => return Err("unescaped control byte in JSON string".to_string()),
                0x20..=0x7f => {}
                _ => self.parse_utf8_tail(byte, capture)?,
            }
        }
    }

    fn parse_utf8_tail(&mut self, leading: u8, capture: &mut Capture<'_>) -> Result<(), String> {
        let ranges: &[(u8, u8)] = match leading {
            0xc2..=0xdf => &[(0x80, 0xbf)],
            0xe0 => &[(0xa0, 0xbf), (0x80, 0xbf)],
            0xe1..=0xec | 0xee..=0xef => &[(0x80, 0xbf), (0x80, 0xbf)],
            0xed => &[(0x80, 0x9f), (0x80, 0xbf)],
            0xf0 => &[(0x90, 0xbf), (0x80, 0xbf), (0x80, 0xbf)],
            0xf1..=0xf3 => &[(0x80, 0xbf), (0x80, 0xbf), (0x80, 0xbf)],
            0xf4 => &[(0x80, 0x8f), (0x80, 0xbf), (0x80, 0xbf)],
            _ => return Err("invalid UTF-8 in JSON string".to_string()),
        };
        for (minimum, maximum) in ranges {
            let byte = self.take(capture)?;
            if byte < *minimum || byte > *maximum {
                return Err("invalid UTF-8 in JSON string".to_string());
            }
        }
        Ok(())
    }

    fn parse_array(&mut self, capture: &mut Capture<'_>, depth: usize) -> Result<(), String> {
        self.expect(b'[', capture)?;
        self.consume_whitespace(capture)?;
        if self.peek()? == Some(b']') {
            self.take(capture)?;
            return Ok(());
        }
        loop {
            self.parse_value(capture, depth + 1)?;
            self.consume_whitespace(capture)?;
            match self.take(capture)? {
                b',' => self.consume_whitespace(capture)?,
                b']' => return Ok(()),
                _ => return Err("invalid JSON array delimiter".to_string()),
            }
        }
    }

    fn parse_object(&mut self, capture: &mut Capture<'_>, depth: usize) -> Result<(), String> {
        self.expect(b'{', capture)?;
        self.consume_whitespace(capture)?;
        if self.peek()? == Some(b'}') {
            self.take(capture)?;
            return Ok(());
        }
        loop {
            if self.peek()? != Some(b'"') {
                return Err("JSON object key must be a string".to_string());
            }
            match capture {
                Capture::Discard => {}
                Capture::Bounded { decoded_budget, .. } | Capture::Validate { decoded_budget } => {
                    decoded_budget.charge(DECODED_NODE_RESERVE_BYTES)?;
                }
            }
            self.parse_string(capture)?;
            self.consume_whitespace(capture)?;
            self.expect(b':', capture)?;
            self.consume_whitespace(capture)?;
            self.parse_value(capture, depth + 1)?;
            self.consume_whitespace(capture)?;
            match self.take(capture)? {
                b',' => self.consume_whitespace(capture)?,
                b'}' => return Ok(()),
                _ => return Err("invalid JSON object delimiter".to_string()),
            }
        }
    }

    fn capture_value(&mut self, max_bytes: usize) -> Result<Vec<u8>, String> {
        let mut decoded_budget = DecodedBudget::new(max_bytes);
        self.capture_value_with_budget(max_bytes, &mut decoded_budget)
    }

    fn capture_value_with_budget(
        &mut self,
        max_bytes: usize,
        decoded_budget: &mut DecodedBudget,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        self.parse_value(
            &mut Capture::Bounded {
                bytes: &mut bytes,
                max_bytes,
                decoded_budget,
            },
            0,
        )?;
        Ok(bytes)
    }

    fn skip_value(&mut self) -> Result<(), String> {
        self.parse_value(&mut Capture::Discard, 0)
    }

    fn validate_value(&mut self, decoded_budget: &mut DecodedBudget) -> Result<(), String> {
        self.parse_value(&mut Capture::Validate { decoded_budget }, 0)
    }

    fn finish(mut self) -> Result<R, String> {
        self.consume_whitespace(&mut Capture::Discard)?;
        if self.peek()?.is_some() {
            return Err("trailing bytes after legacy JSON value".to_string());
        }
        Ok(self.into_inner())
    }
}

pub(crate) fn stream_json_array<R, F>(
    reader: R,
    max_record_bytes: usize,
    mut visit: F,
) -> Result<(R, u64), String>
where
    R: BufRead,
    F: FnMut(u64, &[u8]) -> Result<(), String>,
{
    let mut stream = JsonStream::new(reader);
    stream.consume_whitespace(&mut Capture::Discard)?;
    stream.expect(b'[', &mut Capture::Discard)?;
    stream.consume_whitespace(&mut Capture::Discard)?;
    let mut ordinal = 0_u64;
    if stream.peek()? != Some(b']') {
        loop {
            let raw = stream.capture_value(max_record_bytes)?;
            visit(ordinal, &raw)?;
            ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| "legacy record ordinal overflow".to_string())?;
            stream.consume_whitespace(&mut Capture::Discard)?;
            match stream.take(&mut Capture::Discard)? {
                b',' => stream.consume_whitespace(&mut Capture::Discard)?,
                b']' => break,
                _ => return Err("invalid legacy JSON record array delimiter".to_string()),
            }
        }
    } else {
        stream.take(&mut Capture::Discard)?;
    }
    Ok((stream.finish()?, ordinal))
}

pub(crate) fn collect_selected_object_fields<R, F>(
    reader: R,
    max_field_bytes: usize,
    max_selected_bytes: usize,
    mut selected: F,
) -> Result<(R, BTreeMap<String, Vec<u8>>), String>
where
    R: BufRead,
    F: FnMut(&str) -> bool,
{
    let mut stream = JsonStream::new(reader);
    stream.consume_whitespace(&mut Capture::Discard)?;
    stream.expect(b'{', &mut Capture::Discard)?;
    stream.consume_whitespace(&mut Capture::Discard)?;
    let mut fields = BTreeMap::new();
    let mut selected_bytes = 0_usize;
    let mut decoded_budget = DecodedBudget::new(max_selected_bytes);
    if stream.peek()? != Some(b'}') {
        loop {
            let raw_key = stream.capture_value(max_field_bytes)?;
            let key: String = serde_json::from_slice(&raw_key)
                .map_err(|error| format!("invalid legacy JSON object key: {error}"))?;
            stream.consume_whitespace(&mut Capture::Discard)?;
            stream.expect(b':', &mut Capture::Discard)?;
            stream.consume_whitespace(&mut Capture::Discard)?;
            if selected(&key) {
                let remaining = max_selected_bytes
                    .checked_sub(selected_bytes)
                    .ok_or_else(|| "legacy selected JSON fields exceed their bound".to_string())?;
                let value = stream.capture_value_with_budget(remaining, &mut decoded_budget)?;
                selected_bytes = selected_bytes
                    .checked_add(value.len())
                    .ok_or_else(|| "legacy selected JSON field byte count overflow".to_string())?;
                if fields.insert(key, value).is_some() {
                    return Err("duplicate selected field in legacy JSON object".to_string());
                }
            } else {
                stream.skip_value()?;
            }
            stream.consume_whitespace(&mut Capture::Discard)?;
            match stream.take(&mut Capture::Discard)? {
                b',' => stream.consume_whitespace(&mut Capture::Discard)?,
                b'}' => break,
                _ => return Err("invalid legacy JSON object delimiter".to_string()),
            }
        }
    } else {
        stream.take(&mut Capture::Discard)?;
    }
    Ok((stream.finish()?, fields))
}

pub(crate) fn stream_ndjson_records<R, F>(
    mut reader: R,
    max_record_bytes: usize,
    mut visit: F,
) -> Result<(R, u64), String>
where
    R: BufRead,
    F: FnMut(u64, u64, &[u8]) -> Result<(), String>,
{
    let mut raw = Vec::new();
    let mut ordinal = 0_u64;
    let mut source_byte_offset = 0_u64;
    loop {
        raw.clear();
        let mut terminated = false;
        loop {
            let (copy_bytes, consume_bytes, found_newline) = {
                let available = reader.fill_buf().map_err(|error| error.to_string())?;
                if available.is_empty() {
                    (0, 0, false)
                } else if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
                    (position, position + 1, true)
                } else {
                    (available.len(), available.len(), false)
                }
            };
            if consume_bytes == 0 {
                break;
            }
            if raw.len().saturating_add(copy_bytes) > max_record_bytes {
                return Err(format!(
                    "one legacy NDJSON record exceeds {max_record_bytes} bytes"
                ));
            }
            {
                let available = reader.fill_buf().map_err(|error| error.to_string())?;
                raw.extend_from_slice(&available[..copy_bytes]);
            }
            reader.consume(consume_bytes);
            source_byte_offset = source_byte_offset
                .checked_add(consume_bytes as u64)
                .ok_or_else(|| "legacy NDJSON source byte offset overflow".to_string())?;
            if found_newline {
                terminated = true;
                break;
            }
        }
        if raw.is_empty() && !terminated {
            break;
        }
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
        if raw.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        validate_json_value(&raw, max_record_bytes)?;
        visit(ordinal, source_byte_offset, &raw)?;
        ordinal = ordinal
            .checked_add(1)
            .ok_or_else(|| "legacy NDJSON record ordinal overflow".to_string())?;
    }
    Ok((reader, ordinal))
}

fn validate_json_value(raw: &[u8], max_decoded_bytes: usize) -> Result<(), String> {
    let mut stream = JsonStream::new(std::io::BufReader::new(std::io::Cursor::new(raw)));
    let mut budget = DecodedBudget::new(max_decoded_bytes);
    stream.consume_whitespace(&mut Capture::Discard)?;
    stream.validate_value(&mut budget)?;
    stream.finish().map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::{stream_json_array, stream_ndjson_records};

    #[test]
    fn json_array_rejects_one_element_before_crossing_the_record_bound() {
        const LIMIT: usize = 16 * 1024 * 1024;
        let mut source = Vec::with_capacity(LIMIT + 4);
        source.extend_from_slice(b"[\"");
        source.extend(std::iter::repeat_n(b'x', LIMIT));
        source.extend_from_slice(b"\"]");

        let result = stream_json_array(BufReader::new(Cursor::new(source)), LIMIT, |_, _| Ok(()));

        assert!(result.unwrap_err().contains("exceeds"));
    }

    #[test]
    fn ndjson_rejects_one_line_before_crossing_the_record_bound() {
        const LIMIT: usize = 16 * 1024 * 1024;
        let source = std::iter::repeat_n(b'x', LIMIT + 1).collect::<Vec<_>>();

        let result =
            stream_ndjson_records(BufReader::new(Cursor::new(source)), LIMIT, |_, _, _| Ok(()));

        assert!(result.unwrap_err().contains("exceeds"));
    }

    #[test]
    fn malformed_ndjson_record_is_rejected_before_the_decoder_callback() {
        let mut called = false;
        let result = stream_ndjson_records(
            BufReader::new(Cursor::new(br#"{"unterminated":"value"#)),
            16 * 1024 * 1024,
            |_, _, _| {
                called = true;
                Ok(())
            },
        );

        assert!(result.is_err());
        assert!(!called);
    }
}
