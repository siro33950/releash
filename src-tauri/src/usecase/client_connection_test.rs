use super::*;

struct ChangingConnection(std::sync::Mutex<usize>);
#[async_trait::async_trait]
impl ClientConnectionQueryService for ChangingConnection {
    #[cfg(feature = "desktop")]
    async fn desktop_settings(
        &self,
    ) -> Result<super::super::app_config::query_service::DesktopSettingsDto, ClientConnectionError>
    {
        Err(ClientConnectionError("settings unavailable".into()))
    }

    fn read(&self) -> Result<ClientConnectionDto, ClientConnectionError> {
        let mut count = self.0.lock().unwrap();
        *count += 1;
        if *count == 3 {
            return Err(ClientConnectionError("unavailable".into()));
        }
        Ok(ClientConnectionDto {
            url: format!("instance-{count}"),
            auth_subprotocol: format!("client-{count}"),
        })
    }
}

#[test]
fn test_再接続_毎回接続情報を取得し失敗を返す() {
    // Given
    let connection =
        ClientConnectionUsecase(Box::new(ChangingConnection(std::sync::Mutex::new(0))));
    // When / Then
    assert_eq!(connection.endpoint().unwrap().url, "instance-1");
    assert_eq!(connection.endpoint().unwrap().auth_subprotocol, "client-2");
    assert_eq!(
        connection.endpoint().unwrap_err().to_string(),
        "unavailable"
    );
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn test_desktop設定_読取エラーを返す() {
    let connection =
        ClientConnectionUsecase(Box::new(ChangingConnection(std::sync::Mutex::new(0))));
    assert_eq!(
        connection.desktop_settings().await.unwrap_err().to_string(),
        "settings unavailable"
    );
}
