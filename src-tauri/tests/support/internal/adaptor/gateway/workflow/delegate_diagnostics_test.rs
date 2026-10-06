use super::*;

#[test]
pub fn test_delegate定義_luaは従来どおりdelegateを拒否する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    for completion in [
        "{ delegate = 'worker' }",
        "{ require = r.completion.approval, delegate = 'worker' }",
        "{ require = r.completion.approval, extra = true }",
    ] {
        let source = format!("local r = require('releash')\nreturn r.workflow{{ name = 'delegate', description = 'test', main = r.command{{ command = 'true', completion = {completion} }} }}");
        // When
        let diagnosis = diagnose_lua_workflow_source(
            "delegate.lua",
            &source,
            directory.path(),
            directory.path(),
            None,
        );
        // Then
        assert!(diagnosis.workflow.is_none());
        assert_eq!(diagnosis.diagnostics.len(), 1);
        assert_eq!(diagnosis.diagnostics[0].code, "WFS002");
        assert_eq!(diagnosis.diagnostics[0].stage, DiagnosticStage::ParseShape);
        assert_eq!(diagnosis.diagnostics[0].severity, Severity::Error);
        assert_eq!(
            diagnosis.diagnostics[0].field.as_deref(),
            Some("completion")
        );
        assert_eq!(
            diagnosis.diagnostics[0].message,
            "completion map only accepts the key 'require'"
        );
    }
}
