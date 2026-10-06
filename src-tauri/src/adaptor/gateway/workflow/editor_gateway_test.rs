use super::*;

#[test]
pub fn parse_editor_facet_kind_accepts_wire_and_directory_names() {
    assert_eq!(
        parse_editor_facet_kind("policy").unwrap(),
        facet::FacetKind::Policy
    );
    assert_eq!(
        parse_editor_facet_kind("policies").unwrap(),
        facet::FacetKind::Policy
    );
}
