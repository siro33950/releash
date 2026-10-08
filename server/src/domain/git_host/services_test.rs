pub(crate) mod tests {
    use super::super::*;

    #[test]
    fn issue_branch_name_uses_existing_template() {
        assert_eq!(issue_branch_name(1302), "feat/issues/1302");
    }
}
