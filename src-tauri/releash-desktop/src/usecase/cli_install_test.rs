use super::*;
struct Install(Result<String, String>);
impl CliInstallGateway for Install {
    fn install(&self) -> Result<String, String> {
        self.0.clone()
    }
}
#[tokio::test]
async fn test_cli設置_明示要求の結果と管理者認証失敗を返す() {
    for expected in [
        Ok("installed".to_string()),
        Err("administrator authorization denied".to_string()),
    ] {
        // Given
        let service = CliInstallUsecase(Arc::new(Install(expected.clone())));
        // When
        let result = service.install().await.map_err(|e| e.to_string());
        // Then
        assert_eq!(result, expected);
    }
}
