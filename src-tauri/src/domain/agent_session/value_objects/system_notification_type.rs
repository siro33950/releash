#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemNotificationType {
    Compaction,
}

impl SystemNotificationType {
    #[allow(dead_code)] // issues-1301 F-6: string form is retained for system-notification presentation/backward-compatible event payloads.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compaction => "compaction",
        }
    }
}
