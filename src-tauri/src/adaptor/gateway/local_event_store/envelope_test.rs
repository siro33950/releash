pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn unknown_type_and_version_are_preserved_raw() {
        let registry = EventCodecRegistry::new();
        let payload = encode_canonical(&CborValue::Map(vec![])).unwrap();
        assert_eq!(
            registry.decode("future.event", 1, &payload).unwrap(),
            DecodedStoredEvent::Unknown
        );
        assert_eq!(
            registry
                .decode("application.lifecycle", 999, &payload)
                .unwrap(),
            DecodedStoredEvent::Unknown
        );
    }
}
