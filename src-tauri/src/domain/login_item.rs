#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoginItemStatus {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
}

impl LoginItemStatus {
    pub fn enabled(self) -> bool {
        self == Self::Enabled
    }
    pub fn needs_registration(self, requested: bool) -> bool {
        requested && matches!(self, Self::NotRegistered | Self::NotFound)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RegistrationChange {
    Register,
    Unregister,
    None,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RegistrationLocation {
    pub translocated: bool,
    pub read_only: bool,
}
impl RegistrationLocation {
    pub fn ensure_registration_allowed(self) -> Result<(), String> {
        if self.translocated {
            return Err("Move Releash.app to Applications before enabling Launch at login.".into());
        }
        if self.read_only {
            return Err("Releash.app is on a read-only volume. Move it to Applications before enabling Launch at login.".into());
        }
        Ok(())
    }
}
impl LoginItemStatus {
    pub fn registration_change(self, requested: bool) -> RegistrationChange {
        if self.needs_registration(requested) {
            RegistrationChange::Register
        } else if !requested && matches!(self, Self::Enabled | Self::RequiresApproval) {
            RegistrationChange::Unregister
        } else {
            RegistrationChange::None
        }
    }
    pub fn requested_after_change(self, requested: bool) -> Result<bool, String> {
        if requested && matches!(self, Self::Enabled | Self::RequiresApproval) {
            Ok(true)
        } else if !requested && matches!(self, Self::NotRegistered | Self::NotFound) {
            Ok(false)
        } else {
            Err("Login item registration did not reach the requested state.".into())
        }
    }
}
#[async_trait::async_trait]
pub(crate) trait LoginPreferencePort: Send + Sync {
    async fn load(&self) -> Result<bool, String>;
    async fn save(&self, requested: bool) -> Result<(), String>;
}

pub(crate) trait LoginItemPort: Send + Sync {
    fn status(&self) -> Result<LoginItemStatus, String>;
    fn location(&self) -> Result<RegistrationLocation, String>;
    fn register(&self) -> Result<(), String>;
    fn unregister(&self) -> Result<(), String>;
    fn open_settings(&self) -> Result<(), String>;
}

#[cfg(test)]
#[path = "login_item_test.rs"]
mod login_item_tests;
