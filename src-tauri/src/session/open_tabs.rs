use parking_lot::RwLock;
use std::collections::HashSet;

#[derive(Default)]
pub struct OpenTabRegistry {
    open_tabs: RwLock<HashSet<String>>,
}

impl OpenTabRegistry {
    pub fn add(&self, chat_session_id: &str) -> bool {
        self.open_tabs.write().insert(chat_session_id.to_string())
    }

    pub fn remove(&self, chat_session_id: &str) -> bool {
        self.open_tabs.write().remove(chat_session_id)
    }

    pub fn contains(&self, chat_session_id: &str) -> bool {
        self.open_tabs.read().contains(chat_session_id)
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> HashSet<String> {
        self.open_tabs.read().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::OpenTabRegistry;

    #[test]
    fn add_and_remove_are_idempotent() {
        let registry = OpenTabRegistry::default();

        registry.add("step-session");
        registry.add("step-session");
        assert!(registry.contains("step-session"));
        assert_eq!(registry.snapshot().len(), 1);

        registry.remove("step-session");
        registry.remove("step-session");
        assert!(!registry.contains("step-session"));
        assert!(registry.snapshot().is_empty());
    }
}
