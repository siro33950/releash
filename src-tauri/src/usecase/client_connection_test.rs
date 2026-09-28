use super::*;

struct ChangingConnection(std::sync::Mutex<usize>);
impl ClientConnectionQueryService for ChangingConnection {
    fn read(&self) -> Result<ClientConnectionDto, ClientConnectionError> {
        let mut count = self.0.lock().unwrap();
        *count += 1;
        if *count == 3 {
            return Err(ClientConnectionError("unavailable".into()));
        }
        Ok(ClientConnectionDto {
            url: format!("instance-{count}"),
            token: format!("client-{count}"),
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
    assert_eq!(connection.endpoint().unwrap().token, "client-2");
    assert_eq!(
        connection.endpoint().unwrap_err().to_string(),
        "unavailable"
    );
}
