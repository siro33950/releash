use super::*;
use crate::domain::app_config::value_objects::{TelemetryConfig, WorkflowConfig};
use crate::domain::app_config::{AppConfig, AppConfigError};
use std::sync::Mutex;

struct Settings(Mutex<Result<String, AppConfigError>>);
impl EditorSettingsGateway for Settings {
    fn selected_editor(&self) -> Result<String, EditorError> {
        self.0
            .lock()
            .unwrap()
            .clone()
            .map_err(EditorError::Settings)
    }
}

impl AppConfigRepository for Settings {
    fn load(&self) -> Result<AppConfig, AppConfigError> {
        Ok(AppConfig::new(
            false,
            false,
            vec![],
            self.0.lock().unwrap().clone()?,
            String::new(),
            TelemetryConfig {
                crash_reporting: false,
                performance_telemetry: false,
            },
            WorkflowConfig {
                approval_auto_approve: false,
            },
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        ))
    }

    fn save(&self, config: AppConfig) -> Result<(), AppConfigError> {
        let mut stored = self.0.lock().unwrap();
        stored.as_ref().map_err(Clone::clone)?;
        *stored = Ok(config.external_editor().to_owned());
        Ok(())
    }
}

#[test]
fn test_editor設定_読書きとgatewayエラーをそのまま返す() {
    // Given
    let settings = Settings(Mutex::new(Ok("code".into())));
    // When / Then
    assert_eq!(get_external_editor(&settings), Ok("code".into()));
    update_external_editor(&settings, None, "zed".into(), &parking_lot::Mutex::new(())).unwrap();
    assert_eq!(get_external_editor(&settings), Ok("zed".into()));
    *settings.0.lock().unwrap() = Err(AppConfigError::Technical(
        crate::domain::failure::TechnicalFailure {
            nature: crate::domain::failure::TechnicalFailureNature::Other,
            message: "unavailable".into(),
        },
    ));
    assert_eq!(
        get_external_editor(&settings),
        Err(EditorError::Settings(
            crate::domain::app_config::AppConfigError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "unavailable".into(),
                }
            )
        ))
    );
    assert_eq!(
        update_external_editor(&settings, None, "code".into(), &parking_lot::Mutex::new(())),
        Err(EditorError::Settings(
            crate::domain::app_config::AppConfigError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "unavailable".into(),
                }
            )
        ))
    );
}
