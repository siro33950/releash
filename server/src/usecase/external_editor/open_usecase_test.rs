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

#[derive(Default)]
struct Launcher {
    calls: Mutex<Vec<(Vec<std::path::PathBuf>, String, String)>>,
    failure: Option<EditorError>,
}

impl EditorLauncherGateway for Launcher {
    fn open_paths(
        &self,
        paths: &[std::path::PathBuf],
        editor: &str,
        label: &str,
    ) -> Result<(), EditorError> {
        self.calls
            .lock()
            .unwrap()
            .push((paths.into(), editor.into(), label.into()));
        self.failure.clone().map_or(Ok(()), Err)
    }
}

#[test]
fn test_エディタ起動_選択設定と対象を一度の呼出しに渡す() {
    // Given / When / Then
    for editor in ["code", ""] {
        for target in [
            EditorTarget::worktree("/repo").unwrap(),
            EditorTarget::file("/repo", "src/a.rs").unwrap(),
        ] {
            let settings = Settings(Mutex::new(Ok(editor.into())));
            let launcher = Launcher::default();
            open_in_editor(&launcher, &settings, &target).unwrap();
            assert_eq!(
                *launcher.calls.lock().unwrap(),
                vec![(target.editor_paths(), editor.into(), target.label().into())]
            );
        }
    }
}

#[test]
fn test_エディタ起動_設定取得に失敗したら起動せず失敗を返す() {
    // Given
    let failure = AppConfigError::Technical(crate::domain::failure::TechnicalFailure {
        nature: crate::domain::failure::TechnicalFailureNature::Other,
        message: "settings unavailable".into(),
    });
    let settings = Settings(Mutex::new(Err(failure.clone())));
    let launcher = Launcher::default();
    let target = EditorTarget::worktree("/repo").unwrap();
    // When / Then
    assert_eq!(
        open_in_editor(&launcher, &settings, &target),
        Err(EditorError::Settings(failure))
    );
    assert!(launcher.calls.lock().unwrap().is_empty());
}

#[test]
fn test_エディタ起動_起動の失敗をそのまま返す() {
    // Given
    let settings = Settings(Mutex::new(Ok("code".into())));
    let failure = EditorError::Launch("launch failed".into());
    let launcher = Launcher {
        failure: Some(failure.clone()),
        ..Default::default()
    };
    let target = EditorTarget::worktree("/repo").unwrap();
    // When / Then
    assert_eq!(open_in_editor(&launcher, &settings, &target), Err(failure));
    assert_eq!(launcher.calls.lock().unwrap().len(), 1);
}
