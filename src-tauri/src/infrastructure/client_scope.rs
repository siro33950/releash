pub fn valid_scopes(values: &[i32]) -> bool {
    !values.is_empty()
        && values.iter().all(|value| matches!(value, 1 | 2))
        && values
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == values.len()
}

#[cfg(test)]
#[path = "client_scope_test.rs"]
mod client_scope_tests;
