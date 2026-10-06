use super::*;

#[test]
fn parses_public_filter_and_origin_vocabulary() {
    assert_eq!(parse_status_filter(None).unwrap(), None);
    assert_eq!(
        parse_status_filter(Some("active")).unwrap(),
        Some(ExecutionStatusFilter::Active)
    );
    assert_eq!(parse_execution_origin(None).unwrap(), ExecutionOrigin::Api);
    assert_eq!(
        parse_execution_origin(Some("cli")).unwrap(),
        ExecutionOrigin::Cli
    );
    assert!(parse_execution_origin(Some("desktop_ui")).is_err());
}

#[test]
fn validates_page_limits_and_applies_defaults() {
    assert_eq!(
        parse_page(None, None).unwrap(),
        WorkflowPageRequest::new(0, DEFAULT_PAGE_LIMIT as usize)
    );
    assert_eq!(
        parse_page(Some(2), Some(3)).unwrap(),
        WorkflowPageRequest::new(3, 2)
    );
    assert!(parse_page(Some(0), None).is_err());
    assert!(parse_page(Some(MAX_PAGE_LIMIT + 1), None).is_err());
}
