use super::*;
use crate::domain::app_config::AppConfig;
use crate::domain::app_config::{AppConfigError, AppConfigRepository};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Port(Mutex<Vec<String>>);
impl TelemetryPort for Port {
    fn report_frontend_error(&self, kind: &str, message: &str, stack: Option<&str>) {
        self.0
            .lock()
            .unwrap()
            .push(format!("error:{kind}:{message}:{stack:?}"));
    }
    fn set_mounted_xterm_count(&self, count: u64) {
        self.0.lock().unwrap().push(format!("mounted:{count}"));
    }
    fn record_usage_event(&self, name: &str) {
        self.0.lock().unwrap().push(format!("usage:{name}"));
    }
    fn set_performance_enabled(&self, enabled: bool) {
        self.0
            .lock()
            .unwrap()
            .push(format!("performance:{enabled}"));
    }
    fn set_crash_reporting_enabled(&self, enabled: bool) {
        self.0.lock().unwrap().push(format!("crash:{enabled}"));
    }
}
struct Config {
    document: Mutex<AppConfig>,
    fail: bool,
}
impl AppConfigRepository for Config {
    fn load(&self) -> Result<AppConfig, AppConfigError> {
        Ok(self.document.lock().unwrap().clone())
    }
    fn save(&self, document: AppConfig) -> Result<(), AppConfigError> {
        if self.fail {
            return Err(AppConfigError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: "save failed".into(),
                },
            ));
        }
        *self.document.lock().unwrap() = document;
        Ok(())
    }
}
impl crate::usecase::app_config::query_service::DesktopSettingsQueryService for Config {
    fn desktop_settings(
        &self,
    ) -> Result<crate::usecase::app_config::query_service::DesktopSettingsDto, AppConfigError> {
        let config = self.document.lock().unwrap();
        Ok(
            crate::usecase::app_config::query_service::DesktopSettingsDto {
                start_minimized: config.start_minimized(),
                crash_reporting: config.telemetry().crash_reporting,
                performance_telemetry: config.telemetry().performance_telemetry,
                auto_launch: config.auto_launch(),
            },
        )
    }
}
impl crate::usecase::app_config::query_service::WorkflowConfigQueryService for Config {
    fn get_workflow_config(
        &self,
    ) -> Result<crate::usecase::app_config::query_service::WorkflowConfigDto, AppConfigError> {
        Ok(
            crate::usecase::app_config::query_service::WorkflowConfigDto {
                approval_auto_approve: self
                    .document
                    .lock()
                    .unwrap()
                    .workflow()
                    .approval_auto_approve,
            },
        )
    }
}

#[test]
fn test_telemetry設定_永続化成功時だけruntime設定を更新する() {
    for fail in [false, true] {
        // Given
        let repository = Arc::new(Config {
            document: Mutex::new(AppConfig::new(
                false,
                false,
                Vec::new(),
                String::new(),
                String::new(),
                crate::domain::app_config::value_objects::TelemetryConfig {
                    crash_reporting: false,
                    performance_telemetry: false,
                },
                crate::domain::app_config::value_objects::WorkflowConfig {
                    approval_auto_approve: false,
                },
                Default::default(),
                Default::default(),
            )),
            fail,
        });
        let config = crate::usecase::app_config::AppConfigUsecase::new(
            repository.clone(),
            repository.clone(),
            repository,
            Default::default(),
        );
        let port = Port::default();
        let usecase = TelemetryUsecase::new(&port);
        // When
        let performance = usecase.update_performance_telemetry(&config, true);
        let crash = usecase.update_crash_reporting(&config, true);
        let calls = port.0.lock().unwrap().clone();
        let settings = config.desktop_settings().unwrap();
        let workflow = config.get_workflow_config().unwrap();
        // Then
        assert_eq!(performance.is_err(), fail);
        assert_eq!(crash.is_err(), fail);
        if fail {
            assert!(calls.is_empty());
        } else {
            assert_eq!(calls, ["performance:true", "crash:true"]);
        }
        assert_eq!(settings.performance_telemetry, !fail);
        assert_eq!(settings.crash_reporting, !fail);
        assert!(!workflow.approval_auto_approve);
    }
}

#[test]
fn test_telemetry操作_引数とquery結果をportへ委譲する() {
    // Given
    let port = Port::default();
    let usecase = TelemetryUsecase::new(&port);
    // When
    usecase.report_frontend_error("type", "message", Some("stack"));
    usecase.set_mounted_xterm_count(3);
    usecase.record_usage_event("event");
    // Then
    assert_eq!(
        *port.0.lock().unwrap(),
        [
            "error:type:message:Some(\"stack\")",
            "mounted:3",
            "usage:event"
        ]
    );
}

impl crate::usecase::app_config::query_service::TerminalStartupCommandQueryService for Config {
    fn terminal_startup_command(&self) -> Result<String, AppConfigError> {
        Ok(self
            .document
            .lock()
            .unwrap()
            .terminal_startup_command()
            .to_owned())
    }
}
