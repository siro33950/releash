use sentry::ClientInitGuard;
use std::borrow::Cow;

use crate::config::{load_or_create_config, ReleashConfig};

const SENTRY_DSN: &str = env!("SENTRY_DSN");

fn load_config_direct() -> Option<ReleashConfig> {
    let data_dir = dirs::data_dir()?;
    let config_path = data_dir.join("com.releash.app").join("releash.toml");
    load_or_create_config(&config_path).ok()
}

pub fn init_sentry() -> Option<ClientInitGuard> {
    if SENTRY_DSN.is_empty() {
        return None;
    }

    let config = load_config_direct();
    let enabled = config
        .as_ref()
        .map(|c| c.telemetry.crash_reporting)
        .unwrap_or(true);

    let guard = sentry::init(sentry::ClientOptions {
        dsn: SENTRY_DSN.parse().ok(),
        release: Some(Cow::Borrowed(env!("CARGO_PKG_VERSION"))),
        environment: if cfg!(debug_assertions) {
            Some(Cow::Borrowed("development"))
        } else {
            Some(Cow::Borrowed("production"))
        },
        send_default_pii: false,
        auto_session_tracking: true,
        traces_sample_rate: 0.0,
        before_send: Some(std::sync::Arc::new(|mut event| {
            scrub_event(&mut event);
            Some(event)
        })),
        ..Default::default()
    });

    if !enabled {
        sentry::Hub::current().bind_client(None);
    }

    Some(guard)
}

pub fn set_crash_reporting_enabled(enabled: bool) {
    if enabled {
        let hub = sentry::Hub::current();
        if hub.client().is_none() {
            if let Some(main_hub) = sentry::Hub::main().client() {
                hub.bind_client(Some(main_hub));
            }
        }
    } else {
        sentry::Hub::current().bind_client(None);
    }
}

fn scrub_event(event: &mut sentry::protocol::Event) {
    for ex in event.exception.values.iter_mut() {
        if let Some(ref mut stacktrace) = ex.stacktrace {
            for frame in stacktrace.frames.iter_mut() {
                frame.filename = frame.filename.as_ref().map(|f| scrub_home_dir(f));
                frame.abs_path = frame.abs_path.as_ref().map(|f| scrub_home_dir(f));
            }
        }
    }
}

fn scrub_home_dir(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Some(home_str) = home.to_str() {
            return path.replacen(home_str, "~", 1);
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_home_dir_replaces_home_path() {
        if let Some(home) = dirs::home_dir() {
            let home_str = home.to_str().unwrap();
            let input = format!("{home_str}/projects/releash/src/main.rs");
            let result = scrub_home_dir(&input);
            assert_eq!(result, "~/projects/releash/src/main.rs");
        }
    }

    #[test]
    fn scrub_home_dir_leaves_non_home_paths() {
        let input = "/usr/local/bin/releash";
        let result = scrub_home_dir(input);
        assert_eq!(result, "/usr/local/bin/releash");
    }
}
