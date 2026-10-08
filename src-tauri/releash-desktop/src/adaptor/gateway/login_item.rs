use crate::domain::login_item::{LoginItemPort, LoginItemStatus};
use crate::infrastructure::platform::login_item;

pub(crate) struct MacLoginItem(pub std::sync::Arc<super::daemon_connection::DaemonServiceGateway>);
#[async_trait::async_trait]
impl LoginItemPort for MacLoginItem {
    fn status(&self) -> Result<LoginItemStatus, String> {
        decode_status(login_item::status()?)
    }
    async fn ensure_registration_allowed(&self) -> Result<(), String> {
        use releashd::desktop_api::wire;
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let executable_path = executable
            .to_str()
            .ok_or("Desktop executable path is not UTF-8")?
            .into();
        let result = self
            .0
            .client()?
            .request(wire::command_request::Command::CheckLoginRegistration(
                wire::CheckLoginRegistrationRequest { executable_path },
            ))
            .await?;
        match result {
            wire::command_result::Command::CheckLoginRegistration(result) => {
                registration_allowed(result)
            }
            _ => Err("Unexpected login registration result".into()),
        }
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
    pub std::sync::Arc<super::daemon_connection::DaemonServiceGateway>,
);
impl DaemonLoginPreference {
    async fn request(
        &self,
        command: releashd::desktop_api::wire::command_request::Command,
    ) -> Result<releashd::desktop_api::wire::command_result::Command, String> {
        self.0.client()?.request(command).await
    }
}
#[async_trait::async_trait]
impl crate::domain::login_item::LoginPreferencePort for DaemonLoginPreference {
    async fn load(&self) -> Result<bool, String> {
        self.0
            .client()?
            .current_settings()
            .map(|settings| settings.auto_launch)
            .ok_or_else(|| "Missing login preference".into())
    }

    async fn save(&self, requested: bool) -> Result<(), String> {
        use releashd::desktop_api::wire;
        match self.request(preference_request(requested)).await? {
            wire::command_result::Command::UpdateLoginItemPreference(_) => Ok(()),
            _ => Err("Unexpected login preference update result".into()),
        }
    }
}

fn preference_request(requested: bool) -> releashd::desktop_api::wire::command_request::Command {
    use releashd::desktop_api::wire;
    wire::command_request::Command::UpdateLoginItemPreference(
        wire::UpdateLoginItemPreferenceRequest {
            requested: Some(requested),
        },
    )
}

fn registration_allowed(
    result: releashd::desktop_api::wire::LoginRegistrationResult,
) -> Result<(), String> {
    use releashd::desktop_api::wire::LoginRegistrationStatus as S;
    match S::try_from(result.status) {
        Ok(S::Allowed) => Ok(()),
        Ok(S::Translocated | S::ReadOnly) => Err(result.reason),
        _ => Err("Unknown login registration status".into()),
    }
}
