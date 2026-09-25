use super::value_objects::WorkspaceTreeNode;

impl WorkspaceTreeNode {
    pub(super) fn observe_background_failure(&mut self, message: &str) {
        self.error_reason = Some(message.into());
        self.status_classification = Self::classify_own_status(
            self.kind,
            self.status,
            self.activity,
            self.session_id.is_some(),
            self.process_presence,
            true,
        );
    }
}
