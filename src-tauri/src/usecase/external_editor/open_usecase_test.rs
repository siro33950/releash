use super::*;
use std::sync::Mutex;

struct Settings(Mutex<Result<String, String>>);
impl EditorSettingsGateway for Settings {
    fn selected_editor(&self) -> Result<String, String> {
        self.0.lock().unwrap().clone()
    }
    fn update_selected_editor(&self, editor: String) -> Result<(), String> {
        let mut current = self.0.lock().unwrap();
        current.as_ref().map_err(Clone::clone)?;
        *current = Ok(editor);
        Ok(())
    }
}

#[test]
fn test_editor設定_読書きとgatewayエラーをそのまま返す() {
    // Given
    let settings = Settings(Mutex::new(Ok("code".into())));
    // When / Then
    assert_eq!(get_external_editor(&settings), Ok("code".into()));
    update_external_editor(&settings, "zed".into()).unwrap();
    assert_eq!(get_external_editor(&settings), Ok("zed".into()));
    *settings.0.lock().unwrap() = Err("unavailable".into());
    assert_eq!(get_external_editor(&settings), Err("unavailable".into()));
    assert_eq!(
        update_external_editor(&settings, "code".into()),
        Err("unavailable".into())
    );
}
