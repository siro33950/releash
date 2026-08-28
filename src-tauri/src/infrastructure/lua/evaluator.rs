use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mlua::{
    HookTriggers, Lua, LuaOptions, MetaMethod, MultiValue, RegistryKey, StdLib, UserData,
    UserDataMethods, Value, VmState,
};

const HOOK_INSTRUCTION_INTERVAL: u32 = 10_000;

/// Lua table を `LuaData` へ変換するときの入れ子の上限。Rust 側の再帰変換が
/// stack を使い切る前に打ち切る。
const MAX_TABLE_DEPTH: usize = 64;

/// 一度の変換で扱う table 要素数の上限。共有 table の展開が組み合わせ的に
/// 増える経路を有界にする。
const MAX_TABLE_ELEMENTS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LuaLimits {
    pub(crate) memory_bytes: usize,
    pub(crate) instructions: u64,
}

impl Default for LuaLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            instructions: 50_000_000,
        }
    }
}

pub(crate) struct LuaEvaluationRequest<'a> {
    pub(crate) source_name: &'a str,
    pub(crate) source: &'a str,
    pub(crate) workflows_dir: &'a Path,
    pub(crate) limits: LuaLimits,
}

#[derive(Debug)]
pub(crate) struct LuaEvaluation<H> {
    pub(crate) value: LuaData,
    pub(crate) host: H,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LuaSourceLocation {
    pub(crate) source: String,
    pub(crate) line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LuaData {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String),
    Table(LuaTableData),
    Handle(LuaHostHandle),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum LuaTableKey {
    Boolean(bool),
    Integer(i64),
    String(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LuaTableData {
    pub(crate) entries: BTreeMap<LuaTableKey, LuaData>,
}

impl LuaTableData {
    pub(crate) fn get_string(&self, key: &str) -> Option<&LuaData> {
        self.entries.get(&LuaTableKey::String(key.to_string()))
    }

    pub(crate) fn string_keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().filter_map(|key| match key {
            LuaTableKey::String(key) => Some(key.as_str()),
            _ => None,
        })
    }

    pub(crate) fn as_array(&self) -> Option<Vec<&LuaData>> {
        if self.entries.is_empty() {
            return Some(Vec::new());
        }
        let mut values = Vec::with_capacity(self.entries.len());
        for index in 1..=self.entries.len() {
            values.push(self.entries.get(&LuaTableKey::Integer(index as i64))?);
        }
        (values.len() == self.entries.len()).then_some(values)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LuaHostHandle {
    pub(crate) kind: String,
    pub(crate) index: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LuaModule {
    pub(crate) members: BTreeMap<String, LuaModuleValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LuaModuleValue {
    Function(u32),
    Module(LuaModule),
    Data(LuaData),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LuaHostError {
    pub(crate) category: String,
    pub(crate) message: String,
    pub(crate) location: Option<LuaSourceLocation>,
    pub(crate) field: Option<String>,
}

impl fmt::Display for LuaHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.message)
    }
}

impl std::error::Error for LuaHostError {}

pub(crate) trait LuaHost {
    fn module(&self, name: &str) -> Option<LuaModule>;

    fn call(
        &mut self,
        function: u32,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError>;

    fn index(
        &mut self,
        handle: &LuaHostHandle,
        key: &str,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LuaFailureKind {
    Syntax,
    Evaluation,
    Require,
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LuaFailure {
    pub(crate) kind: LuaFailureKind,
    pub(crate) location: Option<LuaSourceLocation>,
    pub(crate) category: Option<String>,
    pub(crate) message: String,
    pub(crate) field: Option<String>,
}

impl fmt::Display for LuaFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(location) = &self.location {
            write!(
                formatter,
                "{}:{}: {}",
                location.source, location.line, self.message
            )
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl std::error::Error for LuaFailure {}

#[derive(Debug, Clone)]
struct CallbackFailure(LuaFailure);

impl fmt::Display for CallbackFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for CallbackFailure {}

struct HostUserData<H> {
    handle: LuaHostHandle,
    host: Rc<RefCell<H>>,
}

impl<H: LuaHost + 'static> UserData for HostUserData<H> {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(
            MetaMethod::Index,
            |lua, this, key: String| -> mlua::Result<Value> {
                let location = caller_location(lua);
                let result = this
                    .host
                    .borrow_mut()
                    .index(&this.handle, &key, location.clone())
                    .map_err(|error| host_error_to_mlua(error, location))?;
                data_to_lua(lua, result, Rc::clone(&this.host))
            },
        );
    }
}

pub(crate) fn evaluate<H: LuaHost + 'static>(
    request: LuaEvaluationRequest<'_>,
    host: H,
) -> Result<LuaEvaluation<H>, LuaFailure> {
    let base_dir = canonical_workflows_dir(request.workflows_dir)?;
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )
    .map_err(|error| map_mlua_error(error, request.source_name))?;
    lua.set_memory_limit(request.limits.memory_bytes)
        .map_err(|error| map_mlua_error(error, request.source_name))?;
    scrub_globals(&lua).map_err(|error| map_mlua_error(error, request.source_name))?;
    install_instruction_limit(&lua, request.limits.instructions)
        .map_err(|error| map_mlua_error(error, request.source_name))?;

    let host = Rc::new(RefCell::new(host));
    install_require(&lua, &base_dir, Rc::clone(&host))
        .map_err(|error| map_mlua_error(error, request.source_name))?;

    let evaluated = lua
        .load(request.source)
        .set_name(request.source_name)
        .eval::<Value>()
        .map_err(|error| map_mlua_error(error, request.source_name))?;
    let value =
        lua_to_data::<H>(evaluated).map_err(|error| map_mlua_error(error, request.source_name))?;

    drop(lua);
    let host = Rc::try_unwrap(host)
        .map_err(|_| LuaFailure {
            kind: LuaFailureKind::Evaluation,
            location: Some(LuaSourceLocation {
                source: request.source_name.to_string(),
                line: 1,
            }),
            category: None,
            message: "Lua host state remained referenced after evaluation".to_string(),
            field: None,
        })?
        .into_inner();
    Ok(LuaEvaluation { value, host })
}

fn canonical_workflows_dir(path: &Path) -> Result<PathBuf, LuaFailure> {
    fs::canonicalize(path).map_err(|error| LuaFailure {
        kind: LuaFailureKind::Evaluation,
        location: None,
        category: None,
        message: format!(
            "workflows directory '{}' could not be resolved: {error}",
            path.display()
        ),
        field: None,
    })
}

fn scrub_globals(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    for name in [
        "io",
        "os",
        "package",
        "debug",
        "coroutine",
        "utf8",
        "load",
        "loadstring",
        "dofile",
        "loadfile",
        "print",
        "warn",
        "pairs",
        "next",
        "collectgarbage",
        "tostring",
    ] {
        globals.raw_set(name, Value::Nil)?;
    }
    if let Ok(math) = globals.raw_get::<mlua::Table>("math") {
        math.raw_set("random", Value::Nil)?;
        math.raw_set("randomseed", Value::Nil)?;
    }
    Ok(())
}

fn install_instruction_limit(lua: &Lua, limit: u64) -> mlua::Result<()> {
    let executed = Rc::new(RefCell::new(0_u64));
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTION_INTERVAL),
        move |_lua, debug| {
            let mut executed = executed.borrow_mut();
            *executed = executed.saturating_add(u64::from(HOOK_INSTRUCTION_INTERVAL));
            if *executed > limit {
                let source = debug.source();
                let location = LuaSourceLocation {
                    source: normalize_source_name(
                        source
                            .source
                            .as_deref()
                            .or(source.short_src.as_deref())
                            .unwrap_or("<lua>"),
                    ),
                    line: debug.current_line().unwrap_or(1),
                };
                return Err(mlua::Error::external(CallbackFailure(LuaFailure {
                    kind: LuaFailureKind::Evaluation,
                    location: Some(location),
                    category: None,
                    message: format!("Lua instruction limit of {limit} was exceeded"),
                    field: None,
                })));
            }
            Ok(VmState::Continue)
        },
    )
}

fn install_require<H: LuaHost + 'static>(
    lua: &Lua,
    base_dir: &Path,
    host: Rc<RefCell<H>>,
) -> mlua::Result<()> {
    let base_dir = base_dir.to_path_buf();
    let cache = Rc::new(RefCell::new(BTreeMap::<String, RegistryKey>::new()));
    let loading = Rc::new(RefCell::new(BTreeSet::<String>::new()));
    let require = lua.create_function(move |lua, module_name: String| {
        if let Some(key) = cache.borrow().get(&module_name) {
            return lua.registry_value::<Value>(key);
        }
        let location = caller_location(lua);
        if !loading.borrow_mut().insert(module_name.clone()) {
            return Err(callback_failure(
                LuaFailureKind::Require,
                Some(location),
                format!("cyclic require detected for module '{module_name}'"),
            ));
        }

        // 借用 guard を分岐へ持ち込むと、file module の評価中に host 関数が
        // borrow_mut() へ入り二重借用になる。lookup 結果は必ず先に束縛する。
        let host_module = host.borrow().module(&module_name);
        let result = match host_module {
            Some(module) => module_to_lua(lua, module, Rc::clone(&host)),
            None => load_file_module(lua, &base_dir, &module_name, location.clone()),
        };
        loading.borrow_mut().remove(&module_name);

        let mut value = result?;
        if matches!(value, Value::Nil) {
            value = Value::Boolean(true);
        }
        let key = lua.create_registry_value(value.clone())?;
        cache.borrow_mut().insert(module_name, key);
        Ok(value)
    })?;
    lua.globals().raw_set("require", require)
}

fn load_file_module(
    lua: &Lua,
    base_dir: &Path,
    module_name: &str,
    require_location: LuaSourceLocation,
) -> mlua::Result<Value> {
    let path = resolve_module_path(base_dir, module_name).map_err(|message| {
        callback_failure(
            LuaFailureKind::Require,
            Some(require_location.clone()),
            message,
        )
    })?;
    let source = fs::read_to_string(&path).map_err(|error| {
        callback_failure(
            LuaFailureKind::Require,
            Some(require_location),
            format!("module '{}' could not be read: {error}", path.display()),
        )
    })?;
    lua.load(&source)
        .set_name(path.to_string_lossy().as_ref())
        .eval::<Value>()
        .map_err(|error| {
            mlua::Error::external(CallbackFailure(map_mlua_error(
                error,
                path.to_string_lossy().as_ref(),
            )))
        })
}

fn resolve_module_path(base_dir: &Path, module_name: &str) -> Result<PathBuf, String> {
    let components = module_name.split('.').collect::<Vec<_>>();
    if components.is_empty()
        || components.iter().any(|component| {
            component.is_empty()
                || !component.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
        })
    {
        return Err(format!("invalid require module name '{module_name}'"));
    }
    let mut candidate = base_dir.to_path_buf();
    for component in components {
        candidate.push(component);
    }
    candidate.set_extension("lua");
    let canonical = fs::canonicalize(&candidate).map_err(|error| {
        format!(
            "module '{module_name}' could not be resolved as '{}': {error}",
            candidate.display()
        )
    })?;
    if !canonical.starts_with(base_dir) {
        return Err(format!(
            "module '{module_name}' resolves outside the workflows directory"
        ));
    }
    Ok(canonical)
}

fn module_to_lua<H: LuaHost + 'static>(
    lua: &Lua,
    module: LuaModule,
    host: Rc<RefCell<H>>,
) -> mlua::Result<Value> {
    let table = lua.create_table_with_capacity(0, module.members.len())?;
    for (name, value) in module.members {
        let value = match value {
            LuaModuleValue::Function(function_id) => {
                let callback_host = Rc::clone(&host);
                Value::Function(lua.create_function(move |lua, arguments: MultiValue| {
                    let location = caller_location(lua);
                    let arguments = arguments
                        .into_iter()
                        .map(lua_to_data::<H>)
                        .collect::<mlua::Result<Vec<_>>>()?;
                    let result = callback_host
                        .borrow_mut()
                        .call(function_id, arguments, location.clone())
                        .map_err(|error| host_error_to_mlua(error, location))?;
                    data_to_lua(lua, result, Rc::clone(&callback_host))
                })?)
            }
            LuaModuleValue::Module(module) => module_to_lua(lua, module, Rc::clone(&host))?,
            LuaModuleValue::Data(data) => data_to_lua(lua, data, Rc::clone(&host))?,
        };
        table.raw_set(name, value)?;
    }
    Ok(Value::Table(table))
}

fn lua_to_data<H: LuaHost + 'static>(value: Value) -> mlua::Result<LuaData> {
    TableConversion::default().convert::<H>(value, 0)
}

/// 一度の `lua_to_data` 呼び出しで共有する変換状態。再帰パス上の table を
/// identity で覚えて循環参照を拒否し、深さと要素数を有界にする。
#[derive(Default)]
struct TableConversion {
    visiting: Vec<*const std::ffi::c_void>,
    elements: usize,
}

impl TableConversion {
    fn convert<H: LuaHost + 'static>(
        &mut self,
        value: Value,
        depth: usize,
    ) -> mlua::Result<LuaData> {
        match value {
            Value::Nil => Ok(LuaData::Nil),
            Value::Boolean(value) => Ok(LuaData::Boolean(value)),
            Value::Integer(value) => Ok(LuaData::Integer(value)),
            Value::Number(value) => Ok(LuaData::Number(value)),
            Value::String(value) => Ok(LuaData::String(value.to_string_lossy())),
            Value::Table(table) => {
                if depth >= MAX_TABLE_DEPTH {
                    return Err(mlua::Error::runtime(format!(
                        "Lua table nesting exceeded the limit of {MAX_TABLE_DEPTH}"
                    )));
                }
                let pointer = table.to_pointer();
                if self.visiting.contains(&pointer) {
                    return Err(mlua::Error::runtime(
                        "Lua table contains a recursive reference",
                    ));
                }
                self.visiting.push(pointer);
                let mut entries = BTreeMap::new();
                for pair in table.pairs::<Value, Value>() {
                    let (key, value) = pair?;
                    self.elements += 1;
                    if self.elements > MAX_TABLE_ELEMENTS {
                        return Err(mlua::Error::runtime(format!(
                            "Lua table conversion exceeded the limit of {MAX_TABLE_ELEMENTS} entries"
                        )));
                    }
                    let key = match key {
                        Value::Boolean(key) => LuaTableKey::Boolean(key),
                        Value::Integer(key) => LuaTableKey::Integer(key),
                        Value::String(key) => LuaTableKey::String(key.to_string_lossy()),
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "unsupported Lua table key type '{}'",
                                other.type_name()
                            )))
                        }
                    };
                    entries.insert(key, self.convert::<H>(value, depth + 1)?);
                }
                self.visiting.pop();
                Ok(LuaData::Table(LuaTableData { entries }))
            }
            Value::UserData(userdata) => {
                let borrowed = userdata.borrow::<HostUserData<H>>()?;
                Ok(LuaData::Handle(borrowed.handle.clone()))
            }
            other => Err(mlua::Error::runtime(format!(
                "unsupported Lua value type '{}'",
                other.type_name()
            ))),
        }
    }
}

fn data_to_lua<H: LuaHost + 'static>(
    lua: &Lua,
    value: LuaData,
    host: Rc<RefCell<H>>,
) -> mlua::Result<Value> {
    match value {
        LuaData::Nil => Ok(Value::Nil),
        LuaData::Boolean(value) => Ok(Value::Boolean(value)),
        LuaData::Integer(value) => Ok(Value::Integer(value)),
        LuaData::Number(value) => Ok(Value::Number(value)),
        LuaData::String(value) => Ok(Value::String(lua.create_string(value)?)),
        LuaData::Table(table_data) => {
            let table = lua.create_table_with_capacity(0, table_data.entries.len())?;
            for (key, value) in table_data.entries {
                let key = match key {
                    LuaTableKey::Boolean(value) => Value::Boolean(value),
                    LuaTableKey::Integer(value) => Value::Integer(value),
                    LuaTableKey::String(value) => Value::String(lua.create_string(value)?),
                };
                table.raw_set(key, data_to_lua(lua, value, Rc::clone(&host))?)?;
            }
            Ok(Value::Table(table))
        }
        LuaData::Handle(handle) => Ok(Value::UserData(
            lua.create_userdata(HostUserData { handle, host })?,
        )),
    }
}

fn host_error_to_mlua(mut error: LuaHostError, fallback: LuaSourceLocation) -> mlua::Error {
    if error.location.is_none() {
        error.location = Some(fallback);
    }
    mlua::Error::external(CallbackFailure(LuaFailure {
        kind: LuaFailureKind::Host,
        location: error.location,
        category: Some(error.category),
        message: error.message,
        field: error.field,
    }))
}

fn callback_failure(
    kind: LuaFailureKind,
    location: Option<LuaSourceLocation>,
    message: impl Into<String>,
) -> mlua::Error {
    mlua::Error::external(CallbackFailure(LuaFailure {
        kind,
        location,
        category: None,
        message: message.into(),
        field: None,
    }))
}

fn map_mlua_error(error: mlua::Error, fallback_source: &str) -> LuaFailure {
    if let Some(callback) = error.downcast_ref::<CallbackFailure>() {
        return callback.0.clone();
    }
    let display = error.to_string();
    match error {
        mlua::Error::SyntaxError { message, .. } => LuaFailure {
            kind: LuaFailureKind::Syntax,
            location: Some(LuaSourceLocation {
                source: fallback_source.to_string(),
                line: line_from_error_message(&message).unwrap_or(1),
            }),
            category: None,
            message,
            field: None,
        },
        mlua::Error::MemoryError(message) => LuaFailure {
            kind: LuaFailureKind::Evaluation,
            location: Some(LuaSourceLocation {
                source: fallback_source.to_string(),
                line: line_from_error_message(&message).unwrap_or(1),
            }),
            category: None,
            message: format!("Lua memory limit was exceeded: {message}"),
            field: None,
        },
        _ => LuaFailure {
            kind: LuaFailureKind::Evaluation,
            location: Some(LuaSourceLocation {
                source: fallback_source.to_string(),
                line: line_from_error_message(&display).unwrap_or(1),
            }),
            category: None,
            message: display,
            field: None,
        },
    }
}

fn caller_location(lua: &Lua) -> LuaSourceLocation {
    for level in 1..=8 {
        let location = lua.inspect_stack(level, |debug| {
            let source = debug.source();
            (
                source.source.map(|source| source.into_owned()),
                debug.current_line(),
            )
        });
        if let Some((Some(source), Some(line))) = location {
            if source != "=[C]" {
                return LuaSourceLocation {
                    source: normalize_source_name(&source),
                    line,
                };
            }
        }
    }
    LuaSourceLocation {
        source: "<lua>".to_string(),
        line: 1,
    }
}

fn normalize_source_name(source: &str) -> String {
    source
        .strip_prefix('@')
        .or_else(|| source.strip_prefix('='))
        .unwrap_or(source)
        .to_string()
}

fn line_from_error_message(message: &str) -> Option<usize> {
    message
        .split(':')
        .find_map(|part| part.trim().parse::<usize>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[derive(Debug, Default)]
    struct TestHost {
        calls: Vec<(u32, LuaSourceLocation)>,
    }

    impl LuaHost for TestHost {
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
    fn test_lua評価_許可moduleを呼び出して呼出位置を返す() {
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
    fn test_lua評価_標準外部ioと動的loadを公開しない() {
        let dir = TempDir::new().unwrap();
        let source = "return { io = io, os = os, package = package, load = load, print = print, pairs = pairs, next = next, collectgarbage = collectgarbage, tostring = tostring, random = math.random }";

        let result = evaluate(request(dir.path(), source), TestHost::default()).unwrap();
        let LuaData::Table(table) = result.value else {
            panic!("table expected");
        };

        assert!(table.entries.is_empty());
    }

    #[test]
    fn test_lua評価_循環参照するtableを拒否する() {
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
    fn test_lua評価_table入れ子の上限を超えたら拒否する() {
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
    fn test_lua評価_table要素数の上限を超えたら拒否する() {
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
    fn test_lua評価_命令上限で終了しない定義を打ち切る() {
        let dir = TempDir::new().unwrap();
        let mut request = request(dir.path(), "while true do end");
        request.limits.instructions = 20_000;

        let error = evaluate(request, TestHost::default()).unwrap_err();

        assert_eq!(error.kind, LuaFailureKind::Evaluation);
        assert!(error.message.contains("instruction limit"));
    }

    #[test]
    fn test_lua評価_メモリ上限で過大な定義だけを打ち切る() {
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
    fn test_lua評価_requireはworkflow配下だけを解決して一度だけ評価する() {
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
    fn test_lua評価_require先moduleの評価中にhost関数を呼べる() {
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
    fn test_lua評価_require循環を検出して拒否する() {
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
    fn test_lua評価_requireのpath走査を拒否する() {
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
    fn test_lua評価_requireのsymlinkによるdirectory外脱出を拒否する() {
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
    fn test_lua評価_構文エラーをsourceと行番号付きで返す() {
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
    fn test_lua評価_require先の構文エラーをmodule位置で返す() {
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
