use crate::domain::login_item::{LoginItemPort, LoginItemStatus, RegistrationLocation};
use crate::infrastructure::platform::login_item;

pub(crate) struct MacLoginItem;
impl LoginItemPort for MacLoginItem {
    fn status(&self) -> Result<LoginItemStatus, String> {
        decode_status(login_item::status()?)
    }
    fn location(&self) -> Result<RegistrationLocation, String> {
        let (translocated, read_only) = login_item::registration_location(
            &std::env::current_exe().map_err(|e| e.to_string())?,
        )?;
        Ok(RegistrationLocation {
            translocated,
            read_only,
        })
    }
    fn register(&self) -> Result<(), String> {
        registration_result(login_item::set_registered(true), || self.status())
    }
    fn unregister(&self) -> Result<(), String> {
        login_item::set_registered(false)
    }
    fn open_settings(&self) -> Result<(), String> {
        login_item::open_settings()
    }
}
fn decode_status(status: isize) -> Result<LoginItemStatus, String> {
    match status {
        0 => Ok(LoginItemStatus::NotRegistered),
        1 => Ok(LoginItemStatus::Enabled),
        2 => Ok(LoginItemStatus::RequiresApproval),
        3 => Ok(LoginItemStatus::NotFound),
        _ => Err(format!("Unknown login item status: {status}")),
    }
}

#[cfg(test)]
#[path = "login_item_test.rs"]
mod login_item_tests;

fn registration_result(
    result: Result<(), String>,
    status: impl FnOnce() -> Result<LoginItemStatus, String>,
) -> Result<(), String> {
    match result {
        Ok(()) => Ok(()),
        Err(error) => match status()? {
            LoginItemStatus::RequiresApproval => Ok(()),
            _ => Err(error),
        },
    }
}

pub(crate) struct DaemonLoginPreference(
    pub std::sync::Arc<super::daemon_supervision::DaemonProcessGateway>,
);
impl DaemonLoginPreference {
    async fn request(
        &self,
        command: crate::adaptor::protocol::client::command_request::Command,
    ) -> Result<crate::adaptor::protocol::client::command_result::Command, String> {
        self.0.client()?.request(command).await
    }
    async fn settings(&self) -> Result<bool, String> {
        use crate::adaptor::protocol::client as wire;
        preference_result(
            self.request(wire::command_request::Command::GetAppSettings(
                wire::GetAppSettingsRequest {},
            ))
            .await?,
        )
    }
}
#[async_trait::async_trait]
impl crate::domain::login_item::LoginPreferencePort for DaemonLoginPreference {
    async fn load(&self) -> Result<bool, String> {
        self.settings().await
    }

    async fn save(&self, requested: bool) -> Result<(), String> {
        use crate::adaptor::protocol::client as wire;
        match self.request(preference_request(requested)).await? {
            wire::command_result::Command::UpdateLoginItemPreference(_) => Ok(()),
            _ => Err("Unexpected login preference update result".into()),
        }
    }
}

fn preference_result(
    result: crate::adaptor::protocol::client::command_result::Command,
) -> Result<bool, String> {
    use crate::adaptor::protocol::client as wire;
    match result {
        wire::command_result::Command::GetAppSettings(settings) => settings
            .auto_launch
            .ok_or_else(|| "Missing login preference".into()),
        _ => Err("Unexpected login preference result".into()),
    }
}

fn preference_request(
    requested: bool,
) -> crate::adaptor::protocol::client::command_request::Command {
    use crate::adaptor::protocol::client as wire;
    wire::command_request::Command::UpdateLoginItemPreference(
        wire::UpdateLoginItemPreferenceRequest {
            requested: Some(requested),
        },
    )
}
