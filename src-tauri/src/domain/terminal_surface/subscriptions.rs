use std::collections::HashMap;

pub(crate) const TERMINAL_SUBSCRIPTION_LIMIT: usize = 16;
pub(crate) const TERMINAL_ATTACHMENT_LIMIT: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalSubscriptionError {
    InvalidId,
    AlreadyExists,
    Limit,
    Ended,
    PendingLimit,
    AttachmentLimit,
}

impl std::fmt::Display for TerminalSubscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidId => "Missing terminal subscription id",
            Self::AlreadyExists => "Terminal subscription already exists",
            Self::Limit => "Too many terminal subscriptions",
            Self::Ended => "Terminal subscription ended",
            Self::PendingLimit => "Too many pending terminal attachments",
            Self::AttachmentLimit => "Too many terminal attachments",
        })
    }
}
impl std::error::Error for TerminalSubscriptionError {}

pub(crate) struct TerminalSubscriptions<T> {
    subscriptions: HashMap<String, T>,
    attachments: usize,
}
impl<T> Default for TerminalSubscriptions<T> {
    fn default() -> Self {
        Self {
            subscriptions: HashMap::new(),
            attachments: 0,
        }
    }
}
impl<T> TerminalSubscriptions<T> {
    pub fn subscribe(
        &mut self,
        id: String,
        subscription: T,
    ) -> Result<(), TerminalSubscriptionError> {
        if id.trim().is_empty() {
            return Err(TerminalSubscriptionError::InvalidId);
        }
        if self.subscriptions.contains_key(&id) {
            return Err(TerminalSubscriptionError::AlreadyExists);
        }
        if self.subscriptions.len() >= TERMINAL_SUBSCRIPTION_LIMIT {
            return Err(TerminalSubscriptionError::Limit);
        }
        self.subscriptions.insert(id, subscription);
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<&T, TerminalSubscriptionError> {
        self.subscriptions
            .get(id)
            .ok_or(TerminalSubscriptionError::Ended)
    }
    pub fn unsubscribe(&mut self, id: &str) {
        self.subscriptions.remove(id);
    }
    pub fn reserve_attachment(&mut self) -> Result<(), TerminalSubscriptionError> {
        if self.attachments >= TERMINAL_ATTACHMENT_LIMIT {
            return Err(TerminalSubscriptionError::AttachmentLimit);
        }
        self.attachments += 1;
        Ok(())
    }
    pub fn release_attachment(&mut self) {
        self.attachments -= 1;
    }
}

#[cfg(test)]
#[path = "subscriptions_test.rs"]
mod subscriptions_tests;
