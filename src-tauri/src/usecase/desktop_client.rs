use super::client_handoff::ClientHandoffUsecase;
use crate::domain::client_operation::{
    handoff::ClientHandoffReference, registry::OperationIdentity, transmission::ClientTransmission,
};
use std::sync::Arc;

pub(crate) struct DesktopClientUsecase {
    transmission: ClientTransmission,
    handoff: Arc<ClientHandoffUsecase>,
}
impl DesktopClientUsecase {
    pub fn new(handoff: Arc<ClientHandoffUsecase>) -> Self {
        Self {
            transmission: ClientTransmission::default(),
            handoff,
        }
    }
    pub fn transmit(
        &mut self,
        id: &str,
        identity: OperationIdentity,
        deadline: u64,
        now: u64,
    ) -> Result<(), String> {
        self.transmission.admit(id, &identity, deadline, now)?;
        self.handoff
            .remember(ClientHandoffReference {
                id: id.into(),
                command: identity.command.clone(),
                fingerprint: identity.fingerprint.to_vec(),
                ordering_target: identity
                    .target
                    .map_or_else(Vec::new, |(_, target)| target.to_vec()),
            })
            .map_err(|e| e.to_string())?;
        self.transmission.sent(id.into(), identity);
        Ok(())
    }
    pub fn query(
        &mut self,
        id: &str,
        identity: OperationIdentity,
        sent: bool,
        previous: &str,
        current: &str,
    ) -> Result<bool, String> {
        self.transmission
            .queried(id, identity, sent, previous, current)
            .map_err(Into::into)
    }
    pub fn acknowledge(&mut self, id: &str) -> Result<(), String> {
        if self.transmission.needs_handoff(id) {
            self.handoff.forget(id).map_err(|e| e.to_string())?;
        }
        self.transmission.acknowledged(id);
        Ok(())
    }
    pub fn responded(&mut self, id: &str, succeeded: bool) {
        self.transmission.responded(id, succeeded);
    }
    pub fn finish_restoration(&mut self) -> Result<(), String> {
        self.transmission.finish_restoration().map_err(Into::into)
    }
    pub fn unknown_after_restart(&mut self, id: &str) -> bool {
        self.transmission.unknown_after_restart(id)
    }
}

#[cfg(test)]
#[path = "desktop_client_test.rs"]
mod desktop_client_tests;
