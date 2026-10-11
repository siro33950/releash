use std::collections::HashMap;

use crate::domain::workflow::ExecutionParentRef;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct AttemptScope<'a> {
    pub execution: &'a str,
    pub name: &'a str,
    pub parent: Option<&'a ExecutionParentRef>,
}

pub(super) struct LatestAttempts<'a>(HashMap<AttemptScope<'a>, u32>);

impl<'a> LatestAttempts<'a> {
    pub fn new(attempts: impl IntoIterator<Item = (AttemptScope<'a>, u32)>) -> Self {
        let mut latest = HashMap::<AttemptScope<'a>, u32>::new();
        for (scope, attempt) in attempts {
            latest
                .entry(scope)
                .and_modify(|value| *value = (*value).max(attempt))
                .or_insert(attempt);
        }
        Self(latest)
    }

    pub fn contains(&self, scope: AttemptScope<'_>, attempt: u32) -> bool {
        self.0.get(&scope).is_some_and(|latest| *latest == attempt)
    }
}

#[cfg(test)]
#[path = "latest_attempts_test.rs"]
mod latest_attempts_tests;
