pub(crate) mod tests {

    use releashd::test_support::integration::platform::evaluate;
    use releashd::test_support::integration::platform::LuaData;
    use releashd::test_support::integration::platform::LuaEvaluationRequest;
    use releashd::test_support::integration::platform::LuaFailureKind;
    use releashd::test_support::integration::platform::LuaHost;
    use releashd::test_support::integration::platform::LuaHostError;
    use releashd::test_support::integration::platform::LuaHostHandle;
    use releashd::test_support::integration::platform::LuaLimits;
    use releashd::test_support::integration::platform::LuaModule;
    use releashd::test_support::integration::platform::LuaModuleValue;
    use releashd::test_support::integration::platform::LuaSourceLocation;
    use releashd::test_support::integration::platform::MAX_TABLE_DEPTH;
    use releashd::test_support::integration::platform::MAX_TABLE_ELEMENTS;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[derive(Debug, Default)]
    struct TestHost {
        calls: Vec<(u32, LuaSourceLocation)>,
    }

    impl LuaHost for TestHost {
        fn source_loaded(&mut self, _name: &str, _source: &str) {}

        fn module(&self, name: &str) -> Option<LuaModule> {
            (name == "test").then(|| LuaModule {
                members: BTreeMap::from([("value".to_string(), LuaModuleValue::Function(1))]),
            })
        }

        fn call(
            &mut self,
            function: u32,
            arguments: Vec<LuaData>,
            location: LuaSourceLocation,
        ) -> Result<LuaData, LuaHostError> {
            self.calls.push((function, location));
            Ok(arguments.into_iter().next().unwrap_or(LuaData::Nil))
        }

        fn index(
            &mut self,
            _handle: &LuaHostHandle,
            key: &str,
            location: LuaSourceLocation,
        ) -> Result<LuaData, LuaHostError> {
            Err(LuaHostError {
                category: "test".to_string(),
                message: format!("unknown field '{key}'"),
                location: Some(location),
                field: None,
            })
        }
    }

    fn request<'a>(dir: &'a Path, source: &'a str) -> LuaEvaluationRequest<'a> {
        LuaEvaluationRequest {
            source_name: "main.lua",
            source,
            workflows_dir: dir,
            limits: LuaLimits::default(),
        }
    }

    #[test]
    pub fn test_lua評価_許可moduleを呼び出して呼出位置を返す() {
        let dir = TempDir::new().unwrap();

        let result = evaluate(
            request(
                dir.path(),
                "local test = require('test')\nreturn test.value('ok')",
            ),
            TestHost::default(),
        )
        .unwrap();

        assert_eq!(result.value, LuaData::String("ok".to_string()));
        assert_eq!(result.host.calls.len(), 1);
        assert_eq!(result.host.calls[0].1.line, 2);
    }

    #[test]
    pub fn test_lua評価_標準外部ioと動的loadを公開しない() {
        let dir = TempDir::new().unwrap();
        let source = "return { io = io, os = os, package = package, load = load, print = print, pairs = pairs, next = next, collectgarbage = collectgarbage, tostring = tostring, random = math.random }";

        let result = evaluate(request(dir.path(), source), TestHost::default()).unwrap();
        let LuaData::Table(table) = result.value else {
            panic!("table expected");
        };

        assert!(table.entries.is_empty());
    }

    #[test]
    pub fn test_lua評価_循環参照するtableを拒否する() {
        let dir = TempDir::new().unwrap();

        let error = evaluate(
            request(dir.path(), "local t = {}\nt.self = t\nreturn t"),
            TestHost::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Evaluation);
        assert!(error.message.contains("recursive reference"));
    }

    #[test]
    pub fn test_lua評価_table入れ子の上限を超えたら拒否する() {
        let dir = TempDir::new().unwrap();
        let source = format!(
            "local t = {{}}\nfor _ = 1, {} do t = {{ inner = t }} end\nreturn t",
            MAX_TABLE_DEPTH + 1
        );

        let error = evaluate(request(dir.path(), &source), TestHost::default()).unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Evaluation);
        assert!(error.message.contains("nesting exceeded"));
    }

    #[test]
    pub fn test_lua評価_table要素数の上限を超えたら拒否する() {
        let dir = TempDir::new().unwrap();
        let source = format!(
            "local t = {{}}\nfor i = 1, {} do t[i] = i end\nreturn t",
            MAX_TABLE_ELEMENTS + 1
        );

        let error = evaluate(request(dir.path(), &source), TestHost::default()).unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Evaluation);
        assert!(error.message.contains("exceeded the limit"));
    }

    #[test]
    pub fn test_lua評価_命令上限で終了しない定義を打ち切る() {
        let dir = TempDir::new().unwrap();
        let mut request = request(dir.path(), "while true do end");
        request.limits.instructions = 20_000;

        let error = evaluate(request, TestHost::default()).unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Evaluation);
        assert!(error.message.contains("instruction limit"));
    }

    #[test]
    pub fn test_lua評価_メモリ上限で過大な定義だけを打ち切る() {
        let dir = TempDir::new().unwrap();
        let mut oversized = request(dir.path(), "return string.rep('x', 16777216)");
        oversized.limits.memory_bytes = 4 * 1024 * 1024;

        let error = evaluate(oversized, TestHost::default()).unwrap_err();
        let following = evaluate(request(dir.path(), "return true"), TestHost::default()).unwrap();

        assert_eq!(error.kind, LuaFailureKind::Evaluation);
        assert!(error.message.contains("memory limit"));
        assert_eq!(following.value, LuaData::Boolean(true));
    }

    #[test]
    pub fn test_lua評価_requireはworkflow配下だけを解決して一度だけ評価する() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("part.lua"),
            "return function() return 'part' end",
        )
        .unwrap();

        let result = evaluate(
            request(
                dir.path(),
                "local a = require('part')\nlocal b = require('part')\nreturn { a(), b(), a == b }",
            ),
            TestHost::default(),
        )
        .unwrap();
        let LuaData::Table(table) = result.value else {
            panic!("table expected");
        };
        assert_eq!(
            table.as_array().unwrap(),
            vec![
                &LuaData::String("part".to_string()),
                &LuaData::String("part".to_string()),
                &LuaData::Boolean(true),
            ]
        );
    }

    #[test]
    pub fn test_lua評価_require先moduleの評価中にhost関数を呼べる() {
        // Given
        let dir = TempDir::new().unwrap();
        let module = dir.path().join("parts.lua");
        fs::write(
            &module,
            "local test = require('test')\nreturn { made = test.value('ok') }",
        )
        .unwrap();

        // When
        let result = evaluate(
            request(
                dir.path(),
                "local parts = require('parts')\nreturn parts.made",
            ),
            TestHost::default(),
        )
        .unwrap();

        // Then
        assert_eq!(result.value, LuaData::String("ok".to_string()));
        assert_eq!(result.host.calls.len(), 1);
        assert_eq!(result.host.calls[0].0, 1);
        assert_eq!(
            result.host.calls[0].1.source,
            fs::canonicalize(module).unwrap().to_string_lossy()
        );
        assert_eq!(result.host.calls[0].1.line, 2);
    }

    #[test]
    pub fn test_lua評価_require循環を検出して拒否する() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.lua"), "return require('b')").unwrap();
        fs::write(dir.path().join("b.lua"), "return require('a')").unwrap();

        let error = evaluate(
            request(dir.path(), "return require('a')"),
            TestHost::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Require);
        assert!(error.message.contains("cyclic require"));
        assert!(error.location.unwrap().source.ends_with("b.lua"));
    }

    #[test]
    pub fn test_lua評価_requireのpath走査を拒否する() {
        let dir = TempDir::new().unwrap();

        let error = evaluate(
            request(dir.path(), "return require('../outside')"),
            TestHost::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Require);
        assert!(error.message.contains("invalid require module name"));
    }

    #[cfg(unix)]
    #[test]
    pub fn test_lua評価_requireのsymlinkによるdirectory外脱出を拒否する() {
        use std::os::unix::fs::symlink;

        let workflows = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let outside_module = outside.path().join("outside.lua");
        fs::write(&outside_module, "return 'outside'").unwrap();
        symlink(&outside_module, workflows.path().join("escape.lua")).unwrap();

        let error = evaluate(
            request(workflows.path(), "return require('escape')"),
            TestHost::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Require);
        assert!(error.message.contains("outside the workflows directory"));
    }

    #[test]
    pub fn test_lua評価_構文エラーをsourceと行番号付きで返す() {
        let dir = TempDir::new().unwrap();

        let error = evaluate(
            request(dir.path(), "local ok = true\nreturn )"),
            TestHost::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Syntax);
        assert_eq!(error.location.unwrap().line, 2);
    }

    #[test]
    pub fn test_lua評価_require先の構文エラーをmodule位置で返す() {
        let dir = TempDir::new().unwrap();
        let module = dir.path().join("broken.lua");
        fs::write(&module, "local ok = true\nreturn )").unwrap();

        let error = evaluate(
            request(dir.path(), "return require('broken')"),
            TestHost::default(),
        )
        .unwrap_err();
        let location = error.location.unwrap();

        assert_eq!(error.kind, LuaFailureKind::Syntax);
        assert_eq!(
            location.source,
            fs::canonicalize(module).unwrap().to_string_lossy()
        );
        assert_eq!(location.line, 2);
    }
}
