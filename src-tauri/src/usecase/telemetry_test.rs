use super::*;
use crate::domain::app_config::repository::ConfigUpdate;
use crate::domain::app_config::value_objects::AppConfigDocument;
use crate::domain::app_config::{AppConfigError, ConfigRepository};
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
    fn terminal_performance_switches(&self) -> TerminalPerformanceSwitches {
        TerminalPerformanceSwitches {
            disable_webgl_renderer: true,
            ..Default::default()
        }
    }
    fn performance_real_app_mode(&self) -> bool {
        true
    }
    fn start_terminal_launch_collection(&self) {
        self.0.lock().unwrap().push("start-launch".into());
    }
    fn take_terminal_launch_samples(&self) -> Vec<TerminalLaunchSample> {
        vec![TerminalLaunchSample {
            phase: "first_paint",
            duration_ms: 42.0,
        }]
    }
    fn start_terminal_input_collection(&self) {
        self.0.lock().unwrap().push("start-input".into());
    }
    fn take_terminal_input_samples(&self) -> Vec<TerminalInputSample> {
        Vec::new()
    }
    fn record_terminal_launch(&self, _phase: TerminalLaunch, duration: Duration) {
        self.0
            .lock()
            .unwrap()
            .push(format!("launch:{}", duration.as_millis()));
    }
}
struct Config {
    document: Mutex<AppConfigDocument>,
    fail: bool,
}
impl ConfigRepository for Config {
    fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
        Ok(self.document.lock().unwrap().clone())
    }
    fn save(&self, document: AppConfigDocument) -> Result<(), AppConfigError> {
        if self.fail {
            return Err(AppConfigError::Repository("save failed".into()));
        }
        *self.document.lock().unwrap() = document;
        Ok(())
    }
    fn update(&self, update: ConfigUpdate) -> Result<(), AppConfigError> {
        let mut document = self.load()?;
        update(&mut document)?;
        self.save(document)
    }
}

#[test]
fn test_telemetry設定_永続化成功時だけruntime設定を更新する() {
    // Given / When / Then
    for fail in [false, true] {
        let repository = Arc::new(Config {
            document: Mutex::new(AppConfigDocument {
                telemetry: crate::domain::app_config::value_objects::TelemetryConfig {
                    crash_reporting: false,
                    performance_telemetry: false,
                },
                app: crate::domain::app_config::value_objects::AppSettings {
                    close_to_tray: false,
                    auto_launch: false,
                    start_minimized: false,
                    last_root_path: String::new(),
                    last_repo_paths: Vec::new(),
                    external_editor: String::new(),
                },
                workflow: crate::domain::app_config::value_objects::WorkflowConfig {
                    approval_auto_approve: false,
                },
            }),
            fail,
        });
        let config = crate::usecase::app_config::AppConfigUsecase::new(repository);
        let port = Port::default();
        let usecase = TelemetryUsecase::new(&port);
        assert_eq!(
            usecase.update_performance_telemetry(&config, true).is_err(),
            fail
        );
        assert_eq!(usecase.update_crash_reporting(&config, true).is_err(), fail);
        if fail {
            assert!(port.0.lock().unwrap().is_empty());
        } else {
            assert_eq!(*port.0.lock().unwrap(), ["performance:true", "crash:true"]);
            assert!(config.get_performance_telemetry_enabled().unwrap());
            assert!(config.get_crash_reporting_enabled().unwrap());
            config.get_app_settings().unwrap();
            config.get_workflow_config().unwrap();
        }
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
    usecase.start_terminal_launch_collection();
    usecase.start_terminal_input_collection();
    usecase.record_terminal_launch(TerminalLaunch::FirstPaint, Duration::from_millis(12));
    // Then
    assert_eq!(
        *port.0.lock().unwrap(),
        [
            "error:type:message:Some(\"stack\")",
            "mounted:3",
            "usage:event",
            "start-launch",
            "start-input",
            "launch:12"
        ]
    );
    assert!(usecase.performance_real_app_mode());
    assert!(
        usecase
            .terminal_performance_switches()
            .disable_webgl_renderer
    );
    assert_eq!(
        usecase.take_terminal_launch_samples(),
        [TerminalLaunchSample {
            phase: "first_paint",
            duration_ms: 42.0
        }]
    );
    assert!(usecase.take_terminal_input_samples().is_empty());
}
