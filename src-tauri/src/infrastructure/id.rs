pub(crate) fn unique_simple_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod id_tests {
    use super::*;

    #[test]
    fn test_id生成_uuid_v4のsimple形式32桁hexを返す() {
        let id = unique_simple_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|char| char.is_ascii_hexdigit()));
        assert_ne!(id, unique_simple_id());
    }
}
