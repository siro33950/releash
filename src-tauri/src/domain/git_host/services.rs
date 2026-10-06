pub fn issue_branch_name(number: u64) -> String {
    format!("feat/issues/{number}")
}

#[cfg(test)]
#[path = "services_test.rs"]
pub(crate) mod services_tests;
