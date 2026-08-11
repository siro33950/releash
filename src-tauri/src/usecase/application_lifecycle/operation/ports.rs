pub trait RecoveryResultCanonicalizer: Send + Sync {
    fn canonicalize_recovery_result(
        &self,
        _outcome: crate::domain::local_event::RecoveryResultOutcomeRecord,
        _classification: crate::domain::local_event::RecoveryResultClassification,
        _resource_revision: u64,
        _resource_view: crate::domain::local_event::RecoveryResourceViewRecord,
    ) -> Result<crate::domain::local_event::RecoveryResultRecord, ()> {
        Err(())
    }
}

pub trait OperationBindingAuthority: RecoveryResultCanonicalizer + Send + Sync {
    fn mac(&self, message: &[u8]) -> [u8; 32];

    fn digest(&self, message: &[u8]) -> [u8; 32];

    fn seal_command(&self, context: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, ()>;

    fn open_command(&self, context: &[u8], envelope: &[u8]) -> Result<Vec<u8>, ()>;
}
