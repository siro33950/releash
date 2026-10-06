pub(crate) mod tests {
    use super::super::*;

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
        assert_eq!(
            encode_canonical(&CborValue::Negative(0)).unwrap(),
            vec![0x20]
        );
        assert_eq!(
            encode_canonical(&CborValue::Unsigned(i64::MAX as u64)).unwrap(),
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
            ("n", CborValue::Negative(41)),
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
