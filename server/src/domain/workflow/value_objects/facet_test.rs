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

#[test]
fn test_種類の解釈_正規名だけを受け付ける() {
    use super::*;
    use std::str::FromStr;
    // Given / When / Then
    for (name, kind) in [
        ("policy", FacetKind::Policy),
        ("knowledge", FacetKind::Knowledge),
        ("instruction", FacetKind::Instruction),
    ] {
        assert_eq!(FacetKind::from_str(name), Ok(kind));
        assert_eq!(kind.canonical_name(), name);
    }
    for name in [
        "policies",
        "instructions",
        "",
        "Policy",
        " policy",
        "unknown",
    ] {
        assert!(
            matches!(FacetKind::from_str(name), Err(WorkflowError::Validation(_))),
            "{name}"
        );
    }
}

#[test]
fn test_識別子_先頭と使用文字の境界を検証する() {
    use super::*;
    // Given / When / Then
    for key in ["a", "Z", "0", "a-Z_09"] {
        assert_eq!(FacetKey::new(key).unwrap().as_str(), key);
    }
    for key in [
        "", "-a", "_a", "é", "aé", "a.b", "a/b", "a\\b", "a b", "a\n",
    ] {
        assert!(
            matches!(FacetKey::new(key), Err(WorkflowError::Validation(_))),
            "{key:?}"
        );
    }
}
