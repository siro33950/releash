use crate::domain::workflow::WorktreeInventoryEntry;

pub fn classify_branch_cards<T: Clone>(
    repository_root: &str,
    cards: &mut Vec<T>,
    identity: impl Fn(&T) -> (&str, Option<&str>),
) -> Vec<T> {
    cards.retain(|card| {
        let (name, path) = identity(card);
        !path.is_some_and(|path| {
            WorktreeInventoryEntry::new(repository_root, path, name)
                .matches_isolated_identity_rule()
        })
    });
    cards
        .iter()
        .filter(|card| identity(card).1.is_some())
        .cloned()
        .collect()
}

#[cfg(test)]
#[path = "branch_inventory_test.rs"]
mod branch_inventory_tests;
