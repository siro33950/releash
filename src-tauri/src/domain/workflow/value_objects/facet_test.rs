mod facet_tests {
    use super::super::*;

    #[test]
    fn test_facet_kind_dir_nameは既存ディレクトリ名を返す() {
        assert_eq!(FacetKind::Policy.dir_name(), "policies");
        assert_eq!(FacetKind::Instruction.dir_name(), "instructions");
    }

    #[test]
    fn test_facet_key_path要素を拒否する() {
        assert!(FacetKey::new("coding").is_ok());
        assert!(FacetKey::new("../coding").is_err());
        assert!(FacetKey::new("foo/bar").is_err());
    }
}
