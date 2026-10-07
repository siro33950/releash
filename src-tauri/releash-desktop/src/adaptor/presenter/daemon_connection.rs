use crate::domain::daemon_connection::DaemonConnectionFailure;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopConnectionFailure {
    pub message: String,
    pub server_older: bool,
    pub client_older: bool,
}
pub fn failure(state: DaemonConnectionFailure) -> DesktopConnectionFailure {
    let mut server_older = false;
    let mut client_older = false;
    let message = match state {
        DaemonConnectionFailure::NotRunning => "サーバは動いていません".into(),
        DaemonConnectionFailure::Incompatible {
            server_older: older,
            server_release,
            client_release,
        } => {
            server_older = older;
            client_older = !older;
            format!(
                "{}（サーバ {}, 画面 {}）",
                if older {
                    "サーバが古い"
                } else {
                    "画面が古い"
                },
                server_release,
                client_release
            )
        }
        DaemonConnectionFailure::StartupFailed { status, stderr } => match status {
            Some(status) => format!("サーバのプロセスが終了しました ({status})\n{stderr}"),
            None => format!("サーバの起動を確認できませんでした\n{stderr}"),
        },
        DaemonConnectionFailure::InitialSettingsUnavailable { detail } => match detail {
            Some(detail) => format!("サーバの初回設定を受信できませんでした\n{detail}"),
            None => "サーバの初回設定を受信できませんでした".into(),
        },
        DaemonConnectionFailure::TechnicalFailure(message) => message,
    };
    DesktopConnectionFailure {
        message,
        server_older,
        client_older,
    }
}
pub fn message(state: DaemonConnectionFailure) -> String {
    failure(state).message
}
#[cfg(test)]
#[path = "daemon_connection_test.rs"]
mod daemon_connection_tests;
