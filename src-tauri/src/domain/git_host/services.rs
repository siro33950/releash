pub fn issue_branch_name(number: u64) -> String {
    format!("feat/issues/{number}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_branch_name_uses_existing_template() {
        assert_eq!(issue_branch_name(1302), "feat/issues/1302");
    }
}
