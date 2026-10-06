use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use serde_json::{Number, Value};

use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::services::{contract_schema, reference};
use crate::domain::workflow::value_objects::{
    ChildEntry, CommandSpec, EnvironmentVariableName, EnvironmentVariableNameError, FacetRefs,
    FanoutSpec, InputParam, InputParameterRef, InputSourceRef, ItemsSource, NodeCompletion,
    NodeDefinition, NodeKind, NodeNamespace, NodeNamespaceError, Predicate, Rule, SchemaDef,
    SequenceSpec, SessionDelegate, SessionPermission, SessionSpec, WorkflowDefinition,
    MAIN_ENTRY_NODE_NAME,
};
use crate::infrastructure::lua::{
    evaluate, LuaData, LuaEvaluationRequest, LuaFailure, LuaHost, LuaHostError, LuaHostHandle,
    LuaLimits, LuaModule, LuaModuleValue, LuaSourceLocation, LuaTableData, LuaTableKey,
};
use crate::usecase::workflow::diagnostic_dto::DiagnosticSpan;

pub(crate) mod field_span;
pub(crate) mod stubs;

use super::predicate_wire::PredicateShapeError;
use field_span::ArtifactSpanMap;

pub(crate) use stubs::generate_editor_support;

/// 一度の評価で Lua 側から生成できる中間ハンドルの総数。Lua VM のメモリ上限は
/// Rust 側の arena を数えないため、ここで別途有界にする。
pub const MAX_HOST_ARENA_ENTRIES: usize = 100_000;

pub const HANDLE_NODE: &str = "node";
pub const HANDLE_CHILD: &str = "child";
pub const HANDLE_RULE: &str = "rule";
pub const HANDLE_PREDICATE: &str = "predicate";
const HANDLE_INPUT: &str = "input";
pub const HANDLE_SOURCE: &str = "source";
const HANDLE_SCHEMA: &str = "schema";
const HANDLE_FACET: &str = "facet";
const HANDLE_FACET_INDEX: &str = "facet_index";
const HANDLE_WORKFLOW: &str = "workflow";
const HANDLE_PROVIDER: &str = "provider";
const HANDLE_WORKTREE: &str = "worktree";
const HANDLE_COMPLETION: &str = "completion";

const FN_COMMAND: u32 = 1;
const FN_SESSION: u32 = 2;
const FN_FANOUT: u32 = 3;
const FN_SEQUENCE: u32 = 4;
const FN_CHILD: u32 = 5;
const FN_NEXT: u32 = 6;
pub const FN_WHEN: u32 = 7;
const FN_SWITCH: u32 = 8;
const FN_LOOP_GUARD: u32 = 9;
const FN_INPUT: u32 = 11;
const FN_SCHEMA_OBJECT: u32 = 12;
const FN_SCHEMA_ARRAY: u32 = 13;
const FN_SCHEMA_STRING: u32 = 14;
const FN_SCHEMA_BOOLEAN: u32 = 15;
const FN_SCHEMA_INTEGER: u32 = 16;
const FN_SCHEMA_NUMBER: u32 = 17;
const FN_WORKFLOW: u32 = 18;
pub const FN_ALL: u32 = 19;
pub const FN_ANY: u32 = 20;
const FN_DELEGATE: u32 = 21;

#[derive(Debug, Clone, Default)]
pub struct LuaFacetCatalog {
    pub instruction: Vec<String>,
    pub policy: Vec<String>,
    pub knowledge: Vec<String>,
}

pub(crate) fn facet_catalog(
    base_dir: &Path,
) -> Result<LuaFacetCatalog, crate::adaptor::gateway::workflow::facet::FacetError> {
    use crate::adaptor::gateway::workflow::facet::{self, FacetKind};

    Ok(LuaFacetCatalog {
        instruction: facet::list_facets(FacetKind::Instruction, base_dir)?,
        policy: facet::list_facets(FacetKind::Policy, base_dir)?,
        knowledge: facet::list_facets(FacetKind::Knowledge, base_dir)?,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct LuaWorkflowDefinition {
    pub workflow: WorkflowDefinition,
    pub(crate) node_locations: BTreeMap<String, LuaSourceLocation>,
    pub(crate) node_artifact_spans: BTreeMap<String, DiagnosticSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaWorkflowError {
    pub code: String,
    pub message: String,
    pub location: Option<LuaSourceLocation>,
    pub field: Option<String>,
}

impl std::fmt::Display for LuaWorkflowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LuaWorkflowError {}

pub fn load_lua_workflow(
    source_name: &str,
    source: &str,
    workflows_dir: &Path,
    facets: LuaFacetCatalog,
) -> Result<LuaWorkflowDefinition, LuaWorkflowError> {
    load_lua_workflow_with_limits(
        source_name,
        source,
        workflows_dir,
        facets,
        LuaLimits::default(),
    )
}

pub fn load_lua_workflow_with_limits(
    source_name: &str,
    source: &str,
    workflows_dir: &Path,
    facets: LuaFacetCatalog,
    limits: LuaLimits,
) -> Result<LuaWorkflowDefinition, LuaWorkflowError> {
    let host = WorkflowLuaHost::new(facets);
    let evaluation = evaluate(
        LuaEvaluationRequest {
            source_name,
            source,
            workflows_dir,
            limits,
        },
        host,
    )
    .map_err(map_evaluation_error)?;
    let workflow_index =
        expect_handle_data(&evaluation.value, HANDLE_WORKFLOW).map_err(|message| {
            LuaWorkflowError {
                code: "WFS010".to_string(),
                message,
                location: Some(LuaSourceLocation {
                    source: source_name.to_string(),
                    line: 1,
                }),
                field: None,
            }
        })?;
    evaluation.host.build(workflow_index)
}

fn map_evaluation_error(error: LuaFailure) -> LuaWorkflowError {
    let code = match error.kind {
        crate::infrastructure::lua::LuaFailureKind::Syntax => "WFS009",
        crate::infrastructure::lua::LuaFailureKind::Require => "WFS011",
        crate::infrastructure::lua::LuaFailureKind::Evaluation => "WFS010",
        crate::infrastructure::lua::LuaFailureKind::Host => {
            error.category.as_deref().unwrap_or("WFS002")
        }
    };
    LuaWorkflowError {
        code: code.to_string(),
        message: error.message,
        location: error.location,
        field: error.field,
    }
}

#[derive(Debug, Clone)]
struct NodeDraft {
    name: Option<String>,
    kind: NodeDraftKind,
    artifact: Option<usize>,
    input: Vec<usize>,
    completion: NodeCompletion,
    delegate: Option<DelegateDraft>,
    worktree: Option<crate::domain::workflow::WorktreeMode>,
    location: LuaSourceLocation,
}

#[derive(Debug, Clone)]
struct DelegateDraft {
    child: usize,
    inputs: Vec<(String, usize)>,
    when: Predicate<usize>,
    max_iterations: u32,
    location: LuaSourceLocation,
}

#[derive(Debug, Clone)]
enum NodeDraftKind {
    Command {
        command: String,
        env: Vec<(EnvironmentVariableName, usize)>,
    },
    Session {
        provider: ProviderKind,
        model: Option<String>,
        permission: Option<SessionPermission>,
        facets: FacetRefs,
    },
    Fanout {
        children: Vec<usize>,
        items: Option<FanoutItemsDraft>,
    },
    Sequence {
        children: Vec<usize>,
        entry: Option<usize>,
    },
}

#[derive(Debug, Clone)]
enum FanoutItemsDraft {
    Literal(Vec<Value>),
    Source(usize),
}

#[derive(Debug, Clone)]
struct ChildDraft {
    node: usize,
    inputs: Vec<(String, usize)>,
    rules: Option<Vec<usize>>,
    location: LuaSourceLocation,
}

#[derive(Debug, Clone)]
pub enum RuleDraft {
    Next(usize),
    When {
        on: Predicate<usize>,
        on_true: usize,
        next: usize,
    },
    Switch {
        on: usize,
        cases: BTreeMap<String, usize>,
        next: Option<usize>,
    },
    LoopGuard {
        max_iterations: u32,
        on_exhausted: usize,
    },
}

#[derive(Debug, Clone)]
struct InputDraft {
    name: String,
    contract: Option<usize>,
}

#[derive(Debug, Clone)]
pub enum SourceDraft {
    Node {
        node: usize,
        path: usize,
        location: LuaSourceLocation,
    },
    Input {
        input: usize,
        path: usize,
        location: LuaSourceLocation,
    },
    Request,
    Items,
}

impl SourceDraft {
    fn path(&self) -> usize {
        match self {
            Self::Node { path, .. } | Self::Input { path, .. } => *path,
            Self::Request => SourcePaths::REQUEST,
            Self::Items => SourcePaths::ITEMS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceRoot {
    Node(usize),
    Input(usize),
}

#[derive(Debug, Default)]
struct SourcePaths {
    roots: HashMap<SourceRoot, usize>,
    children: HashMap<usize, HashMap<String, usize>>,
    parents: Vec<Option<(usize, String)>>,
    consumed: RefCell<HashSet<usize>>,
}

impl SourcePaths {
    const REQUEST: usize = 0;
    const ITEMS: usize = 1;

    fn new() -> Self {
        Self {
            parents: vec![None, None],
            ..Self::default()
        }
    }

    fn root(&mut self, root: SourceRoot) -> usize {
        if let Some(path) = self.roots.get(&root) {
            return *path;
        }
        let path = self.parents.len();
        self.parents.push(None);
        self.roots.insert(root, path);
        path
    }

    fn child(&mut self, parent: usize, field: &str) -> usize {
        if let Some(path) = self
            .children
            .get(&parent)
            .and_then(|children| children.get(field))
        {
            return *path;
        }
        let path = self.parents.len();
        self.parents.push(Some((parent, field.to_string())));
        self.children
            .entry(parent)
            .or_default()
            .insert(field.to_string(), path);
        path
    }

    fn is_root(&self, path: usize) -> bool {
        self.parents[path].is_none()
    }

    fn fields(&self, mut path: usize) -> Vec<String> {
        let mut fields = Vec::new();
        while let Some((parent, field)) = &self.parents[path] {
            fields.push(field.clone());
            path = *parent;
        }
        fields.reverse();
        fields
    }

    fn mark(&self, path: usize) {
        let mut consumed = self.consumed.borrow_mut();
        let mut current = Some(path);
        while let Some(path) = current {
            if !consumed.insert(path) {
                break;
            }
            current = self.parents[path].as_ref().map(|(parent, _)| *parent);
        }
    }

    fn contains(&self, path: usize) -> bool {
        self.consumed.borrow().contains(&path)
    }
}

#[derive(Debug, Clone)]
struct SchemaDraft {
    name: Option<String>,
    kind: SchemaDraftKind,
}

#[derive(Debug, Clone)]
enum SchemaDraftKind {
    Object {
        properties: BTreeMap<String, usize>,
        required: BTreeSet<String>,
    },
    Array {
        items: usize,
    },
    String {
        values: Option<Vec<String>>,
    },
    Boolean,
    Integer,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FacetKind {
    Instruction,
    Policy,
    Knowledge,
}

#[derive(Debug, Clone)]
struct FacetDraft {
    kind: FacetKind,
    key: String,
}

#[derive(Debug, Clone)]
struct WorkflowDraft {
    name: String,
    description: String,
    main: usize,
}

#[derive(Debug)]
pub struct WorkflowLuaHost {
    artifact_spans: HashMap<String, ArtifactSpanMap>,
    nodes: Vec<NodeDraft>,
    children: Vec<ChildDraft>,
    rules: Vec<RuleDraft>,
    predicates: Vec<(Predicate<usize>, usize)>,
    predicate_entries: usize,
    inputs: Vec<InputDraft>,
    sources: Vec<SourceDraft>,
    source_paths: SourcePaths,
    schemas: Vec<SchemaDraft>,
    facets: Vec<FacetDraft>,
    workflows: Vec<WorkflowDraft>,
    facet_module: LuaModule,
    facet_keys: [BTreeSet<String>; 3],
    request_source: usize,
    items_source: usize,
}

impl WorkflowLuaHost {
    pub fn new(catalog: LuaFacetCatalog) -> Self {
        let mut host = Self {
            artifact_spans: HashMap::new(),
            nodes: Vec::new(),
            children: Vec::new(),
            rules: Vec::new(),
            predicates: Vec::new(),
            predicate_entries: 0,
            inputs: Vec::new(),
            sources: vec![SourceDraft::Request, SourceDraft::Items],
            source_paths: SourcePaths::new(),
            schemas: Vec::new(),
            facets: Vec::new(),
            workflows: Vec::new(),
            facet_module: LuaModule::default(),
            facet_keys: [
                catalog.instruction.into_iter().collect(),
                catalog.policy.into_iter().collect(),
                catalog.knowledge.into_iter().collect(),
            ],
            request_source: 0,
            items_source: 1,
        };
        host.facet_module = host.make_facet_module();
        host
    }

    fn make_facet_module(&self) -> LuaModule {
        LuaModule {
            members: BTreeMap::from([
                (
                    "instruction".to_string(),
                    LuaModuleValue::Data(handle(HANDLE_FACET_INDEX, 0)),
                ),
                (
                    "policy".to_string(),
                    LuaModuleValue::Data(handle(HANDLE_FACET_INDEX, 1)),
                ),
                (
                    "knowledge".to_string(),
                    LuaModuleValue::Data(handle(HANDLE_FACET_INDEX, 2)),
                ),
            ]),
        }
    }

    fn releash_module(&self) -> LuaModule {
        let functions = [
            ("command", FN_COMMAND),
            ("session", FN_SESSION),
            ("fanout", FN_FANOUT),
            ("sequence", FN_SEQUENCE),
            ("child", FN_CHILD),
            ("next", FN_NEXT),
            ("when", FN_WHEN),
            ("all", FN_ALL),
            ("any", FN_ANY),
            ("switch", FN_SWITCH),
            ("loop_guard", FN_LOOP_GUARD),
            ("input", FN_INPUT),
            ("workflow", FN_WORKFLOW),
        ];
        let mut members = BTreeMap::new();
        for (name, function) in functions {
            members.insert(name.to_string(), LuaModuleValue::Function(function));
        }
        members.insert(
            "request".to_string(),
            LuaModuleValue::Data(handle(HANDLE_SOURCE, self.request_source)),
        );
        members.insert(
            "items".to_string(),
            LuaModuleValue::Data(handle(HANDLE_SOURCE, self.items_source)),
        );
        members.insert(
            "worktree".to_string(),
            LuaModuleValue::Module(LuaModule {
                members: BTreeMap::from([
                    (
                        "shared".to_string(),
                        LuaModuleValue::Data(handle(HANDLE_WORKTREE, 0)),
                    ),
                    (
                        "isolated".to_string(),
                        LuaModuleValue::Data(handle(HANDLE_WORKTREE, 1)),
                    ),
                ]),
            }),
        );
        members.insert(
            "completion".to_string(),
            LuaModuleValue::Module(LuaModule {
                members: BTreeMap::from([(
                    "approval".to_string(),
                    LuaModuleValue::Data(handle(HANDLE_COMPLETION, 0)),
                )]),
            }),
        );
        members.insert(
            "provider".to_string(),
            LuaModuleValue::Module(LuaModule {
                members: BTreeMap::from([
                    (
                        "claude".to_string(),
                        LuaModuleValue::Data(handle(HANDLE_PROVIDER, 0)),
                    ),
                    (
                        "codex".to_string(),
                        LuaModuleValue::Data(handle(HANDLE_PROVIDER, 1)),
                    ),
                ]),
            }),
        );
        members.insert(
            "schema".to_string(),
            LuaModuleValue::Module(LuaModule {
                members: BTreeMap::from([
                    (
                        "object".to_string(),
                        LuaModuleValue::Function(FN_SCHEMA_OBJECT),
                    ),
                    (
                        "array".to_string(),
                        LuaModuleValue::Function(FN_SCHEMA_ARRAY),
                    ),
                    (
                        "string".to_string(),
                        LuaModuleValue::Function(FN_SCHEMA_STRING),
                    ),
                    (
                        "boolean".to_string(),
                        LuaModuleValue::Function(FN_SCHEMA_BOOLEAN),
                    ),
                    (
                        "integer".to_string(),
                        LuaModuleValue::Function(FN_SCHEMA_INTEGER),
                    ),
                    (
                        "number".to_string(),
                        LuaModuleValue::Function(FN_SCHEMA_NUMBER),
                    ),
                ]),
            }),
        );
        LuaModule { members }
    }

    fn push_schema(&mut self, draft: SchemaDraft) -> LuaData {
        let index = self.schemas.len();
        self.schemas.push(draft);
        handle(HANDLE_SCHEMA, index)
    }

    fn push_source(&mut self, draft: SourceDraft) -> LuaData {
        let index = self.sources.len();
        self.sources.push(draft);
        handle(HANDLE_SOURCE, index)
    }

    /// arena に積まれた中間ハンドルの総数。
    pub fn arena_entries(&self) -> usize {
        self.nodes.len()
            + self.children.len()
            + self.rules.len()
            + self.predicate_entries
            + self.inputs.len()
            + self.sources.len()
            + self.schemas.len()
            + self.facets.len()
            + self.workflows.len()
    }

    /// Lua VM のメモリ上限は Rust 側の arena を数えないため、ビルダー呼び出しの
    /// 入口で総数を有界にする。`MAX_NODES_PER_WORKFLOW` に収まる定義は到達しない。
    fn ensure_arena_budget(
        &self,
        additional_entries: usize,
        location: &LuaSourceLocation,
    ) -> Result<(), LuaHostError> {
        if self.arena_entries().saturating_add(additional_entries) > MAX_HOST_ARENA_ENTRIES {
            return Err(host_error(
                "WFS010",
                format!(
                    "Lua definition exceeded the limit of {MAX_HOST_ARENA_ENTRIES} builder values"
                ),
                location.clone(),
            ));
        }
        Ok(())
    }

    fn build(self, workflow_index: usize) -> Result<LuaWorkflowDefinition, LuaWorkflowError> {
        WorkflowGraphBuilder::new(self).build(workflow_index)
    }
}

impl LuaHost for WorkflowLuaHost {
    fn source_loaded(&mut self, name: &str, source: &str) {
        self.artifact_spans
            .insert(name.to_string(), ArtifactSpanMap::parse(source));
    }

    fn module(&self, name: &str) -> Option<LuaModule> {
        match name {
            "releash" => Some(self.releash_module()),
            "facets" => Some(self.facet_module.clone()),
            _ => None,
        }
    }

    fn call(
        &mut self,
        function: u32,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        self.ensure_arena_budget(1, &location)?;
        match function {
            FN_COMMAND => self.call_command(arguments, location),
            FN_SESSION => self.call_session(arguments, location),
            FN_DELEGATE => self.call_delegate(arguments, location),
            FN_FANOUT => self.call_fanout(arguments, location),
            FN_SEQUENCE => self.call_sequence(arguments, location),
            FN_CHILD => self.call_child(arguments, location),
            FN_NEXT => self.call_next(arguments, location),
            FN_WHEN => self.call_when(arguments, location),
            FN_ALL | FN_ANY => self.call_predicate(function == FN_ALL, arguments, location),
            FN_SWITCH => self.call_switch(arguments, location),
            FN_LOOP_GUARD => self.call_loop_guard(arguments, location),
            FN_INPUT => self.call_input(arguments, location),
            FN_SCHEMA_OBJECT => self.call_schema_object(arguments, location),
            FN_SCHEMA_ARRAY => self.call_schema_array(arguments, location),
            FN_SCHEMA_STRING => self.call_schema_string(arguments, location),
            FN_SCHEMA_BOOLEAN => {
                self.call_primitive_schema(arguments, location, SchemaDraftKind::Boolean)
            }
            FN_SCHEMA_INTEGER => {
                self.call_primitive_schema(arguments, location, SchemaDraftKind::Integer)
            }
            FN_SCHEMA_NUMBER => {
                self.call_primitive_schema(arguments, location, SchemaDraftKind::Number)
            }
            FN_WORKFLOW => self.call_workflow(arguments, location),
            _ => Err(host_error("WFS002", "unknown builder function", location)),
        }
    }

    fn index(
        &mut self,
        handle: &LuaHostHandle,
        key: &str,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        self.ensure_arena_budget(1, &location)?;
        if handle.kind == HANDLE_FACET_INDEX {
            let kind = match handle.index {
                0 => FacetKind::Instruction,
                1 => FacetKind::Policy,
                2 => FacetKind::Knowledge,
                _ => return Err(host_error("WFS002", "invalid facet index", location)),
            };
            if !self.facet_keys[handle.index].contains(key) {
                return Err(host_error(
                    "WFR900",
                    format!("facet '{key}' does not exist"),
                    location,
                ));
            }
            let index = self.facets.len();
            self.facets.push(FacetDraft {
                kind,
                key: key.to_string(),
            });
            return Ok(LuaData::Handle(LuaHostHandle {
                kind: HANDLE_FACET.to_string(),
                index,
            }));
        }
        if handle.kind == HANDLE_NODE
            && key == "delegate"
            && matches!(self.nodes[handle.index].kind, NodeDraftKind::Session { .. })
        {
            return Ok(LuaData::BoundFunction {
                function: FN_DELEGATE,
                receiver: handle.clone(),
            });
        }
        let (node, parent_path) = match handle.kind.as_str() {
            HANDLE_NODE => (
                handle.index,
                self.source_paths.root(SourceRoot::Node(handle.index)),
            ),
            HANDLE_INPUT => {
                let parent_path = self.source_paths.root(SourceRoot::Input(handle.index));
                return self.index_input(handle.index, parent_path, key, location);
            }
            HANDLE_SOURCE => match self.sources.get(handle.index) {
                Some(SourceDraft::Input { input, path, .. }) => {
                    return self.index_input(*input, *path, key, location);
                }
                Some(SourceDraft::Node { node, path, .. }) => (*node, *path),
                _ => {
                    return Err(host_error(
                        "WFR003",
                        "only a node or input source can be indexed",
                        location,
                    ));
                }
            },
            _ => {
                return Err(host_error(
                    "WFR003",
                    "only a node can be indexed as an artifact source",
                    location,
                ));
            }
        };
        let path = self.source_paths.child(parent_path, key);
        Ok(self.push_source(SourceDraft::Node {
            node,
            path,
            location,
        }))
    }
}

impl WorkflowLuaHost {
    fn call_command(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(
            &table,
            &[
                "name",
                "command",
                "env",
                "artifact",
                "input",
                "completion",
                "worktree",
            ],
            &location,
        )?;
        let env = self.command_env_sources(&table, &location)?;
        let draft = NodeDraft {
            name: optional_string(&table, "name", &location)?,
            kind: NodeDraftKind::Command {
                command: required_string(&table, "command", &location)?,
                env,
            },
            artifact: optional_handle(&table, "artifact", HANDLE_SCHEMA, &location)?,
            input: optional_handle_array(&table, "input", HANDLE_INPUT, &location)?
                .unwrap_or_default(),
            completion: parse_completion(&table, &location)?,
            delegate: None,
            worktree: parse_worktree(&table, &location)?,
            location,
        };
        Ok(push_node(&mut self.nodes, draft))
    }

    fn command_env_sources(
        &mut self,
        table: &LuaTableData,
        location: &LuaSourceLocation,
    ) -> Result<Vec<(EnvironmentVariableName, usize)>, LuaHostError> {
        let values = match table.get_string("env") {
            None | Some(LuaData::Nil) => return Ok(Vec::new()),
            Some(LuaData::Table(values)) => values,
            Some(_) => return Err(type_error("env", "string-keyed table", location)),
        };
        let mut env = Vec::new();
        for (key, value) in &values.entries {
            self.ensure_arena_budget(1, location)?;
            let LuaTableKey::String(key) = key else {
                return Err(type_error("env", "string-keyed table", location));
            };
            let variable = EnvironmentVariableName::new(key.clone()).map_err(|error| {
                let code = match &error {
                    EnvironmentVariableNameError::Invalid(_) => "WFS006",
                    EnvironmentVariableNameError::Reserved(_) => "WFR004",
                };
                host_field_error(code, error.to_string(), location.clone(), "env")
            })?;
            let source = expect_handle(value, HANDLE_SOURCE)
                .or_else(|_| self.input_as_source_index(value, location))
                .map_err(|_| type_error("env", "ReleashInput values", location))?;
            if !matches!(self.sources.get(source), Some(SourceDraft::Input { .. })) {
                return Err(type_error("env", "ReleashInput values", location));
            }
            env.push((variable, source));
        }
        Ok(env)
    }

    fn call_session(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(
            &table,
            &[
                "name",
                "provider",
                "model",
                "permission",
                "facets",
                "artifact",
                "input",
                "completion",
                "worktree",
            ],
            &location,
        )?;
        let provider = match required_handle(&table, "provider", HANDLE_PROVIDER, &location)? {
            0 => ProviderKind::Claude,
            1 => ProviderKind::Codex,
            _ => return Err(host_error("WFS002", "invalid provider", location)),
        };
        let facets = match table.get_string("facets") {
            None | Some(LuaData::Nil) => FacetRefs::default(),
            Some(LuaData::Table(table)) => self.parse_facets(table, &location)?,
            Some(_) => return Err(type_error("facets", "table", &location)),
        };
        let permission = optional_string(&table, "permission", &location)?
            .map(|value| {
                value.parse::<SessionPermission>().map_err(|error| {
                    host_field_error("WFS002", error.to_string(), location.clone(), "permission")
                })
            })
            .transpose()?;
        let draft = NodeDraft {
            name: optional_string(&table, "name", &location)?,
            kind: NodeDraftKind::Session {
                provider,
                model: optional_string(&table, "model", &location)?,
                permission,
                facets,
            },
            artifact: optional_handle(&table, "artifact", HANDLE_SCHEMA, &location)?,
            input: optional_handle_array(&table, "input", HANDLE_INPUT, &location)?
                .unwrap_or_default(),
            completion: parse_completion(&table, &location)?,
            delegate: None,
            worktree: parse_worktree(&table, &location)?,
            location,
        };
        Ok(push_node(&mut self.nodes, draft))
    }

    fn parse_facets(
        &self,
        table: &LuaTableData,
        location: &LuaSourceLocation,
    ) -> Result<FacetRefs, LuaHostError> {
        reject_unknown(table, &["policy", "knowledge", "instruction"], location)?;
        let policy = self.optional_facet(table, "policy", FacetKind::Policy, location)?;
        let instruction =
            self.optional_facet(table, "instruction", FacetKind::Instruction, location)?;
        let knowledge = match table.get_string("knowledge") {
            None | Some(LuaData::Nil) => Vec::new(),
            Some(LuaData::Table(values)) => values
                .as_array()
                .ok_or_else(|| type_error("knowledge", "array", location))?
                .into_iter()
                .map(|value| self.facet_key(value, FacetKind::Knowledge, "knowledge", location))
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(type_error("knowledge", "array", location)),
        };
        Ok(FacetRefs {
            policy,
            knowledge,
            instruction,
        })
    }

    fn optional_facet(
        &self,
        table: &LuaTableData,
        field: &str,
        expected: FacetKind,
        location: &LuaSourceLocation,
    ) -> Result<Option<String>, LuaHostError> {
        match table.get_string(field) {
            None | Some(LuaData::Nil) => Ok(None),
            Some(value) => self.facet_key(value, expected, field, location).map(Some),
        }
    }

    fn facet_key(
        &self,
        value: &LuaData,
        expected: FacetKind,
        field: &str,
        location: &LuaSourceLocation,
    ) -> Result<String, LuaHostError> {
        let index = expect_handle(value, HANDLE_FACET)
            .map_err(|_| type_error(field, "matching Facet", location))?;
        let facet = self
            .facets
            .get(index)
            .ok_or_else(|| type_error(field, "matching Facet", location))?;
        if facet.kind != expected {
            return Err(type_error(field, "matching Facet", location));
        }
        Ok(facet.key.clone())
    }

    fn call_fanout(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(
            &table,
            &[
                "name",
                "children",
                "items",
                "artifact",
                "input",
                "completion",
                "worktree",
            ],
            &location,
        )?;
        let items = match table.get_string("items") {
            None | Some(LuaData::Nil) => None,
            Some(value @ LuaData::Handle(_)) => {
                let source = expect_handle(value, HANDLE_SOURCE)
                    .or_else(|_| self.node_as_source_index(value, &location))
                    .map_err(|_| type_error("items", "Source or literal array", &location))?;
                if let Some(SourceDraft::Input {
                    input,
                    path,
                    location: source_location,
                }) = self.sources.get(source)
                {
                    if !self.source_paths.is_root(*path) && self.inputs[*input].contract.is_none() {
                        return Err(host_error(
                            "WFR003",
                            "input does not declare a contract",
                            source_location.clone(),
                        ));
                    }
                }
                if !matches!(
                    self.sources.get(source),
                    Some(SourceDraft::Node { path, .. }) if !self.source_paths.is_root(*path)
                ) {
                    return Err(host_error(
                        "WFR003",
                        "fanout items must reference an artifact field",
                        location,
                    ));
                }
                Some(FanoutItemsDraft::Source(source))
            }
            Some(LuaData::Table(values)) => Some(FanoutItemsDraft::Literal(lua_array_to_json(
                values, &location,
            )?)),
            Some(_) => return Err(type_error("items", "Source or literal array", &location)),
        };
        let draft = NodeDraft {
            name: optional_string(&table, "name", &location)?,
            kind: NodeDraftKind::Fanout {
                children: required_handle_array(&table, "children", HANDLE_CHILD, &location)?,
                items,
            },
            artifact: optional_handle(&table, "artifact", HANDLE_SCHEMA, &location)?,
            input: optional_handle_array(&table, "input", HANDLE_INPUT, &location)?
                .unwrap_or_default(),
            completion: parse_completion(&table, &location)?,
            delegate: None,
            worktree: parse_worktree(&table, &location)?,
            location,
        };
        Ok(push_node(&mut self.nodes, draft))
    }

    fn node_as_source_index(
        &mut self,
        value: &LuaData,
        location: &LuaSourceLocation,
    ) -> Result<usize, ()> {
        let node = expect_handle(value, HANDLE_NODE)?;
        let index = self.sources.len();
        let path = self.source_paths.root(SourceRoot::Node(node));
        self.sources.push(SourceDraft::Node {
            node,
            path,
            location: location.clone(),
        });
        Ok(index)
    }

    fn call_sequence(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(
            &table,
            &[
                "name",
                "entry",
                "children",
                "artifact",
                "input",
                "completion",
                "worktree",
            ],
            &location,
        )?;
        let draft = NodeDraft {
            name: optional_string(&table, "name", &location)?,
            kind: NodeDraftKind::Sequence {
                children: required_handle_array(&table, "children", HANDLE_CHILD, &location)?,
                entry: optional_handle(&table, "entry", HANDLE_NODE, &location)?,
            },
            artifact: optional_handle(&table, "artifact", HANDLE_SCHEMA, &location)?,
            input: optional_handle_array(&table, "input", HANDLE_INPUT, &location)?
                .unwrap_or_default(),
            completion: parse_completion(&table, &location)?,
            delegate: None,
            worktree: parse_worktree(&table, &location)?,
            location,
        };
        Ok(push_node(&mut self.nodes, draft))
    }

    fn call_child(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(&table, &["node", "inputs", "rules"], &location)?;
        let inputs = self.parse_inputs(&table, &location)?;
        let rules = optional_handle_array(&table, "rules", HANDLE_RULE, &location)?;
        let index = self.children.len();
        self.children.push(ChildDraft {
            node: required_handle(&table, "node", HANDLE_NODE, &location)?,
            inputs,
            rules,
            location,
        });
        Ok(handle(HANDLE_CHILD, index))
    }

    fn parse_inputs(
        &mut self,
        table: &LuaTableData,
        location: &LuaSourceLocation,
    ) -> Result<Vec<(String, usize)>, LuaHostError> {
        Ok(match table.get_string("inputs") {
            None | Some(LuaData::Nil) => Vec::new(),
            Some(LuaData::Table(values)) => {
                let mut result = Vec::new();
                for (key, value) in &values.entries {
                    // inputs は 1 回の呼び出しで要素数ぶんの Source を積むため、
                    // 呼び出し入口の検査だけでは上限を超えられる。要素ごとに見る。
                    self.ensure_arena_budget(1, location)?;
                    let LuaTableKey::String(key) = key else {
                        return Err(type_error("inputs", "string-keyed table", location));
                    };
                    let source = self
                        .source_index(value, location)
                        .map_err(|_| type_error("inputs", "Source values", location))?;
                    result.push((key.clone(), source));
                }
                result
            }
            Some(_) => return Err(type_error("inputs", "table", location)),
        })
    }

    fn call_delegate(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let invalid = |message: &str| {
            host_field_error("WFS002", message, location.clone(), "completion.delegate")
        };
        let [receiver, LuaData::Table(table)] = arguments.as_slice() else {
            return Err(invalid("completion delegate must be a map"));
        };
        let owner = expect_handle(receiver, HANDLE_NODE)
            .map_err(|_| invalid("delegate requires a Session handle"))?;
        if self.nodes[owner].delegate.is_some() {
            return Err(invalid(
                "the same Session handle cannot declare delegate twice",
            ));
        }
        if table.entries.keys().any(|key| {
            !matches!(key, LuaTableKey::String(key) if matches!(key.as_str(), "child" | "inputs" | "when" | "max_iterations"))
        }) {
            return Err(invalid(
                "completion delegate only accepts child, inputs, when, and max_iterations",
            ));
        }
        let child = table
            .get_string("child")
            .and_then(|value| expect_handle(value, HANDLE_NODE).ok())
            .ok_or_else(|| invalid("completion delegate requires a child node name"))?;
        let when = table
            .get_string("when")
            .ok_or_else(|| invalid("completion delegate requires when"))?;
        let when = self
            .predicate_value(when, "completion.delegate.when", &location)?
            .0;
        let max_iterations = match table.get_string("max_iterations") {
            Some(LuaData::Integer(value)) => u32::try_from(*value).ok(),
            _ => None,
        }
        .ok_or_else(|| {
            invalid("completion delegate max_iterations must be an unsigned 32-bit integer")
        })?;
        let inputs = self.parse_inputs(table, &location)?;
        self.nodes[owner].delegate = Some(DelegateDraft {
            child,
            inputs,
            when,
            max_iterations,
            location,
        });
        Ok(LuaData::Nil)
    }

    fn input_as_source_index(
        &mut self,
        value: &LuaData,
        location: &LuaSourceLocation,
    ) -> Result<usize, ()> {
        let input = expect_handle(value, HANDLE_INPUT)?;
        let index = self.sources.len();
        let path = self.source_paths.root(SourceRoot::Input(input));
        self.sources.push(SourceDraft::Input {
            input,
            path,
            location: location.clone(),
        });
        Ok(index)
    }

    fn call_next(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let node = one_handle(arguments, HANDLE_NODE, &location)?;
        Ok(push_rule(&mut self.rules, RuleDraft::Next(node)))
    }

    fn call_when(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(&table, &["on", "on_true", "next"], &location)?;
        let value = table
            .get_string("on")
            .ok_or_else(|| missing_field("on", &location))?;
        let on = if expect_handle(value, HANDLE_PREDICATE).is_ok() {
            self.predicate_value(value, "on", &location)?.0
        } else {
            Predicate::Ref(self.predicate_source(value, "on", &location)?)
        };
        self.ensure_arena_budget(1, &location)?;
        let draft = RuleDraft::When {
            on,
            on_true: required_handle(&table, "on_true", HANDLE_NODE, &location)?,
            next: required_handle(&table, "next", HANDLE_NODE, &location)?,
        };
        Ok(push_rule(&mut self.rules, draft))
    }

    fn call_predicate(
        &mut self,
        all: bool,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let elements = match arguments.as_slice() {
            [LuaData::Table(table)] => table.as_array(),
            _ => None,
        }
        .ok_or_else(|| {
            host_error(
                "WFS002",
                PredicateShapeError::ExpectedArray.to_string(),
                location.clone(),
            )
        })?;
        let mut predicates = Vec::with_capacity(elements.len());
        let mut size = 1;
        for value in elements {
            let (predicate, entries) =
                self.predicate_value(value, "predicate element", &location)?;
            predicates.push(predicate);
            size += entries;
        }
        let predicate = if all {
            Predicate::and(predicates)
        } else {
            Predicate::or(predicates)
        }
        .map_err(|error| {
            host_error(
                "WFS002",
                PredicateShapeError::from(error).to_string(),
                location.clone(),
            )
        })?;
        self.ensure_arena_budget(1, &location)?;
        self.predicate_entries += 1;
        let index = self.predicates.len();
        self.predicates.push((predicate, size));
        Ok(handle(HANDLE_PREDICATE, index))
    }

    fn predicate_value(
        &mut self,
        value: &LuaData,
        field: &str,
        location: &LuaSourceLocation,
    ) -> Result<(Predicate<usize>, usize), LuaHostError> {
        if let Ok(index) = expect_handle(value, HANDLE_PREDICATE) {
            let (predicate, size) = self.predicates.get(index).ok_or_else(|| {
                host_field_error(
                    "WFS002",
                    PredicateShapeError::InvalidPredicate.to_string(),
                    location.clone(),
                    field,
                )
            })?;
            self.ensure_arena_budget(*size, location)?;
            let result = (predicate.clone(), *size);
            self.predicate_entries += size;
            return Ok(result);
        }
        self.ensure_arena_budget(1, location)?;
        let source = self.predicate_source(value, field, location)?;
        self.ensure_arena_budget(1, location)?;
        self.predicate_entries += 1;
        Ok((Predicate::Ref(source), 1))
    }

    fn predicate_source(
        &mut self,
        value: &LuaData,
        field: &str,
        location: &LuaSourceLocation,
    ) -> Result<usize, LuaHostError> {
        self.source_index(value, location).map_err(|_| {
            host_field_error(
                "WFS002",
                PredicateShapeError::InvalidPredicate.to_string(),
                location.clone(),
                field,
            )
        })
    }

    fn call_switch(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(&table, &["on", "cases", "next"], &location)?;
        let cases = required_table(&table, "cases", &location)?;
        let mut targets = BTreeMap::new();
        for (key, value) in &cases.entries {
            let key = lua_key_as_case(key)
                .ok_or_else(|| type_error("cases", "scalar-keyed Node map", &location))?;
            let target = expect_handle(value, HANDLE_NODE)
                .map_err(|_| type_error("cases", "scalar-keyed Node map", &location))?;
            targets.insert(key, target);
        }
        let draft = RuleDraft::Switch {
            on: self.required_source(&table, "on", &location)?,
            cases: targets,
            next: optional_handle(&table, "next", HANDLE_NODE, &location)?,
        };
        Ok(push_rule(&mut self.rules, draft))
    }

    fn call_loop_guard(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(&table, &["max_iterations", "on_exhausted"], &location)?;
        let max_iterations = required_u32(&table, "max_iterations", &location)?;
        let draft = RuleDraft::LoopGuard {
            max_iterations,
            on_exhausted: required_handle(&table, "on_exhausted", HANDLE_NODE, &location)?,
        };
        Ok(push_rule(&mut self.rules, draft))
    }

    fn call_input(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        if !(1..=2).contains(&arguments.len()) {
            return Err(host_error(
                "WFS002",
                "input expects name and optional contract",
                location,
            ));
        }
        let name = match &arguments[0] {
            LuaData::String(value) if !value.is_empty() => value.clone(),
            _ => return Err(type_error("input name", "non-empty string", &location)),
        };
        let contract = arguments
            .get(1)
            .map(|value| expect_handle(value, HANDLE_SCHEMA))
            .transpose()
            .map_err(|_| type_error("input contract", "Schema", &location))?;
        let index = self.inputs.len();
        self.inputs.push(InputDraft { name, contract });
        Ok(handle(HANDLE_INPUT, index))
    }

    fn call_schema_object(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(&table, &["name", "properties", "required"], &location)?;
        let raw_properties = required_table(&table, "properties", &location)?;
        let mut properties = BTreeMap::new();
        for (key, value) in &raw_properties.entries {
            let LuaTableKey::String(key) = key else {
                return Err(type_error(
                    "properties",
                    "string-keyed Schema map",
                    &location,
                ));
            };
            let schema = expect_handle(value, HANDLE_SCHEMA)
                .map_err(|_| type_error("properties", "string-keyed Schema map", &location))?;
            properties.insert(key.clone(), schema);
        }
        let required = optional_string_array(&table, "required", &location)?
            .unwrap_or_default()
            .into_iter()
            .collect();
        Ok(self.push_schema(SchemaDraft {
            name: optional_string(&table, "name", &location)?,
            kind: SchemaDraftKind::Object {
                properties,
                required,
            },
        }))
    }

    fn call_schema_array(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(&table, &["name", "items"], &location)?;
        Ok(self.push_schema(SchemaDraft {
            name: optional_string(&table, "name", &location)?,
            kind: SchemaDraftKind::Array {
                items: required_handle(&table, "items", HANDLE_SCHEMA, &location)?,
            },
        }))
    }

    fn call_schema_string(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(&table, &["enum"], &location)?;
        Ok(self.push_schema(SchemaDraft {
            name: None,
            kind: SchemaDraftKind::String {
                values: optional_string_array(&table, "enum", &location)?,
            },
        }))
    }

    fn call_primitive_schema(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
        kind: SchemaDraftKind,
    ) -> Result<LuaData, LuaHostError> {
        if !arguments.is_empty() {
            return Err(host_error(
                "WFS002",
                "primitive schema expects no arguments",
                location,
            ));
        }
        Ok(self.push_schema(SchemaDraft { name: None, kind }))
    }

    fn call_workflow(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let table = one_table(arguments, &location)?;
        reject_unknown(&table, &["name", "description", "main"], &location)?;
        let name = required_string(&table, "name", &location)?;
        let description = required_string(&table, "description", &location)?;
        let index = self.workflows.len();
        let main = match table.get_string("main") {
            None | Some(LuaData::Nil) => {
                return Err(host_error(
                    "WFR006",
                    "workflow main node does not exist",
                    location,
                ));
            }
            Some(value) => expect_handle(value, HANDLE_NODE)
                .map_err(|_| type_error("main", HANDLE_NODE, &location))?,
        };
        self.workflows.push(WorkflowDraft {
            name,
            description,
            main,
        });
        Ok(handle(HANDLE_WORKFLOW, index))
    }

    fn required_source(
        &mut self,
        table: &LuaTableData,
        field: &str,
        location: &LuaSourceLocation,
    ) -> Result<usize, LuaHostError> {
        let value = table
            .get_string(field)
            .ok_or_else(|| missing_field(field, location))?;
        self.source_index(value, location)
            .map_err(|_| type_error(field, "Source", location))
    }

    fn source_index(&mut self, value: &LuaData, location: &LuaSourceLocation) -> Result<usize, ()> {
        expect_handle(value, HANDLE_SOURCE)
            .or_else(|_| self.node_as_source_index(value, location))
            .or_else(|_| self.input_as_source_index(value, location))
    }

    fn index_input(
        &mut self,
        input: usize,
        parent_path: usize,
        key: &str,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let path = self.source_paths.child(parent_path, key);
        Ok(self.push_source(SourceDraft::Input {
            input,
            path,
            location,
        }))
    }

    fn validate_schema_path(
        &self,
        mut schema: usize,
        fields: &[String],
        source_kind: &str,
    ) -> Result<(), String> {
        for field in fields {
            let draft = self
                .schemas
                .get(schema)
                .ok_or_else(|| "unknown artifact schema".to_string())?;
            let SchemaDraftKind::Object { properties, .. } = &draft.kind else {
                return Err(format!(
                    "{source_kind} field '{field}' cannot be read from a non-object schema"
                ));
            };
            schema = *properties
                .get(field)
                .ok_or_else(|| format!("{source_kind} field '{field}' does not exist"))?;
        }
        Ok(())
    }

    fn mark_source_consumed(&self, source: usize) {
        let Some(source) = self.sources.get(source) else {
            return;
        };
        self.source_paths.mark(source.path());
    }
}

struct WorkflowGraphBuilder {
    host: WorkflowLuaHost,
    namespace: NodeNamespace,
    names: HashMap<usize, String>,
    child_uses: HashSet<usize>,
    nodes: Vec<NodeDefinition>,
    locations: BTreeMap<String, LuaSourceLocation>,
    artifact_spans: BTreeMap<String, DiagnosticSpan>,
    schemas: BTreeMap<String, SchemaDef>,
    schema_names: HashMap<usize, String>,
}

impl WorkflowGraphBuilder {
    fn new(host: WorkflowLuaHost) -> Self {
        Self {
            host,
            namespace: NodeNamespace::default(),
            names: HashMap::new(),
            child_uses: HashSet::new(),
            nodes: Vec::new(),
            locations: BTreeMap::new(),
            artifact_spans: BTreeMap::new(),
            schemas: BTreeMap::new(),
            schema_names: HashMap::new(),
        }
    }

    fn build(mut self, workflow_index: usize) -> Result<LuaWorkflowDefinition, LuaWorkflowError> {
        let workflow = self
            .host
            .workflows
            .get(workflow_index)
            .cloned()
            .ok_or_else(|| build_error("WFS010", "returned Workflow handle is invalid", None))?;
        let main = self
            .host
            .nodes
            .get(workflow.main)
            .ok_or_else(|| build_error("WFR006", "workflow main node does not exist", None))?;
        if main.name.is_some() {
            return Err(build_error(
                "WFS006",
                "workflow main node must not declare a name",
                Some(main.location.clone()),
            ));
        }
        self.namespace
            .register(MAIN_ENTRY_NODE_NAME)
            .map_err(|error| {
                build_error("WFS006", error.to_string(), Some(main.location.clone()))
            })?;
        self.names
            .insert(workflow.main, MAIN_ENTRY_NODE_NAME.to_string());
        self.child_uses.insert(workflow.main);
        self.visit_node(workflow.main)?;
        let workflow = WorkflowDefinition {
            name: workflow.name,
            description: workflow.description,
            builtin: false,
            schemas: std::mem::take(&mut self.schemas),
            nodes: std::mem::take(&mut self.nodes),
            entry: MAIN_ENTRY_NODE_NAME.to_string(),
        };
        self.validate_unconsumed_sources(&workflow)?;
        Ok(LuaWorkflowDefinition {
            workflow,
            node_locations: self.locations,
            node_artifact_spans: self.artifact_spans,
        })
    }

    fn visit_node(&mut self, index: usize) -> Result<(), LuaWorkflowError> {
        if self
            .nodes
            .iter()
            .any(|node| self.names.get(&index) == Some(&node.name))
        {
            return Ok(());
        }
        let draft = self
            .host
            .nodes
            .get(index)
            .cloned()
            .ok_or_else(|| build_error("WFR001", "node handle does not exist", None))?;
        let name = self.names.get(&index).cloned().ok_or_else(|| {
            build_error(
                "WFS006",
                "node name was not assigned",
                Some(draft.location.clone()),
            )
        })?;
        let child_indices = match &draft.kind {
            NodeDraftKind::Fanout { children, .. } | NodeDraftKind::Sequence { children, .. } => {
                children.clone()
            }
            _ => Vec::new(),
        };
        for (position, child_index) in child_indices.iter().enumerate() {
            let child = self.host.children.get(*child_index).ok_or_else(|| {
                build_error(
                    "WFR001",
                    "child handle does not exist",
                    Some(draft.location.clone()),
                )
            })?;
            if !self.child_uses.insert(child.node) {
                return Err(build_error(
                    "WFC007",
                    "the same Node value cannot be used by multiple children",
                    Some(child.location.clone()),
                ));
            }
            self.assign_node_name(child.node, &name, position, child.location.clone())?;
        }
        if let Some(delegate) = &draft.delegate {
            self.assign_node_name(delegate.child, &name, 0, delegate.location.clone())?;
        }
        let artifact = draft
            .artifact
            .map(|schema| self.register_schema(schema))
            .transpose()?;
        let input = draft
            .input
            .iter()
            .map(|input| self.build_input(*input))
            .collect::<Result<Vec<_>, _>>()?;
        let kind = match draft.kind {
            NodeDraftKind::Command { command, env } => NodeKind::Command(CommandSpec {
                command,
                env: self.build_command_env(&env)?,
            }),
            NodeDraftKind::Session {
                provider,
                model,
                permission,
                facets,
            } => NodeKind::Session(SessionSpec {
                provider,
                model,
                permission,
                facets,
            }),
            NodeDraftKind::Fanout { children, items } => NodeKind::Fanout(FanoutSpec {
                children: self.build_children(index, &children, true)?,
                items: items
                    .map(|value| self.build_fanout_items(index, value))
                    .transpose()?,
            }),
            NodeDraftKind::Sequence { children, entry } => {
                let child_nodes = children
                    .iter()
                    .map(|child| self.host.children[*child].node)
                    .collect::<HashSet<_>>();
                let entry =
                    self.optional_child_name(entry, &child_nodes, &draft.location, "entry")?;
                NodeKind::Sequence(SequenceSpec {
                    entry,
                    children: self.build_children(index, &children, false)?,
                })
            }
        };
        if artifact.is_some() {
            if let Some(mut span) = self
                .host
                .artifact_spans
                .get(&draft.location.source)
                .and_then(|spans| spans.node_span(draft.location.line, &name))
            {
                span.source = Some(draft.location.source.clone());
                self.artifact_spans.insert(name.clone(), span);
            }
        }
        let mut completion = draft.completion;
        if let Some(delegate) = &draft.delegate {
            let scope = HashSet::from([index]);
            let owner_inputs = draft.input.iter().copied().collect();
            completion.delegate = Some(SessionDelegate {
                child: self.names[&delegate.child].clone(),
                inputs: delegate
                    .inputs
                    .iter()
                    .map(|(parameter, source)| {
                        self.source_ref(*source, &scope, &owner_inputs, false, &delegate.location)
                            .map(|raw| {
                                let source = if matches!(self.host.sources[*source], SourceDraft::Node { node, .. } if node == index) {
                                    InputSourceRef::node_artifact(raw)
                                } else {
                                    InputSourceRef::new(raw)
                                };
                                (parameter.clone(), source)
                            })
                    })
                    .collect::<Result<_, _>>()?,
                when: self.build_rule_predicate(&delegate.when, index, &delegate.location)?,
                max_iterations: delegate.max_iterations,
            });
        }
        self.locations.insert(name.clone(), draft.location);
        self.nodes.push(NodeDefinition {
            name,
            kind,
            artifact,
            input,
            completion,
            worktree: draft.worktree,
        });
        for child_index in &child_indices {
            let node = self.host.children[*child_index].node;
            self.visit_node(node)?;
        }
        if let Some(delegate) = draft.delegate {
            self.visit_node(delegate.child)?;
        }
        Ok(())
    }

    fn assign_node_name(
        &mut self,
        index: usize,
        owner: &str,
        position: usize,
        location: LuaSourceLocation,
    ) -> Result<(), LuaWorkflowError> {
        if self.names.contains_key(&index) {
            return Ok(());
        }
        let node =
            self.host.nodes.get(index).ok_or_else(|| {
                build_error("WFR001", "child node does not exist", Some(location))
            })?;
        let name = match &node.name {
            Some(explicit) => self.namespace.register_explicit(explicit.clone()),
            None => self.namespace.register_synthesized(owner, position),
        }
        .map_err(|error| {
            let code = match &error {
                NodeNamespaceError::Reserved(_) => "WFR004",
                NodeNamespaceError::Duplicate(_) => "WFS006",
            };
            build_error(code, error.to_string(), Some(node.location.clone()))
        })?;
        self.names.insert(index, name);
        Ok(())
    }

    fn build_input(&mut self, input_index: usize) -> Result<InputParam, LuaWorkflowError> {
        let input = self
            .host
            .inputs
            .get(input_index)
            .cloned()
            .ok_or_else(|| build_error("WFS002", "input handle does not exist", None))?;
        let contract = input
            .contract
            .map(|schema| self.register_schema(schema))
            .transpose()?;
        Ok(InputParam {
            name: input.name,
            contract,
        })
    }

    fn register_schema(&mut self, index: usize) -> Result<String, LuaWorkflowError> {
        if let Some(name) = self.schema_names.get(&index) {
            return Ok(name.clone());
        }
        let draft = self
            .host
            .schemas
            .get(index)
            .cloned()
            .ok_or_else(|| build_error("WFS002", "schema handle does not exist", None))?;
        let name = draft
            .name
            .clone()
            .unwrap_or_else(|| format!("schema-{index}"));
        if self.schemas.contains_key(&name)
            || self.schema_names.values().any(|value| value == &name)
        {
            return Err(build_error(
                "WFS006",
                format!("schema name '{name}' is duplicated"),
                None,
            ));
        }
        self.schema_names.insert(index, name.clone());
        let definition = match draft.kind {
            SchemaDraftKind::Object {
                properties,
                required,
            } => {
                let mut mapped = BTreeMap::new();
                for (property, schema) in properties {
                    mapped.insert(property, self.inline_schema(schema)?);
                }
                SchemaDef::Object {
                    properties: mapped,
                    required,
                }
            }
            SchemaDraftKind::Array { items } => SchemaDef::Array {
                items: self.register_schema(items)?,
            },
            SchemaDraftKind::String { values } => SchemaDef::String { r#enum: values },
            SchemaDraftKind::Boolean => SchemaDef::Boolean,
            SchemaDraftKind::Integer => SchemaDef::Integer,
            SchemaDraftKind::Number => SchemaDef::Number,
        };
        self.schemas.insert(name.clone(), definition);
        Ok(name)
    }

    fn inline_schema(&mut self, index: usize) -> Result<SchemaDef, LuaWorkflowError> {
        let draft = self
            .host
            .schemas
            .get(index)
            .cloned()
            .ok_or_else(|| build_error("WFS002", "schema handle does not exist", None))?;
        if draft.name.is_some() {
            let name = self.register_schema(index)?;
            return Ok(self.schemas[&name].clone());
        }
        match draft.kind {
            SchemaDraftKind::Object {
                properties,
                required,
            } => {
                let mut mapped = BTreeMap::new();
                for (property, schema) in properties {
                    mapped.insert(property, self.inline_schema(schema)?);
                }
                Ok(SchemaDef::Object {
                    properties: mapped,
                    required,
                })
            }
            SchemaDraftKind::Array { .. } => {
                let name = self.register_schema(index)?;
                Ok(self.schemas[&name].clone())
            }
            SchemaDraftKind::String { values } => Ok(SchemaDef::String { r#enum: values }),
            SchemaDraftKind::Boolean => Ok(SchemaDef::Boolean),
            SchemaDraftKind::Integer => Ok(SchemaDef::Integer),
            SchemaDraftKind::Number => Ok(SchemaDef::Number),
        }
    }

    fn optional_child_name(
        &self,
        node: Option<usize>,
        children: &HashSet<usize>,
        location: &LuaSourceLocation,
        field: &str,
    ) -> Result<Option<String>, LuaWorkflowError> {
        let Some(node) = node else {
            return Ok(None);
        };
        if !children.contains(&node) {
            return Err(build_error(
                "WFR001",
                format!("sequence {field} must be one of its children"),
                Some(location.clone()),
            ));
        }
        Ok(self.names.get(&node).cloned())
    }

    fn build_children(
        &self,
        owner: usize,
        children: &[usize],
        fanout: bool,
    ) -> Result<Vec<ChildEntry>, LuaWorkflowError> {
        let scope = children
            .iter()
            .map(|child| self.host.children[*child].node)
            .collect::<HashSet<_>>();
        let owner_inputs = self.host.nodes[owner]
            .input
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        children
            .iter()
            .map(|child_index| {
                let child = &self.host.children[*child_index];
                let inputs = child
                    .inputs
                    .iter()
                    .map(|(parameter, source)| {
                        self.source_ref(*source, &scope, &owner_inputs, fanout, &child.location)
                            .map(|source| (parameter.clone(), InputSourceRef::new(source)))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let rules = child
                    .rules
                    .as_ref()
                    .map(|rules| {
                        rules
                            .iter()
                            .map(|rule| self.build_rule(*rule, child.node, &scope, &child.location))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?;
                Ok(ChildEntry {
                    name: self.names[&child.node].clone(),
                    inputs,
                    rules,
                })
            })
            .collect()
    }

    fn build_command_env(
        &self,
        env: &[(EnvironmentVariableName, usize)],
    ) -> Result<BTreeMap<EnvironmentVariableName, InputParameterRef>, LuaWorkflowError> {
        env.iter()
            .map(|(variable, source)| match self.host.sources.get(*source) {
                Some(SourceDraft::Input {
                    input,
                    path,
                    location,
                }) => {
                    self.host.mark_source_consumed(*source);
                    let mut reference = self.host.inputs[*input].name.clone();
                    if !self.host.source_paths.is_root(*path) {
                        reference.push('.');
                        reference.push_str(&self.host.source_paths.fields(*path).join("."));
                    }
                    InputParameterRef::new(&reference)
                        .map(|reference| (variable.clone(), reference))
                        .map_err(|message| build_error("WFR003", message, Some(location.clone())))
                }
                _ => Err(build_error(
                    "WFS002",
                    "command env values must be ReleashInput values",
                    None,
                )),
            })
            .collect()
    }

    fn source_ref(
        &self,
        source: usize,
        scope: &HashSet<usize>,
        owner_inputs: &HashSet<usize>,
        fanout: bool,
        location: &LuaSourceLocation,
    ) -> Result<String, LuaWorkflowError> {
        match self.host.sources.get(source) {
            Some(SourceDraft::Node { node, path, .. }) if !fanout && scope.contains(node) => {
                self.host.mark_source_consumed(source);
                let mut raw = self.names[node].clone();
                if !self.host.source_paths.is_root(*path) {
                    raw.push('.');
                    raw.push_str(&self.host.source_paths.fields(*path).join("."));
                }
                Ok(raw)
            }
            Some(SourceDraft::Input {
                input,
                path,
                location: source_location,
            }) if !self.host.source_paths.is_root(*path)
                && self.host.inputs[*input].contract.is_none() =>
            {
                Err(build_error(
                    "WFR003",
                    "input does not declare a contract",
                    Some(source_location.clone()),
                ))
            }
            Some(SourceDraft::Input {
                input,
                path,
                location: _,
            }) if owner_inputs.contains(input) => {
                self.host.mark_source_consumed(source);
                let mut raw = self.host.inputs[*input].name.clone();
                if !self.host.source_paths.is_root(*path) {
                    raw.push('.');
                    raw.push_str(&self.host.source_paths.fields(*path).join("."));
                }
                Ok(raw)
            }
            Some(SourceDraft::Request) => {
                self.host.mark_source_consumed(source);
                Ok("request".to_string())
            }
            Some(SourceDraft::Items) if fanout => {
                self.host.mark_source_consumed(source);
                Ok("items".to_string())
            }
            Some(_) => Err(build_error(
                "WFR007",
                "input source is outside the composite node scope",
                Some(location.clone()),
            )),
            None => Err(build_error(
                "WFR007",
                "input source does not exist",
                Some(location.clone()),
            )),
        }
    }

    fn build_rule(
        &self,
        rule: usize,
        child_node: usize,
        scope: &HashSet<usize>,
        location: &LuaSourceLocation,
    ) -> Result<Rule, LuaWorkflowError> {
        let target = |node: &usize| {
            if !scope.contains(node) {
                return Err(build_error(
                    "WFR001",
                    "rule target must be a sibling child",
                    Some(location.clone()),
                ));
            }
            Ok(self.names[node].clone())
        };
        match self.host.rules.get(rule) {
            Some(RuleDraft::Next(node)) => Ok(Rule::Next(target(node)?)),
            Some(RuleDraft::When { on, on_true, next }) => Ok(Rule::When {
                on: self.build_rule_predicate(on, child_node, location)?,
                then: target(on_true)?,
                next: target(next)?,
            }),
            Some(RuleDraft::Switch { on, cases, next }) => Ok(Rule::Switch {
                on: self.rule_field(*on, child_node, location)?,
                cases: cases
                    .iter()
                    .map(|(value, node)| target(node).map(|name| (value.clone(), name)))
                    .collect::<Result<_, _>>()?,
                next: next.as_ref().map(target).transpose()?,
            }),
            Some(RuleDraft::LoopGuard {
                max_iterations,
                on_exhausted,
            }) => Ok(Rule::LoopGuard {
                max_iterations: *max_iterations,
                on_exhausted: target(on_exhausted)?,
            }),
            None => Err(build_error(
                "WFS002",
                "rule handle does not exist",
                Some(location.clone()),
            )),
        }
    }

    fn build_rule_predicate(
        &self,
        predicate: &Predicate<usize>,
        child_node: usize,
        location: &LuaSourceLocation,
    ) -> Result<Predicate<String>, LuaWorkflowError> {
        match predicate {
            Predicate::Ref(source) => self
                .rule_field(*source, child_node, location)
                .map(Predicate::Ref),
            Predicate::And(elements) => elements
                .iter()
                .map(|element| self.build_rule_predicate(element, child_node, location))
                .collect::<Result<_, _>>()
                .map(Predicate::And),
            Predicate::Or(elements) => elements
                .iter()
                .map(|element| self.build_rule_predicate(element, child_node, location))
                .collect::<Result<_, _>>()
                .map(Predicate::Or),
        }
    }

    fn rule_field(
        &self,
        source: usize,
        child_node: usize,
        location: &LuaSourceLocation,
    ) -> Result<String, LuaWorkflowError> {
        match self.host.sources.get(source) {
            Some(SourceDraft::Node { node, path, .. })
                if *node == child_node && !self.host.source_paths.is_root(*path) =>
            {
                self.host.mark_source_consumed(source);
                Ok(self.host.source_paths.fields(*path).join("."))
            }
            Some(SourceDraft::Input {
                input,
                path,
                location: source_location,
            }) if !self.host.source_paths.is_root(*path)
                && self.host.inputs[*input].contract.is_none() =>
            {
                Err(build_error(
                    "WFR003",
                    "input does not declare a contract",
                    Some(source_location.clone()),
                ))
            }
            _ => Err(build_error(
                "WFR003",
                "rule discriminator must reference the current child artifact field",
                Some(location.clone()),
            )),
        }
    }

    fn build_fanout_items(
        &self,
        owner: usize,
        items: FanoutItemsDraft,
    ) -> Result<ItemsSource, LuaWorkflowError> {
        match items {
            FanoutItemsDraft::Literal(values) => Ok(ItemsSource::Literal(values)),
            FanoutItemsDraft::Source(source) => match self.host.sources.get(source) {
                Some(SourceDraft::Node { node, path, .. })
                    if !self.host.source_paths.is_root(*path) =>
                {
                    self.host.mark_source_consumed(source);
                    let node_name =
                        self.names.get(node).cloned().unwrap_or_else(|| {
                            self.host.nodes[*node].name.clone().unwrap_or_default()
                        });
                    let field_path = crate::domain::workflow::FieldPath::new(
                        self.host.source_paths.fields(*path),
                    );
                    field_path.to_reference("source").map_err(|_| {
                        build_error("WFR003", "invalid fanout items field path", None)
                    })?;
                    Ok(ItemsSource::ArtifactField {
                        node: node_name,
                        field_path,
                    })
                }
                Some(SourceDraft::Input { input, .. })
                    if self.host.nodes[owner].input.contains(input) =>
                {
                    Err(build_error(
                        "WFR003",
                        "fanout items cannot use an input parameter directly",
                        None,
                    ))
                }
                _ => Err(build_error(
                    "WFR003",
                    "fanout items must reference an artifact field",
                    None,
                )),
            },
        }
    }

    fn validate_unconsumed_sources(
        &self,
        workflow: &WorkflowDefinition,
    ) -> Result<(), LuaWorkflowError> {
        for source in &self.host.sources {
            if self.host.source_paths.contains(source.path()) {
                continue;
            }
            match source {
                SourceDraft::Node {
                    node,
                    path,
                    location,
                } if !self.host.source_paths.is_root(*path) => {
                    let Some(node) = self
                        .names
                        .get(node)
                        .and_then(|name| workflow.node_by_name(name))
                    else {
                        continue;
                    };
                    let fields = self.host.source_paths.fields(*path);
                    reference::resolve_node_field_path(
                        workflow,
                        node,
                        &crate::domain::workflow::FieldPath::new(fields.iter().cloned()),
                    )
                    .map_err(|error| {
                        let message = match error {
                            reference::NodeFieldPathError::NoReferenceableArtifact => {
                                "node does not declare an artifact".to_string()
                            }
                            reference::NodeFieldPathError::ArtifactNotObject => format!(
                                "artifact field '{}' cannot be read from a non-object schema",
                                fields[0]
                            ),
                            reference::NodeFieldPathError::Segment(error) => match error.kind {
                                contract_schema::FieldPathResolutionErrorKind::NonObject => {
                                    format!(
                                    "artifact field '{}' cannot be read from a non-object schema",
                                    error.segment
                                )
                                }
                                contract_schema::FieldPathResolutionErrorKind::MissingProperty => {
                                    format!("artifact field '{}' does not exist", error.segment)
                                }
                            },
                        };
                        build_error("WFR003", message, Some(location.clone()))
                    })?;
                }
                SourceDraft::Input {
                    input,
                    path,
                    location,
                } if !self.host.source_paths.is_root(*path) => {
                    if let Some(schema) = self.host.inputs[*input].contract {
                        self.host
                            .validate_schema_path(
                                schema,
                                &self.host.source_paths.fields(*path),
                                "input",
                            )
                            .map_err(|message| {
                                build_error("WFR003", message, Some(location.clone()))
                            })?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

pub fn handle(kind: &str, index: usize) -> LuaData {
    LuaData::Handle(LuaHostHandle {
        kind: kind.to_string(),
        index,
    })
}

fn push_node(nodes: &mut Vec<NodeDraft>, draft: NodeDraft) -> LuaData {
    let index = nodes.len();
    nodes.push(draft);
    handle(HANDLE_NODE, index)
}

fn push_rule(rules: &mut Vec<RuleDraft>, draft: RuleDraft) -> LuaData {
    let index = rules.len();
    rules.push(draft);
    handle(HANDLE_RULE, index)
}

fn expect_handle_data(value: &LuaData, kind: &str) -> Result<usize, String> {
    expect_handle(value, kind).map_err(|_| format!("Lua chunk must return a {kind} value"))
}

fn expect_handle(value: &LuaData, kind: &str) -> Result<usize, ()> {
    match value {
        LuaData::Handle(handle) if handle.kind == kind => Ok(handle.index),
        _ => Err(()),
    }
}

fn one_table(
    arguments: Vec<LuaData>,
    location: &LuaSourceLocation,
) -> Result<LuaTableData, LuaHostError> {
    match arguments.as_slice() {
        [LuaData::Table(table)] => Ok(table.clone()),
        _ => Err(host_error(
            "WFS002",
            "builder expects exactly one table argument",
            location.clone(),
        )),
    }
}

fn one_handle(
    arguments: Vec<LuaData>,
    kind: &str,
    location: &LuaSourceLocation,
) -> Result<usize, LuaHostError> {
    match arguments.as_slice() {
        [value] => expect_handle(value, kind).map_err(|_| type_error("argument", kind, location)),
        _ => Err(host_error(
            "WFS002",
            "builder expects exactly one argument",
            location.clone(),
        )),
    }
}

fn reject_unknown(
    table: &LuaTableData,
    known: &[&str],
    location: &LuaSourceLocation,
) -> Result<(), LuaHostError> {
    for key in table.string_keys() {
        if !known.contains(&key) {
            return Err(host_error(
                "WFS002",
                format!("unknown field '{key}'"),
                location.clone(),
            ));
        }
    }
    if table
        .entries
        .keys()
        .any(|key| !matches!(key, LuaTableKey::String(_)))
    {
        return Err(host_error(
            "WFS002",
            "builder table keys must be strings",
            location.clone(),
        ));
    }
    Ok(())
}

fn required_string(
    table: &LuaTableData,
    field: &str,
    location: &LuaSourceLocation,
) -> Result<String, LuaHostError> {
    match table.get_string(field) {
        Some(LuaData::String(value)) => Ok(value.clone()),
        None | Some(LuaData::Nil) => Err(missing_field(field, location)),
        Some(_) => Err(type_error(field, "string", location)),
    }
}

fn optional_string(
    table: &LuaTableData,
    field: &str,
    location: &LuaSourceLocation,
) -> Result<Option<String>, LuaHostError> {
    match table.get_string(field) {
        None | Some(LuaData::Nil) => Ok(None),
        Some(LuaData::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(type_error(field, "string", location)),
    }
}

fn required_u32(
    table: &LuaTableData,
    field: &str,
    location: &LuaSourceLocation,
) -> Result<u32, LuaHostError> {
    match table.get_string(field) {
        Some(LuaData::Integer(value)) => {
            u32::try_from(*value).map_err(|_| type_error(field, "u32", location))
        }
        None | Some(LuaData::Nil) => Err(missing_field(field, location)),
        Some(_) => Err(type_error(field, "u32", location)),
    }
}

fn required_table<'a>(
    table: &'a LuaTableData,
    field: &str,
    location: &LuaSourceLocation,
) -> Result<&'a LuaTableData, LuaHostError> {
    match table.get_string(field) {
        Some(LuaData::Table(value)) => Ok(value),
        None | Some(LuaData::Nil) => Err(missing_field(field, location)),
        Some(_) => Err(type_error(field, "table", location)),
    }
}

fn required_handle(
    table: &LuaTableData,
    field: &str,
    kind: &str,
    location: &LuaSourceLocation,
) -> Result<usize, LuaHostError> {
    match table.get_string(field) {
        Some(value) => expect_handle(value, kind).map_err(|_| type_error(field, kind, location)),
        None => Err(missing_field(field, location)),
    }
}

fn optional_handle(
    table: &LuaTableData,
    field: &str,
    kind: &str,
    location: &LuaSourceLocation,
) -> Result<Option<usize>, LuaHostError> {
    match table.get_string(field) {
        None | Some(LuaData::Nil) => Ok(None),
        Some(value) => expect_handle(value, kind)
            .map(Some)
            .map_err(|_| type_error(field, kind, location)),
    }
}

fn required_handle_array(
    table: &LuaTableData,
    field: &str,
    kind: &str,
    location: &LuaSourceLocation,
) -> Result<Vec<usize>, LuaHostError> {
    optional_handle_array(table, field, kind, location)?
        .ok_or_else(|| missing_field(field, location))
}

fn optional_handle_array(
    table: &LuaTableData,
    field: &str,
    kind: &str,
    location: &LuaSourceLocation,
) -> Result<Option<Vec<usize>>, LuaHostError> {
    match table.get_string(field) {
        None | Some(LuaData::Nil) => Ok(None),
        Some(LuaData::Table(values)) => values
            .as_array()
            .ok_or_else(|| type_error(field, "array", location))?
            .into_iter()
            .map(|value| expect_handle(value, kind).map_err(|_| type_error(field, kind, location)))
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(type_error(field, "array", location)),
    }
}

fn optional_string_array(
    table: &LuaTableData,
    field: &str,
    location: &LuaSourceLocation,
) -> Result<Option<Vec<String>>, LuaHostError> {
    match table.get_string(field) {
        None | Some(LuaData::Nil) => Ok(None),
        Some(LuaData::Table(values)) => values
            .as_array()
            .ok_or_else(|| type_error(field, "string array", location))?
            .into_iter()
            .map(|value| match value {
                LuaData::String(value) => Ok(value.clone()),
                _ => Err(type_error(field, "string array", location)),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(type_error(field, "string array", location)),
    }
}

fn parse_worktree(
    table: &LuaTableData,
    location: &LuaSourceLocation,
) -> Result<Option<crate::domain::workflow::WorktreeMode>, LuaHostError> {
    use crate::domain::workflow::WorktreeMode;
    match table.get_string("worktree") {
        None | Some(LuaData::Nil) => Ok(None),
        Some(value) if expect_handle(value, HANDLE_WORKTREE) == Ok(0) => {
            Ok(Some(WorktreeMode::Shared))
        }
        Some(value) if expect_handle(value, HANDLE_WORKTREE) == Ok(1) => {
            Ok(Some(WorktreeMode::Isolated))
        }
        _ => Err(type_error(
            "worktree",
            "r.worktree.shared or r.worktree.isolated",
            location,
        )),
    }
}

fn parse_completion(
    table: &LuaTableData,
    location: &LuaSourceLocation,
) -> Result<NodeCompletion, LuaHostError> {
    use super::completion_wire::CompletionShapeError;

    let error = |error: CompletionShapeError| {
        host_field_error("WFS002", error.to_string(), location.clone(), "completion")
    };
    let completion = match table.get_string("completion") {
        None | Some(LuaData::Nil) => return Ok(NodeCompletion::default()),
        Some(LuaData::Table(completion)) => completion,
        Some(_) => return Err(error(CompletionShapeError::ExpectedMap)),
    };
    if completion.entries.is_empty() {
        return Err(error(CompletionShapeError::Empty));
    }
    if completion.as_array().is_some() {
        return Err(error(CompletionShapeError::ExpectedMap));
    }
    if completion.entries.len() != 1 || completion.get_string("require").is_none() {
        return Err(host_field_error(
            "WFS002",
            "completion map only accepts the key 'require'",
            location.clone(),
            "completion",
        ));
    }
    match completion.get_string("require") {
        Some(value) if expect_handle(value, HANDLE_COMPLETION) == Ok(0) => {
            Ok(NodeCompletion::require_approval())
        }
        _ => Err(error(CompletionShapeError::InvalidRequirement)),
    }
}

fn lua_array_to_json(
    table: &LuaTableData,
    location: &LuaSourceLocation,
) -> Result<Vec<Value>, LuaHostError> {
    table
        .as_array()
        .ok_or_else(|| type_error("items", "literal array", location))?
        .into_iter()
        .map(|value| lua_to_json(value, location))
        .collect()
}

fn lua_to_json(value: &LuaData, location: &LuaSourceLocation) -> Result<Value, LuaHostError> {
    match value {
        LuaData::Nil => Ok(Value::Null),
        LuaData::Boolean(value) => Ok(Value::Bool(*value)),
        LuaData::Integer(value) => Ok(Value::Number((*value).into())),
        LuaData::Number(value) => Number::from_f64(*value)
            .map(Value::Number)
            .ok_or_else(|| type_error("items", "finite JSON value", location)),
        LuaData::String(value) => Ok(Value::String(value.clone())),
        LuaData::Table(table) => {
            if let Some(array) = table.as_array() {
                return array
                    .into_iter()
                    .map(|value| lua_to_json(value, location))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array);
            }
            let mut object = serde_json::Map::new();
            for (key, value) in &table.entries {
                let LuaTableKey::String(key) = key else {
                    return Err(type_error("items", "JSON-compatible value", location));
                };
                object.insert(key.clone(), lua_to_json(value, location)?);
            }
            Ok(Value::Object(object))
        }
        LuaData::Handle(_) | LuaData::BoundFunction { .. } => {
            Err(type_error("items", "JSON-compatible value", location))
        }
    }
}

fn lua_key_as_case(key: &LuaTableKey) -> Option<String> {
    match key {
        LuaTableKey::Boolean(value) => Some(value.to_string()),
        LuaTableKey::Integer(value) => Some(value.to_string()),
        LuaTableKey::String(value) => Some(value.clone()),
    }
}

fn missing_field(field: &str, location: &LuaSourceLocation) -> LuaHostError {
    host_field_error(
        "WFS002",
        format!("missing required field '{field}'"),
        location.clone(),
        field,
    )
}

fn type_error(field: &str, expected: &str, location: &LuaSourceLocation) -> LuaHostError {
    host_field_error(
        "WFS002",
        format!("field '{field}' must be {expected}"),
        location.clone(),
        field,
    )
}

fn host_error(code: &str, message: impl Into<String>, location: LuaSourceLocation) -> LuaHostError {
    LuaHostError {
        category: code.to_string(),
        message: message.into(),
        location: Some(location),
        field: None,
    }
}

fn host_field_error(
    code: &str,
    message: impl Into<String>,
    location: LuaSourceLocation,
    field: &str,
) -> LuaHostError {
    LuaHostError {
        category: code.to_string(),
        message: message.into(),
        location: Some(location),
        field: Some(field.to_string()),
    }
}

fn build_error(
    code: &str,
    message: impl Into<String>,
    location: Option<LuaSourceLocation>,
) -> LuaWorkflowError {
    LuaWorkflowError {
        code: code.to_string(),
        message: message.into(),
        location,
        field: None,
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_tests;

#[cfg(feature = "test-support")]
impl WorkflowLuaHost {
    pub fn test_predicate_entries(&self) -> usize {
        self.predicate_entries
    }
    pub fn test_predicates(&self) -> &Vec<(Predicate<usize>, usize)> {
        &self.predicates
    }
    pub fn test_rules(&self) -> &Vec<RuleDraft> {
        &self.rules
    }
    pub fn test_sources(&self) -> &Vec<SourceDraft> {
        &self.sources
    }
}

#[cfg(feature = "test-support")]
impl WorkflowLuaHost {
    pub fn test_predicate_entries_mut(&mut self) -> &mut usize {
        &mut self.predicate_entries
    }
}

#[cfg(feature = "test-support")]
impl WorkflowLuaHost {
    pub fn test_child_inputs(&self, index: usize) -> &[(String, usize)] {
        &self.children[index].inputs
    }
    pub fn test_source_fields(&self, path: usize) -> Vec<String> {
        self.source_paths.fields(path)
    }
}

#[cfg(feature = "test-support")]
pub fn test_handle_index(value: &LuaData, kind: &str) -> Option<usize> {
    expect_handle(value, kind).ok()
}
