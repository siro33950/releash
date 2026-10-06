pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn public_filter_parser_owns_the_external_status_vocabulary() {
        assert_eq!(
            ExecutionStatusFilter::from_public_filter(None).unwrap(),
            None
        );
        assert_eq!(
            ExecutionStatusFilter::from_public_filter(Some("")).unwrap(),
            None
        );
        assert_eq!(
            ExecutionStatusFilter::from_public_filter(Some("active")).unwrap(),
            Some(ExecutionStatusFilter::Active)
        );
        assert_eq!(
            ExecutionStatusFilter::from_public_filter(Some("terminal")).unwrap(),
            Some(ExecutionStatusFilter::Terminal)
        );
        assert!(ExecutionStatusFilter::from_public_filter(Some("running")).is_err());
    }
}
