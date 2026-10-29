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
impl crate::usecase::app_config::query_service::WorkflowConfigQueryService for Config {
    fn get_workflow_config(
        &self,
    ) -> Result<crate::usecase::app_config::query_service::WorkflowConfigDto, AppConfigError> {
        Ok(
            crate::usecase::app_config::query_service::WorkflowConfigDto {
                approval_auto_approve: self.document.lock().unwrap().workflow().approval_auto_approve,
            },
        )
    }
}

#[test]
fn test_telemetry設定_永続化成功時だけruntime設定を更新する() {
    todo!()
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
            .terminal_startup_command().to_owned())
    }
}
