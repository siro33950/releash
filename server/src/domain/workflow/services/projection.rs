//! Workflow state projection rules.
//!
//! This module keeps presentation-independent workflow state derivation in the
//! domain layer. Infrastructure can map runtime storage types into these value
//! objects, but the rules for derived fields live here.

use crate::domain::workflow::value_objects::{NodeHistoryEntry, TokenUsage};
#[cfg(test)]
use crate::domain::workflow::NODE_STATUS_COMPLETED;

pub fn total_token_usage(node_history: &[NodeHistoryEntry]) -> TokenUsage {
    let mut usage = TokenUsage::default();
    for entry in node_history {
        if let Some(entry_usage) = &entry.token_usage {
            usage.add(entry_usage);
        }
    }
    usage
}

#[cfg(test)]
#[path = "projection_test.rs"]
mod projection_tests;
