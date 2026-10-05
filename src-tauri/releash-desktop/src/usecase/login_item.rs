use crate::domain::login_item::{
    LoginItemPort, LoginItemStatus, LoginPreferencePort, RegistrationChange,
};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct LoginItemError(pub String);

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginItemState {
    pub enabled: bool,
    pub requested: bool,
    pub requires_approval: bool,
    pub reason: Option<String>,
}

pub(crate) struct LoginItemUsecase {
    port: Arc<dyn LoginItemPort>,
    preference: Arc<dyn LoginPreferencePort>,
    saving: tokio::sync::Mutex<()>,
    registration_error: parking_lot::Mutex<Option<String>>,
}
impl LoginItemUsecase {
    pub fn new(port: Arc<dyn LoginItemPort>, preference: Arc<dyn LoginPreferencePort>) -> Self {
        Self {
            port,
            preference,
            saving: tokio::sync::Mutex::new(()),
            registration_error: parking_lot::Mutex::new(None),
        }
    }
    pub async fn status(&self) -> Result<LoginItemState, LoginItemError> {
        let status = self.port.status().map_err(LoginItemError)?;
        Ok(LoginItemState {
            enabled: status.enabled(),
            requested: self.preference.load().await.map_err(LoginItemError)?,
            requires_approval: status == LoginItemStatus::RequiresApproval,
            reason: self.registration_error.lock().clone(),
        })
    }
    pub fn restore(&self, requested: bool) -> Result<(), LoginItemError> {
        if self
            .port
            .status()
            .map_err(LoginItemError)?
            .needs_registration(requested)
        {
            self.change_registration(true)?;
        }
        Ok(())
    }
    pub async fn set_enabled(&self, enabled: bool) -> Result<LoginItemState, LoginItemError> {
        let _guard = self.saving.lock().await;
        self.preference.load().await.map_err(LoginItemError)?;
        self.change_registration(enabled)?;
        let requested = self
            .port
            .status()
            .map_err(LoginItemError)?
            .requested_after_change(enabled)
            .map_err(LoginItemError)?;
        self.preference
            .save(requested)
            .await
            .map_err(LoginItemError)?;
        self.status().await
    }
    fn change_registration(&self, enabled: bool) -> Result<(), LoginItemError> {
        let status = self.port.status().map_err(LoginItemError)?;
        let result = match status.registration_change(enabled) {
            RegistrationChange::Register => self
                .port
                .location()
                .and_then(|location| location.ensure_registration_allowed())
                .and_then(|()| self.port.register()),
            RegistrationChange::Unregister => self.port.unregister(),
            RegistrationChange::None => Ok(()),
        };
        *self.registration_error.lock() = result.as_ref().err().cloned();
        result.map_err(LoginItemError)?;
        Ok(())
    }
    pub fn open_settings(&self) -> Result<(), LoginItemError> {
        self.port.open_settings().map_err(LoginItemError)
    }
}

#[cfg(test)]
#[path = "login_item_test.rs"]
mod login_item_tests;
