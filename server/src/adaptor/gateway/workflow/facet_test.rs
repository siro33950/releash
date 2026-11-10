use super::*;
use crate::adaptor::gateway::workflow::workflow_host::prompt_rendering;

#[test]
fn valid_keys() {
    assert!(validate_facet_key("coder").is_ok());
    assert!(validate_facet_key("my-facet").is_ok());
    assert!(validate_facet_key("test_123").is_ok());
    assert!(validate_facet_key("a").is_ok());
    assert!(validate_facet_key("A1-b_c").is_ok());
}

#[test]
fn invalid_keys() {
    assert!(validate_facet_key("").is_err());
    assert!(validate_facet_key("-start").is_err());
    assert!(validate_facet_key("_start").is_err());
    assert!(validate_facet_key("a/b").is_err());
    assert!(validate_facet_key("a b").is_err());
    assert!(validate_facet_key("../evil").is_err());
}

#[test]
fn find_undefined_template_variables_returns_only_invalid_reference_syntax() {
    let content = "{{ goal }} {{ plan.summary }} {{ plan.a.b }} {{ bad ref }}";
    let undefined = prompt_rendering::find_undefined_template_variables(content);
    assert_eq!(undefined, vec!["bad ref".to_string()]);
}
