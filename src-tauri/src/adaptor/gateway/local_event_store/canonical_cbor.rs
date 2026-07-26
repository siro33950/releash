//! Canonical CBOR codec for stored event payloads.
//!
//! The canonical form is fixed by codec tests: definite lengths only,
//! smallest-width integer heads, map keys sorted bytewise ascending over
//! their encoded form, duplicate keys rejected, floats / tags / indefinite
//! items rejected. Decoding verifies canonicity so that any stored payload
//! re-encodes to identical bytes.

use std::fmt;

const MAJOR_UNSIGNED: u8 = 0;
const MAJOR_NEGATIVE: u8 = 1;
const MAJOR_BYTES: u8 = 2;
const MAJOR_TEXT: u8 = 3;
const MAJOR_ARRAY: u8 = 4;
const MAJOR_MAP: u8 = 5;
const MAJOR_TAG: u8 = 6;
const MAJOR_SIMPLE: u8 = 7;

const MAX_NESTING_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalCborError {
    DuplicateMapKey,
    UnsortedMapKeys,
    FloatNotAllowed,
    TagNotAllowed,
    IndefiniteLengthNotAllowed,
    NonMinimalInteger,
    InvalidUtf8,
    UnexpectedEnd,
    TrailingBytes,
    UnsupportedItem,
    DepthExceeded,
}

impl fmt::Display for CanonicalCborError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateMapKey => write!(f, "duplicate map key"),
            Self::UnsortedMapKeys => write!(f, "map keys are not in canonical order"),
            Self::FloatNotAllowed => write!(f, "floating point values are not allowed"),
            Self::TagNotAllowed => write!(f, "tags are not allowed"),
            Self::IndefiniteLengthNotAllowed => write!(f, "indefinite lengths are not allowed"),
            Self::NonMinimalInteger => write!(f, "integer head is not minimal width"),
            Self::InvalidUtf8 => write!(f, "text is not valid UTF-8"),
            Self::UnexpectedEnd => write!(f, "unexpected end of input"),
            Self::TrailingBytes => write!(f, "trailing bytes after value"),
            Self::UnsupportedItem => write!(f, "unsupported CBOR item"),
            Self::DepthExceeded => write!(f, "nesting depth exceeded"),
        }
    }
}

impl std::error::Error for CanonicalCborError {}

/// CBOR value restricted to the canonical subset. No floats, no tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CborValue {
    Unsigned(u64),
    /// Encodes the negative integer `-1 - n` for the carried `n`.
    Negative(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<CborValue>),
    /// Entries are canonicalized (sorted, duplicate-checked) at encode time.
    Map(Vec<(CborValue, CborValue)>),
    Bool(bool),
    Null,
}

impl CborValue {
    pub fn int(value: i64) -> Self {
        if value >= 0 {
            Self::Unsigned(value as u64)
        } else {
            Self::Negative((-1 - value) as u64)
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Unsigned(n) => i64::try_from(*n).ok(),
            Self::Negative(n) => i64::try_from(*n).ok().and_then(|n| (-1i64).checked_sub(n)),
            _ => None,
        }
    }
}

fn write_head(out: &mut Vec<u8>, major: u8, value: u64) {
    let major = major << 5;
    if value < 24 {
        out.push(major | value as u8);
    } else if value <= u8::MAX as u64 {
        out.push(major | 24);
        out.push(value as u8);
    } else if value <= u16::MAX as u64 {
        out.push(major | 25);
        out.extend_from_slice(&(value as u16).to_be_bytes());
    } else if value <= u32::MAX as u64 {
        out.push(major | 26);
        out.extend_from_slice(&(value as u32).to_be_bytes());
    } else {
        out.push(major | 27);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// Encode a value into canonical CBOR bytes.
pub fn encode_canonical(value: &CborValue) -> Result<Vec<u8>, CanonicalCborError> {
    let mut out = Vec::new();
    encode_into(value, &mut out, 0)?;
    Ok(out)
}

fn encode_into(
    value: &CborValue,
    out: &mut Vec<u8>,
    depth: usize,
) -> Result<(), CanonicalCborError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(CanonicalCborError::DepthExceeded);
    }
    match value {
        CborValue::Unsigned(n) => write_head(out, MAJOR_UNSIGNED, *n),
        CborValue::Negative(n) => write_head(out, MAJOR_NEGATIVE, *n),
        CborValue::Bytes(bytes) => {
            write_head(out, MAJOR_BYTES, bytes.len() as u64);
            out.extend_from_slice(bytes);
        }
        CborValue::Text(text) => {
            write_head(out, MAJOR_TEXT, text.len() as u64);
            out.extend_from_slice(text.as_bytes());
        }
        CborValue::Array(items) => {
            write_head(out, MAJOR_ARRAY, items.len() as u64);
            for item in items {
                encode_into(item, out, depth + 1)?;
            }
        }
        CborValue::Map(entries) => {
            let mut encoded: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(entries.len());
            for (key, entry_value) in entries {
                let mut key_bytes = Vec::new();
                encode_into(key, &mut key_bytes, depth + 1)?;
                let mut value_bytes = Vec::new();
                encode_into(entry_value, &mut value_bytes, depth + 1)?;
                encoded.push((key_bytes, value_bytes));
            }
            encoded.sort_by(|a, b| a.0.cmp(&b.0));
            for window in encoded.windows(2) {
                if window[0].0 == window[1].0 {
                    return Err(CanonicalCborError::DuplicateMapKey);
                }
            }
            write_head(out, MAJOR_MAP, encoded.len() as u64);
            for (key_bytes, value_bytes) in encoded {
                out.extend_from_slice(&key_bytes);
                out.extend_from_slice(&value_bytes);
            }
        }
        CborValue::Bool(false) => out.push((MAJOR_SIMPLE << 5) | 20),
        CborValue::Bool(true) => out.push((MAJOR_SIMPLE << 5) | 21),
        CborValue::Null => out.push((MAJOR_SIMPLE << 5) | 22),
    }
    Ok(())
}

struct Decoder<'a> {
    input: &'a [u8],
    position: usize,
}

impl<'a> Decoder<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], CanonicalCborError> {
        if self.position + count > self.input.len() {
            return Err(CanonicalCborError::UnexpectedEnd);
        }
        let slice = &self.input[self.position..self.position + count];
        self.position += count;
        Ok(slice)
    }

    fn read_head(&mut self) -> Result<(u8, u64), CanonicalCborError> {
        let initial = self.take(1)?[0];
        let major = initial >> 5;
        let info = initial & 0x1f;
        let value = match info {
            0..=23 => info as u64,
            24 => {
                let value = self.take(1)?[0] as u64;
                if value < 24 {
                    return Err(CanonicalCborError::NonMinimalInteger);
                }
                value
            }
            25 => {
                let bytes = self.take(2)?;
                let value = u16::from_be_bytes([bytes[0], bytes[1]]) as u64;
                if value <= u8::MAX as u64 {
                    return Err(CanonicalCborError::NonMinimalInteger);
                }
                value
            }
            26 => {
                let bytes = self.take(4)?;
                let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as u64;
                if value <= u16::MAX as u64 {
                    return Err(CanonicalCborError::NonMinimalInteger);
                }
                value
            }
            27 => {
                let bytes = self.take(8)?;
                let value = u64::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                if value <= u32::MAX as u64 {
                    return Err(CanonicalCborError::NonMinimalInteger);
                }
                value
            }
            31 => return Err(CanonicalCborError::IndefiniteLengthNotAllowed),
            _ => return Err(CanonicalCborError::UnsupportedItem),
        };
        Ok((major, value))
    }

    fn decode_value(&mut self, depth: usize) -> Result<CborValue, CanonicalCborError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(CanonicalCborError::DepthExceeded);
        }
        let initial = self
            .input
            .get(self.position)
            .copied()
            .ok_or(CanonicalCborError::UnexpectedEnd)?;
        let major = initial >> 5;
        if major == MAJOR_SIMPLE {
            let info = initial & 0x1f;
            self.position += 1;
            return match info {
                20 => Ok(CborValue::Bool(false)),
                21 => Ok(CborValue::Bool(true)),
                22 => Ok(CborValue::Null),
                25..=27 => Err(CanonicalCborError::FloatNotAllowed),
                31 => Err(CanonicalCborError::IndefiniteLengthNotAllowed),
                _ => Err(CanonicalCborError::UnsupportedItem),
            };
        }
        let (major, value) = self.read_head()?;
        match major {
            MAJOR_UNSIGNED => Ok(CborValue::Unsigned(value)),
            MAJOR_NEGATIVE => Ok(CborValue::Negative(value)),
            MAJOR_BYTES => Ok(CborValue::Bytes(self.take(value as usize)?.to_vec())),
            MAJOR_TEXT => {
                let bytes = self.take(value as usize)?;
                let text =
                    std::str::from_utf8(bytes).map_err(|_| CanonicalCborError::InvalidUtf8)?;
                Ok(CborValue::Text(text.to_string()))
            }
            MAJOR_ARRAY => {
                let mut items = Vec::new();
                for _ in 0..value {
                    items.push(self.decode_value(depth + 1)?);
                }
                Ok(CborValue::Array(items))
            }
            MAJOR_MAP => {
                let mut entries = Vec::new();
                let mut previous_key: Option<Vec<u8>> = None;
                for _ in 0..value {
                    let key_start = self.position;
                    let key = self.decode_value(depth + 1)?;
                    let key_bytes = self.input[key_start..self.position].to_vec();
                    if let Some(previous) = &previous_key {
                        match previous.cmp(&key_bytes) {
                            std::cmp::Ordering::Less => {}
                            std::cmp::Ordering::Equal => {
                                return Err(CanonicalCborError::DuplicateMapKey)
                            }
                            std::cmp::Ordering::Greater => {
                                return Err(CanonicalCborError::UnsortedMapKeys)
                            }
                        }
                    }
                    previous_key = Some(key_bytes);
                    let entry_value = self.decode_value(depth + 1)?;
                    entries.push((key, entry_value));
                }
                Ok(CborValue::Map(entries))
            }
            MAJOR_TAG => Err(CanonicalCborError::TagNotAllowed),
            _ => Err(CanonicalCborError::UnsupportedItem),
        }
    }
}

/// Decode canonical CBOR bytes, verifying every canonicity rule.
pub fn decode_canonical(input: &[u8]) -> Result<CborValue, CanonicalCborError> {
    let mut decoder = Decoder { input, position: 0 };
    let value = decoder.decode_value(0)?;
    if decoder.position != input.len() {
        return Err(CanonicalCborError::TrailingBytes);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: Vec<(&str, CborValue)>) -> CborValue {
        CborValue::Map(
            entries
                .into_iter()
                .map(|(k, v)| (CborValue::Text(k.to_string()), v))
                .collect(),
        )
    }

    #[test]
    fn integer_widths_are_minimal_and_fixed() {
        assert_eq!(
            encode_canonical(&CborValue::Unsigned(0)).unwrap(),
            vec![0x00]
        );
        assert_eq!(
            encode_canonical(&CborValue::Unsigned(23)).unwrap(),
            vec![0x17]
        );
        assert_eq!(
            encode_canonical(&CborValue::Unsigned(24)).unwrap(),
            vec![0x18, 24]
        );
        assert_eq!(
            encode_canonical(&CborValue::Unsigned(256)).unwrap(),
            vec![0x19, 0x01, 0x00]
        );
        assert_eq!(encode_canonical(&CborValue::int(-1)).unwrap(), vec![0x20]);
        assert_eq!(
            encode_canonical(&CborValue::int(i64::MAX)).unwrap(),
            vec![0x1b, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
        );
    }

    #[test]
    fn map_keys_are_sorted_bytewise_and_deterministic() {
        let a = map(vec![
            ("b", CborValue::Unsigned(2)),
            ("a", CborValue::Unsigned(1)),
        ]);
        let b = map(vec![
            ("a", CborValue::Unsigned(1)),
            ("b", CborValue::Unsigned(2)),
        ]);
        assert_eq!(encode_canonical(&a).unwrap(), encode_canonical(&b).unwrap());
        // Fixed golden bytes: {"a": 1, "b": 2}
        assert_eq!(
            encode_canonical(&a).unwrap(),
            vec![0xa2, 0x61, b'a', 0x01, 0x61, b'b', 0x02]
        );
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let value = map(vec![
            ("a", CborValue::Unsigned(1)),
            ("a", CborValue::Unsigned(2)),
        ]);
        assert_eq!(
            encode_canonical(&value),
            Err(CanonicalCborError::DuplicateMapKey)
        );
        // 0xa2 {"a":1,"a":2}
        let bytes = vec![0xa2, 0x61, b'a', 0x01, 0x61, b'a', 0x02];
        assert_eq!(
            decode_canonical(&bytes),
            Err(CanonicalCborError::DuplicateMapKey)
        );
    }

    #[test]
    fn floats_tags_and_indefinite_are_rejected() {
        // 0xf9 = half float
        assert_eq!(
            decode_canonical(&[0xf9, 0x00, 0x00]),
            Err(CanonicalCborError::FloatNotAllowed)
        );
        // 0xc0 = tag 0
        assert_eq!(
            decode_canonical(&[0xc0, 0x00]),
            Err(CanonicalCborError::TagNotAllowed)
        );
        // 0x5f = indefinite byte string
        assert_eq!(
            decode_canonical(&[0x5f, 0xff]),
            Err(CanonicalCborError::IndefiniteLengthNotAllowed)
        );
    }

    #[test]
    fn non_minimal_integers_are_rejected_on_decode() {
        // 24 encoded with one-byte argument 10 (should be 0x0a)
        assert_eq!(
            decode_canonical(&[0x18, 0x0a]),
            Err(CanonicalCborError::NonMinimalInteger)
        );
        // 16-bit argument 100 (fits in 8-bit)
        assert_eq!(
            decode_canonical(&[0x19, 0x00, 0x64]),
            Err(CanonicalCborError::NonMinimalInteger)
        );
    }

    #[test]
    fn unsorted_map_keys_are_rejected_on_decode() {
        // {"b":2,"a":1}
        let bytes = vec![0xa2, 0x61, b'b', 0x02, 0x61, b'a', 0x01];
        assert_eq!(
            decode_canonical(&bytes),
            Err(CanonicalCborError::UnsortedMapKeys)
        );
    }

    #[test]
    fn round_trip_is_identity_on_canonical_bytes() {
        let value = map(vec![
            ("id", CborValue::Text("s-1".to_string())),
            ("n", CborValue::int(-42)),
            (
                "flags",
                CborValue::Array(vec![CborValue::Bool(true), CborValue::Null]),
            ),
            ("bytes", CborValue::Bytes(vec![1, 2, 3])),
        ]);
        let encoded = encode_canonical(&value).unwrap();
        let decoded = decode_canonical(&encoded).unwrap();
        assert_eq!(encode_canonical(&decoded).unwrap(), encoded);
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        assert_eq!(
            decode_canonical(&[0x00, 0x00]),
            Err(CanonicalCborError::TrailingBytes)
        );
    }
}
