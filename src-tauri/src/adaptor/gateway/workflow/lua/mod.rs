use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use serde_json::{Number, Value};

use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::value_objects::{
    ChildEntry, CommandSpec, FacetRefs, FanoutSpec, InputParam, InputSourceRef, ItemsSource,
    NodeCompletion, NodeDefinition, NodeKind, NodeNamespace, OnFailure, Rule, SchemaDef,
    SequenceSpec, SessionSpec, WorkflowDefinition, MAIN_ENTRY_NODE_NAME,
};
use crate::infrastructure::lua::{
    evaluate, LuaData, LuaEvaluationRequest, LuaFailure, LuaHost, LuaHostError, LuaHostHandle,
    LuaLimits, LuaModule, LuaModuleValue, LuaSourceLocation, LuaTableData, LuaTableKey,
};

mod stubs;

pub(crate) use stubs::generate_editor_support;

/// 一度の評価で Lua 側から生成できる中間ハンドルの総数。Lua VM のメモリ上限は
/// Rust 側の arena を数えないため、ここで別途有界にする。
const MAX_HOST_ARENA_ENTRIES: usize = 100_000;

const HANDLE_NODE: &str = "node";
const HANDLE_CHILD: &str = "child";
const HANDLE_RULE: &str = "rule";
const HANDLE_FAILURE: &str = "on_failure";
const HANDLE_INPUT: &str = "input";
const HANDLE_SOURCE: &str = "source";
const HANDLE_SCHEMA: &str = "schema";
const HANDLE_FACET: &str = "facet";
const HANDLE_FACET_INDEX: &str = "facet_index";
const HANDLE_WORKFLOW: &str = "workflow";
const HANDLE_PROVIDER: &str = "provider";
const HANDLE_COMPLETION: &str = "completion";

const FN_COMMAND: u32 = 1;
const FN_SESSION: u32 = 2;
const FN_FANOUT: u32 = 3;
const FN_SEQUENCE: u32 = 4;
const FN_CHILD: u32 = 5;
const FN_NEXT: u32 = 6;
const FN_WHEN: u32 = 7;
const FN_SWITCH: u32 = 8;
const FN_LOOP_GUARD: u32 = 9;
const FN_RETRY: u32 = 10;
const FN_INPUT: u32 = 11;
const FN_SCHEMA_OBJECT: u32 = 12;
const FN_SCHEMA_ARRAY: u32 = 13;
const FN_SCHEMA_STRING: u32 = 14;
const FN_SCHEMA_BOOLEAN: u32 = 15;
const FN_SCHEMA_INTEGER: u32 = 16;
const FN_SCHEMA_NUMBER: u32 = 17;
const FN_WORKFLOW: u32 = 18;

#[derive(Debug, Clone, Default)]
pub(crate) struct LuaFacetCatalog {
    pub(crate) instruction: Vec<String>,
    pub(crate) policy: Vec<String>,
    pub(crate) knowledge: Vec<String>,
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
pub(crate) struct LuaWorkflowDefinition {
    pub(crate) workflow: WorkflowDefinition,
    pub(crate) node_locations: BTreeMap<String, LuaSourceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LuaWorkflowError {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) location: Option<LuaSourceLocation>,
}

impl std::fmt::Display for LuaWorkflowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LuaWorkflowError {}

pub(crate) fn load_lua_workflow(
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

fn load_lua_workflow_with_limits(
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
    }
}

#[derive(Debug, Clone)]
struct NodeDraft {
    name: Option<String>,
    kind: NodeDraftKind,
    artifact: Option<usize>,
    input: Vec<usize>,
    completion: NodeCompletion,
    location: LuaSourceLocation,
}

#[derive(Debug, Clone)]
enum NodeDraftKind {
    Command(String),
    Session {
        provider: ProviderKind,
        model: Option<String>,
        permission: Option<String>,
        facets: FacetRefs,
    },
    Fanout {
        children: Vec<usize>,
        items: Option<FanoutItemsDraft>,
    },
    Sequence {
        children: Vec<usize>,
        entry: Option<usize>,
        output: Option<usize>,
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
    on_failure: Option<OnFailure>,
    location: LuaSourceLocation,
}

#[derive(Debug, Clone)]
enum RuleDraft {
    Next(usize),
    When {
        on: usize,
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
enum SourceDraft {
    Node { node: usize, fields: Vec<String> },
    Input { input: usize, fields: Vec<String> },
    Request,
    Items,
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
struct WorkflowLuaHost {
    nodes: Vec<NodeDraft>,
    children: Vec<ChildDraft>,
    rules: Vec<RuleDraft>,
    failures: Vec<OnFailure>,
    inputs: Vec<InputDraft>,
    sources: Vec<SourceDraft>,
    schemas: Vec<SchemaDraft>,
    facets: Vec<FacetDraft>,
    workflows: Vec<WorkflowDraft>,
    facet_module: LuaModule,
    facet_keys: [BTreeSet<String>; 3],
    request_source: usize,
    items_source: usize,
}

impl WorkflowLuaHost {
    fn new(catalog: LuaFacetCatalog) -> Self {
        let mut host = Self {
            nodes: Vec::new(),
            children: Vec::new(),
            rules: Vec::new(),
            failures: vec![OnFailure::Ignore],
            inputs: Vec::new(),
            sources: vec![SourceDraft::Request, SourceDraft::Items],
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
            ("switch", FN_SWITCH),
            ("loop_guard", FN_LOOP_GUARD),
            ("retry", FN_RETRY),
            ("input", FN_INPUT),
            ("workflow", FN_WORKFLOW),
        ];
        let mut members = BTreeMap::new();
        for (name, function) in functions {
            members.insert(name.to_string(), LuaModuleValue::Function(function));
        }
        members.insert(
            "ignore".to_string(),
            LuaModuleValue::Data(handle(HANDLE_FAILURE, 0)),
        );
        members.insert(
            "request".to_string(),
            LuaModuleValue::Data(handle(HANDLE_SOURCE, self.request_source)),
        );
        members.insert(
            "items".to_string(),
            LuaModuleValue::Data(handle(HANDLE_SOURCE, self.items_source)),
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
    fn arena_entries(&self) -> usize {
        self.nodes.len()
            + self.children.len()
            + self.rules.len()
            + self.failures.len()
            + self.inputs.len()
            + self.sources.len()
            + self.schemas.len()
            + self.facets.len()
            + self.workflows.len()
    }

    /// Lua VM のメモリ上限は Rust 側の arena を数えないため、ビルダー呼び出しの
    /// 入口で総数を有界にする。`MAX_NODES_PER_WORKFLOW` に収まる定義は到達しない。
    fn ensure_arena_budget(&self, location: &LuaSourceLocation) -> Result<(), LuaHostError> {
        if self.arena_entries() >= MAX_HOST_ARENA_ENTRIES {
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
        self.ensure_arena_budget(&location)?;
        match function {
            FN_COMMAND => self.call_command(arguments, location),
            FN_SESSION => self.call_session(arguments, location),
            FN_FANOUT => self.call_fanout(arguments, location),
            FN_SEQUENCE => self.call_sequence(arguments, location),
            FN_CHILD => self.call_child(arguments, location),
            FN_NEXT => self.call_next(arguments, location),
            FN_WHEN => self.call_when(arguments, location),
            FN_SWITCH => self.call_switch(arguments, location),
            FN_LOOP_GUARD => self.call_loop_guard(arguments, location),
            FN_RETRY => self.call_retry(arguments, location),
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
        self.ensure_arena_budget(&location)?;
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
        let (node, mut fields) = match handle.kind.as_str() {
            HANDLE_NODE => (handle.index, Vec::new()),
            HANDLE_INPUT => {
                return self.index_input(handle.index, Vec::new(), key, location);
            }
            HANDLE_SOURCE => match self.sources.get(handle.index) {
                Some(SourceDraft::Input { input, fields }) => {
                    return self.index_input(*input, fields.clone(), key, location);
                }
                Some(SourceDraft::Node { node, fields }) if fields.is_empty() => {
                    (*node, fields.clone())
                }
                Some(SourceDraft::Node { .. }) => {
                    return Err(host_error(
                        "WFR003",
                        "nested artifact field references are not supported",
                        location,
                    ));
                }
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
        fields.push(key.to_string());
        self.validate_artifact_path(node, &fields)
            .map_err(|message| host_error("WFR003", message, location.clone()))?;
        Ok(self.push_source(SourceDraft::Node { node, fields }))
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
            &["name", "command", "artifact", "input", "completion"],
            &location,
        )?;
        let draft = NodeDraft {
            name: optional_string(&table, "name", &location)?,
            kind: NodeDraftKind::Command(required_string(&table, "command", &location)?),
            artifact: optional_handle(&table, "artifact", HANDLE_SCHEMA, &location)?,
            input: optional_handle_array(&table, "input", HANDLE_INPUT, &location)?
                .unwrap_or_default(),
            completion: parse_completion(&table, &location)?,
            location,
        };
        Ok(push_node(&mut self.nodes, draft))
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
        let draft = NodeDraft {
            name: optional_string(&table, "name", &location)?,
            kind: NodeDraftKind::Session {
                provider,
                model: optional_string(&table, "model", &location)?,
                permission: optional_string(&table, "permission", &location)?,
                facets,
            },
            artifact: optional_handle(&table, "artifact", HANDLE_SCHEMA, &location)?,
            input: optional_handle_array(&table, "input", HANDLE_INPUT, &location)?
                .unwrap_or_default(),
            completion: parse_completion(&table, &location)?,
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
            ],
            &location,
        )?;
        let items = match table.get_string("items") {
            None | Some(LuaData::Nil) => None,
            Some(value @ LuaData::Handle(_)) => {
                let source = expect_handle(value, HANDLE_SOURCE)
                    .or_else(|_| self.node_as_source_index(value))
                    .map_err(|_| type_error("items", "Source or literal array", &location))?;
                if !matches!(
                    self.sources.get(source),
                    Some(SourceDraft::Node { fields, .. }) if !fields.is_empty()
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
            location,
        };
        Ok(push_node(&mut self.nodes, draft))
    }

    fn node_as_source_index(&mut self, value: &LuaData) -> Result<usize, ()> {
        let node = expect_handle(value, HANDLE_NODE)?;
        let index = self.sources.len();
        self.sources.push(SourceDraft::Node {
            node,
            fields: Vec::new(),
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
                "output",
                "children",
                "artifact",
                "input",
                "completion",
            ],
            &location,
        )?;
        let draft = NodeDraft {
            name: optional_string(&table, "name", &location)?,
            kind: NodeDraftKind::Sequence {
                children: required_handle_array(&table, "children", HANDLE_CHILD, &location)?,
                entry: optional_handle(&table, "entry", HANDLE_NODE, &location)?,
                output: optional_handle(&table, "output", HANDLE_NODE, &location)?,
            },
            artifact: optional_handle(&table, "artifact", HANDLE_SCHEMA, &location)?,
            input: optional_handle_array(&table, "input", HANDLE_INPUT, &location)?
                .unwrap_or_default(),
            completion: parse_completion(&table, &location)?,
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
        reject_unknown(
            &table,
            &["node", "inputs", "rules", "on_failure"],
            &location,
        )?;
        let inputs = match table.get_string("inputs") {
            None | Some(LuaData::Nil) => Vec::new(),
            Some(LuaData::Table(values)) => {
                let mut result = Vec::new();
                for (key, value) in &values.entries {
                    let LuaTableKey::String(key) = key else {
                        return Err(type_error("inputs", "string-keyed table", &location));
                    };
                    let source = expect_handle(value, HANDLE_SOURCE)
                        .or_else(|_| self.node_as_source_index(value))
                        .or_else(|_| self.input_as_source_index(value))
                        .map_err(|_| type_error("inputs", "Source values", &location))?;
                    result.push((key.clone(), source));
                }
                result
            }
            Some(_) => return Err(type_error("inputs", "table", &location)),
        };
        let rules = optional_handle_array(&table, "rules", HANDLE_RULE, &location)?;
        let on_failure = match table.get_string("on_failure") {
            None | Some(LuaData::Nil) => None,
            Some(value) => {
                let index = expect_handle(value, HANDLE_FAILURE)
                    .map_err(|_| type_error("on_failure", "OnFailure", &location))?;
                Some(
                    *self
                        .failures
                        .get(index)
                        .ok_or_else(|| type_error("on_failure", "OnFailure", &location))?,
                )
            }
        };
        let index = self.children.len();
        self.children.push(ChildDraft {
            node: required_handle(&table, "node", HANDLE_NODE, &location)?,
            inputs,
            rules,
            on_failure,
            location,
        });
        Ok(handle(HANDLE_CHILD, index))
    }

    fn input_as_source_index(&mut self, value: &LuaData) -> Result<usize, ()> {
        let input = expect_handle(value, HANDLE_INPUT)?;
        let index = self.sources.len();
        self.sources.push(SourceDraft::Input {
            input,
            fields: Vec::new(),
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
        let on = self.required_source(&table, "on", &location)?;
        let draft = RuleDraft::When {
            on,
            on_true: required_handle(&table, "on_true", HANDLE_NODE, &location)?,
            next: required_handle(&table, "next", HANDLE_NODE, &location)?,
        };
        Ok(push_rule(&mut self.rules, draft))
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

    fn call_retry(
        &mut self,
        arguments: Vec<LuaData>,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        let count = one_u32(arguments, &location)?;
        if count == 0 {
            return Err(host_error(
                "WFS002",
                "retry count must be at least 1",
                location,
            ));
        }
        let index = self.failures.len();
        self.failures.push(OnFailure::Retry(count));
        Ok(handle(HANDLE_FAILURE, index))
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
        expect_handle(value, HANDLE_SOURCE)
            .or_else(|_| self.node_as_source_index(value))
            .or_else(|_| self.input_as_source_index(value))
            .map_err(|_| type_error(field, "Source", location))
    }

    fn validate_artifact_path(&self, node: usize, fields: &[String]) -> Result<(), String> {
        let node = self
            .nodes
            .get(node)
            .ok_or_else(|| "unknown node handle".to_string())?;
        if matches!(node.kind, NodeDraftKind::Command(_))
            && fields.len() == 1
            && crate::domain::workflow::services::contract_schema::COMMAND_RESERVED_FIELDS
                .contains(&fields[0].as_str())
        {
            return Ok(());
        }
        let schema = node
            .artifact
            .ok_or_else(|| "node does not declare an artifact".to_string())?;
        self.validate_schema_path(schema, fields, "artifact")
    }

    fn index_input(
        &mut self,
        input: usize,
        mut fields: Vec<String>,
        key: &str,
        location: LuaSourceLocation,
    ) -> Result<LuaData, LuaHostError> {
        if !fields.is_empty() {
            return Err(host_error(
                "WFR003",
                "nested input field references are not supported",
                location,
            ));
        }
        fields.push(key.to_string());
        let schema = self
            .inputs
            .get(input)
            .and_then(|input| input.contract)
            .ok_or_else(|| {
                host_error(
                    "WFR003",
                    "input does not declare a contract",
                    location.clone(),
                )
            })?;
        self.validate_schema_path(schema, &fields, "input")
            .map_err(|message| host_error("WFR003", message, location))?;
        Ok(self.push_source(SourceDraft::Input { input, fields }))
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
}

struct WorkflowGraphBuilder {
    host: WorkflowLuaHost,
    namespace: NodeNamespace,
    names: HashMap<usize, String>,
    child_uses: HashSet<usize>,
    nodes: Vec<NodeDefinition>,
    locations: BTreeMap<String, LuaSourceLocation>,
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
        Ok(LuaWorkflowDefinition {
            workflow: WorkflowDefinition {
                name: workflow.name,
                description: workflow.description,
                builtin: false,
                schemas: self.schemas,
                nodes: self.nodes,
                entry: MAIN_ENTRY_NODE_NAME.to_string(),
            },
            node_locations: self.locations,
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
            let child_node = self.host.nodes.get(child.node).ok_or_else(|| {
                build_error(
                    "WFR001",
                    "child node does not exist",
                    Some(child.location.clone()),
                )
            })?;
            let child_name = match &child_node.name {
                Some(explicit) => {
                    self.namespace
                        .register_explicit(explicit.clone())
                        .map_err(|error| {
                            build_error(
                                "WFS006",
                                error.to_string(),
                                Some(child_node.location.clone()),
                            )
                        })?
                }
                None => self
                    .namespace
                    .register_synthesized(&name, position)
                    .map_err(|error| {
                        build_error(
                            "WFS006",
                            error.to_string(),
                            Some(child_node.location.clone()),
                        )
                    })?,
            };
            self.names.insert(child.node, child_name);
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
            NodeDraftKind::Command(command) => NodeKind::Command(CommandSpec { command }),
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
            NodeDraftKind::Sequence {
                children,
                entry,
                output,
            } => {
                let child_nodes = children
                    .iter()
                    .map(|child| self.host.children[*child].node)
                    .collect::<HashSet<_>>();
                let entry =
                    self.optional_child_name(entry, &child_nodes, &draft.location, "entry")?;
                let output =
                    self.optional_child_name(output, &child_nodes, &draft.location, "output")?;
                NodeKind::Sequence(SequenceSpec {
                    entry,
                    output,
                    children: self.build_children(index, &children, false)?,
                })
            }
        };
        self.locations.insert(name.clone(), draft.location);
        self.nodes.push(NodeDefinition {
            name,
            kind,
            artifact,
            input,
            completion: draft.completion,
            worktree: None,
        });
        for child_index in &child_indices {
            let node = self.host.children[*child_index].node;
            self.visit_node(node)?;
        }
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
                    on_failure: child.on_failure,
                })
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
            Some(SourceDraft::Node { node, fields }) if !fanout && scope.contains(node) => {
                let mut raw = self.names[node].clone();
                if !fields.is_empty() {
                    raw.push('.');
                    raw.push_str(&fields.join("."));
                }
                Ok(raw)
            }
            Some(SourceDraft::Input { input, fields }) if owner_inputs.contains(input) => {
                let mut raw = self.host.inputs[*input].name.clone();
                if !fields.is_empty() {
                    raw.push('.');
                    raw.push_str(&fields.join("."));
                }
                Ok(raw)
            }
            Some(SourceDraft::Request) => Ok("request".to_string()),
            Some(SourceDraft::Items) if fanout => Ok("items".to_string()),
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
                on: self.rule_field(*on, child_node, location)?,
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

    fn rule_field(
        &self,
        source: usize,
        child_node: usize,
        location: &LuaSourceLocation,
    ) -> Result<String, LuaWorkflowError> {
        match self.host.sources.get(source) {
            Some(SourceDraft::Node { node, fields })
                if *node == child_node && !fields.is_empty() =>
            {
                Ok(fields.join("."))
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
                Some(SourceDraft::Node { node, fields }) if !fields.is_empty() => {
                    Ok(ItemsSource::ArtifactField {
                        node: self.names.get(node).cloned().unwrap_or_else(|| {
                            self.host.nodes[*node].name.clone().unwrap_or_default()
                        }),
                        field: fields.join("."),
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
}

fn handle(kind: &str, index: usize) -> LuaData {
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

fn one_u32(arguments: Vec<LuaData>, location: &LuaSourceLocation) -> Result<u32, LuaHostError> {
    match arguments.as_slice() {
        [LuaData::Integer(value)] => {
            u32::try_from(*value).map_err(|_| type_error("argument", "u32", location))
        }
        _ => Err(type_error("argument", "u32", location)),
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

fn parse_completion(
    table: &LuaTableData,
    location: &LuaSourceLocation,
) -> Result<NodeCompletion, LuaHostError> {
    match table.get_string("completion") {
        None | Some(LuaData::Nil) => Ok(NodeCompletion::Auto),
        Some(value) if expect_handle(value, HANDLE_COMPLETION) == Ok(0) => {
            Ok(NodeCompletion::Approval)
        }
        Some(_) => Err(type_error("completion", "Completion", location)),
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
        LuaData::Handle(_) => Err(type_error("items", "JSON-compatible value", location)),
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
    host_error(
        "WFS002",
        format!("missing required field '{field}'"),
        location.clone(),
    )
}

fn type_error(field: &str, expected: &str, location: &LuaSourceLocation) -> LuaHostError {
    host_error(
        "WFS002",
        format!("field '{field}' must be {expected}"),
        location.clone(),
    )
}

fn host_error(code: &str, message: impl Into<String>, location: LuaSourceLocation) -> LuaHostError {
    LuaHostError {
        category: code.to_string(),
        message: message.into(),
        location: Some(location),
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
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn load(source: &str) -> Result<LuaWorkflowDefinition, LuaWorkflowError> {
        let directory = TempDir::new().unwrap();
        load_lua_workflow(
            "review.lua",
            source,
            directory.path(),
            LuaFacetCatalog::default(),
        )
    }

    #[test]
    fn builds_a_workflow_and_synthesizes_child_names() {
        let loaded = load(
            r#"
local r = require("releash")
local first = r.command{ command = "echo first" }
local second = r.command{ command = "echo second" }
return r.workflow{
  name = "review",
  description = "Review",
  main = r.sequence{
    children = {
      r.child{ node = first },
      r.child{ node = second },
    },
  },
}
"#,
        )
        .unwrap();

        assert_eq!(loaded.workflow.entry, "main");
        assert_eq!(loaded.workflow.nodes[0].name, "main");
        assert!(loaded.workflow.node_by_name("main#0").is_some());
        assert!(loaded.workflow.node_by_name("main#1").is_some());
    }

    #[test]
    fn rejects_same_node_value_in_multiple_children() {
        let error = load(
            r#"
local r = require("releash")
local child = r.command{ command = "echo" }
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = child }, r.child{ node = child },
  } },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFC007");
    }

    #[test]
    fn rejects_unknown_builder_field_at_call_line() {
        let error = load(
            r#"
local r = require("releash")
local child = r.command{ command = "echo", unknown = true }
return r.workflow{ name = "review", description = "Review", main = child }
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFS002");
        assert_eq!(error.location.unwrap().line, 3);
    }

    #[test]
    fn maps_require_failures_and_non_workflow_returns_to_spec_codes() {
        let directory = TempDir::new().unwrap();
        let require_error = load_lua_workflow(
            "review.lua",
            "local value = require('../outside')\nreturn value",
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();
        let return_error = load_lua_workflow(
            "review.lua",
            "return {}",
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();

        assert_eq!(require_error.code, "WFS011");
        assert_eq!(require_error.location.unwrap().line, 1);
        assert_eq!(return_error.code, "WFS010");
        assert_eq!(return_error.location.unwrap().line, 1);
    }

    #[test]
    fn rejects_unknown_artifact_field_at_index_line() {
        let error = load(
            r#"
local r = require("releash")
local child = r.command{
  command = "echo",
  artifact = r.schema.object{ properties = { ok = r.schema.boolean() } },
}
local invalid = child.missing
return r.workflow{ name = "review", description = "Review", main = child }
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        let location = error.location.unwrap();
        assert_eq!(location.source, "review.lua");
        assert_eq!(location.line, 7);
    }

    #[test]
    fn rejects_unknown_facet_at_reference_line() {
        let directory = TempDir::new().unwrap();
        let error = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local f = require("facets")
local child = r.session{
  provider = r.provider.claude,
  facets = { instruction = f.instruction.missing },
}
return r.workflow{ name = "review", description = "Review", main = child }
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR900");
        let location = error.location.unwrap();
        assert_eq!(location.source, "review.lua");
        assert_eq!(location.line, 6);
    }

    #[test]
    fn reports_reference_error_at_required_component_file_and_line() {
        let directory = TempDir::new().unwrap();
        let component = directory.path().join("component.lua");
        fs::write(
            &component,
            r#"
local r = require("releash")
return function()
  local child = r.command{ command = "echo" }
  local invalid = child.missing
  return child
end
"#,
        )
        .unwrap();

        let error = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local component = require("component")
return r.workflow{ name = "review", description = "Review", main = component() }
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();
        let location = error.location.unwrap();

        assert_eq!(error.code, "WFR003");
        assert_eq!(
            location.source,
            fs::canonicalize(component).unwrap().to_string_lossy()
        );
        assert_eq!(location.line, 5);
    }

    #[test]
    fn require_component_function_creates_independent_nodes() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("component.lua"),
            r#"
local r = require("releash")
return function(command)
  local leaf = r.command{ command = command }
  return r.sequence{
    completion = r.completion.approval,
    children = { r.child{ node = leaf } },
  }
end
"#,
        )
        .unwrap();
        let loaded = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local component = require("component")
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = component("one") },
    r.child{ node = component("two") },
  } },
}
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap();

        assert_eq!(loaded.workflow.nodes.len(), 5);
        assert_eq!(
            loaded
                .workflow
                .nodes
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["main", "main#0", "main#0#0", "main#1", "main#1#0"]
        );
        assert_eq!(
            loaded.workflow.node_by_name("main#0").unwrap().completion,
            NodeCompletion::Approval
        );
        assert!(loaded
            .workflow
            .node_by_name("main#0")
            .unwrap()
            .is_sequence());
    }

    #[test]
    fn builds_schema_fanout_items_and_scoped_item_wiring() {
        let directory = TempDir::new().unwrap();
        let loaded = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
local f = require("facets")
local topic = r.schema.string{}
local detail = r.schema.object{
  name = "topic-detail",
  properties = { label = r.schema.string{} },
}
local scan = r.command{
  command = "scan",
  artifact = r.schema.object{
    properties = {
      topics = r.schema.array{ items = topic },
      detail = detail,
    },
    required = { "topics" },
  },
}
local worker = r.session{
  provider = r.provider.codex,
  facets = { instruction = f.instruction.review },
  input = { r.input("topic", topic) },
}
local spread = r.fanout{
  items = scan.topics,
  children = {
    r.child{ node = worker, inputs = { topic = r.items } },
  },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = scan },
    r.child{ node = spread },
  } },
}
"#,
            directory.path(),
            LuaFacetCatalog {
                instruction: vec!["review".to_string()],
                ..LuaFacetCatalog::default()
            },
        )
        .unwrap();

        assert_eq!(
            loaded
                .workflow
                .nodes
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["main", "main#0", "main#1", "main#1#0"]
        );
        let validation_errors = crate::domain::workflow::validation::validate_all(&loaded.workflow);
        assert!(validation_errors.is_empty(), "{validation_errors:#?}");
        assert!(loaded.workflow.schemas.contains_key("topic-detail"));
    }

    #[test]
    fn rejects_input_source_from_outer_composite_scope() {
        let error = load(
            r#"
local r = require("releash")
local outer = r.input("outer")
local leaf = r.command{ command = "echo", input = { r.input("value") } }
local inner = r.sequence{
  children = { r.child{ node = leaf, inputs = { value = outer } } },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{
    input = { outer },
    children = { r.child{ node = inner } },
  },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR007");
        let location = error.location.unwrap();
        assert_eq!(location.source, "review.lua");
        assert_eq!(location.line, 6);
    }

    #[test]
    fn accepts_field_reference_from_composite_input_contract() {
        let loaded = load(
            r#"
local r = require("releash")
local payload = r.input("payload", r.schema.object{
  properties = { message = r.schema.string{} },
  required = { "message" },
})
local leaf = r.command{ command = "echo", input = { r.input("value") } }
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{
    input = { payload },
    children = { r.child{ node = leaf, inputs = { value = payload.message } } },
  },
}
"#,
        )
        .unwrap();

        let sequence = loaded.workflow.entry_node().unwrap().sequence().unwrap();
        assert_eq!(sequence.children[0].inputs[0].1.raw(), "payload.message");
    }

    #[test]
    fn rejects_named_main_node() {
        let error = load(
            r#"
local r = require("releash")
return r.workflow{
  name = "review", description = "Review",
  main = r.command{ name = "root", command = "true" },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFS006");
        assert_eq!(error.location.unwrap().line, 5);
    }

    #[test]
    fn rejects_missing_main_with_existing_resolve_diagnostic() {
        let error = load(
            r#"
local r = require("releash")
return r.workflow{ name = "review", description = "Review" }
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR006");
        let location = error.location.unwrap();
        assert_eq!(location.source, "review.lua");
        assert_eq!(location.line, 3);
    }

    #[test]
    fn rejects_nested_field_reference_at_the_index_line() {
        let error = load(
            r#"
local r = require("releash")
local child = r.command{
  command = "echo",
  artifact = r.schema.object{ properties = {
    nested = r.schema.object{ properties = { value = r.schema.string{} } },
  } },
}
local invalid = child.nested.value
return r.workflow{ name = "review", description = "Review", main = child }
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        assert_eq!(error.location.unwrap().line, 9);
    }

    #[test]
    fn rejects_non_field_fanout_items_at_the_builder_line() {
        let error = load(
            r#"
local r = require("releash")
local source = r.command{ command = "source" }
local child = r.command{ command = "child" }
local spread = r.fanout{
  items = source,
  children = { r.child{ node = child } },
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = source }, r.child{ node = spread },
  } },
}
"#,
        )
        .unwrap_err();

        assert_eq!(error.code, "WFR003");
        assert_eq!(error.location.unwrap().line, 5);
    }

    #[test]
    fn lua_definition_equals_the_same_yaml_definition_without_origin_metadata() {
        let loaded = load(
            r#"
local r = require("releash")
local result = r.schema.object{
  name = "result",
  properties = { message = r.schema.string{} },
  required = { "message" },
}
local inspect = r.command{
  name = "inspect",
  command = "echo inspect",
  artifact = result,
  input = { r.input("request_text") },
  completion = r.completion.approval,
}
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = inspect, inputs = { request_text = r.request } },
  } },
}
"#,
        )
        .unwrap();
        let yaml: WorkflowDefinition = serde_saphyr::from_str(
            r#"
name: review
description: Review
schemas:
  result:
    type: object
    properties:
      message: string
    required:
      - message
nodes:
  main:
    sequence:
      children:
        - inspect:
            command: echo inspect
            artifact: result
            input:
              - request_text
            completion: approval
            inputs:
              request_text: request
"#,
        )
        .unwrap();

        assert_eq!(loaded.workflow, yaml);

        use crate::domain::workflow::entities::workflow_execution::{
            WorkflowExecution, WorkflowExecutionRestore,
        };
        let mut lua_execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
            id: "execution".to_string(),
            workflow: loaded.workflow,
            ..WorkflowExecutionRestore::default()
        });
        let mut yaml_execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
            id: "execution".to_string(),
            workflow: yaml,
            ..WorkflowExecutionRestore::default()
        });
        let mut lua_index = 0_u32;
        let mut yaml_index = 0_u32;
        let lua_started = lua_execution
            .start_root(
                &mut || {
                    lua_index += 1;
                    format!("node-{lua_index}")
                },
                1.0,
            )
            .unwrap();
        let yaml_started = yaml_execution
            .start_root(
                &mut || {
                    yaml_index += 1;
                    format!("node-{yaml_index}")
                },
                1.0,
            )
            .unwrap();

        assert_eq!(lua_started, yaml_started);
        assert_eq!(lua_execution, yaml_execution);
    }

    #[test]
    fn builds_all_rule_completion_and_failure_variants() {
        let loaded = load(
            r#"
local r = require("releash")
local check = r.command{
  command = "check",
}
local classify = r.command{
  command = "classify",
  completion = r.completion.approval,
  artifact = r.schema.object{
    properties = { status = r.schema.string{ enum = { "done", "retry" } } },
    required = { "status" },
  },
}
local retry = r.command{ command = "retry" }
local done = r.command{ command = "done" }
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{
      node = check,
      rules = { r.when{ on = check.ok, on_true = classify, next = retry } },
      on_failure = r.retry(2),
    },
    r.child{
      node = classify,
      rules = { r.switch{ on = classify.status, cases = {
        done = done,
        retry = retry,
      }, next = done } },
    },
    r.child{
      node = retry,
      rules = {
        r.loop_guard{ max_iterations = 3, on_exhausted = done },
        r.next(check),
      },
    },
    r.child{ node = done, rules = {}, on_failure = r.ignore },
  } },
}
"#,
        )
        .unwrap();

        let main = loaded.workflow.root_sequence().unwrap();
        assert!(matches!(
            main.children[0].rules.as_deref(),
            Some([Rule::When { .. }])
        ));
        assert_eq!(main.children[0].on_failure, Some(OnFailure::Retry(2)));
        assert!(matches!(
            main.children[1].rules.as_deref(),
            Some([Rule::Switch { .. }])
        ));
        assert!(matches!(
            main.children[2].rules.as_deref(),
            Some([Rule::LoopGuard { .. }, Rule::Next(_)])
        ));
        assert_eq!(main.children[3].rules.as_deref(), Some(&[][..]));
        assert_eq!(main.children[3].on_failure, Some(OnFailure::Ignore));
        assert_eq!(
            loaded.workflow.node_by_name("main#1").unwrap().completion,
            NodeCompletion::Approval
        );
        let errors = crate::domain::workflow::validation::validate_all(&loaded.workflow);
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn runtime_modules_ignore_missing_or_stale_editor_stubs() {
        let directory = TempDir::new().unwrap();
        let source = r#"
local r = require("releash")
local f = require("facets")
return r.workflow{
  name = "review", description = "Review",
  main = r.session{
    provider = r.provider.claude,
    facets = { instruction = f.instruction.live },
  },
}
"#;
        let catalog = || LuaFacetCatalog {
            instruction: vec!["live".to_string()],
            ..LuaFacetCatalog::default()
        };
        let without_stubs =
            load_lua_workflow("review.lua", source, directory.path(), catalog()).unwrap();
        fs::create_dir_all(directory.path().join(".releash")).unwrap();
        fs::write(
            directory.path().join(".releash/releash.lua"),
            "error('stale runtime stub must not run')",
        )
        .unwrap();
        fs::write(
            directory.path().join(".releash/facets.lua"),
            "return { instruction = {} }",
        )
        .unwrap();

        let with_stale_stubs =
            load_lua_workflow("review.lua", source, directory.path(), catalog()).unwrap();

        assert_eq!(with_stale_stubs.workflow, without_stubs.workflow);
    }

    #[test]
    fn repeated_loads_of_the_same_file_group_are_deterministic() {
        let directory = TempDir::new().unwrap();
        fs::write(
            directory.path().join("component.lua"),
            r#"
local r = require("releash")
return function(label)
  return r.command{ command = "echo " .. label }
end
"#,
        )
        .unwrap();
        let source = r#"
local r = require("releash")
local component = require("component")
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = {
    r.child{ node = component("one") },
    r.child{ node = component("two") },
  } },
}
"#;

        let first = load_lua_workflow(
            "review.lua",
            source,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap();
        let second = load_lua_workflow(
            "review.lua",
            source,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap();

        assert_eq!(second.workflow, first.workflow);
    }

    #[test]
    fn rejects_definitions_that_exhaust_the_host_arena_budget() {
        let directory = TempDir::new().unwrap();

        let error = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
for _ = 1, 200000 do
  r.command{ command = "x" }
end
return r.workflow{
  name = "review", description = "Review",
  main = r.sequence{ children = { r.child{ node = r.command{ command = "true" } } } },
}
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        )
        .unwrap_err();

        assert_eq!(error.code, "WFS010");
        assert!(error.message.contains("builder values"));
    }

    #[test]
    fn resource_limits_are_reported_as_wfs010_without_poisoning_following_loads() {
        let directory = TempDir::new().unwrap();
        let infinite = load_lua_workflow_with_limits(
            "infinite.lua",
            "while true do end",
            directory.path(),
            LuaFacetCatalog::default(),
            LuaLimits {
                memory_bytes: 64 * 1024 * 1024,
                instructions: 20_000,
            },
        )
        .unwrap_err();
        let oversized = load_lua_workflow_with_limits(
            "oversized.lua",
            "return string.rep('x', 16777216)",
            directory.path(),
            LuaFacetCatalog::default(),
            LuaLimits {
                memory_bytes: 4 * 1024 * 1024,
                instructions: 50_000_000,
            },
        )
        .unwrap_err();
        let following = load_lua_workflow(
            "review.lua",
            r#"
local r = require("releash")
return r.workflow{
  name = "review", description = "Review",
  main = r.command{ command = "true" },
}
"#,
            directory.path(),
            LuaFacetCatalog::default(),
        );

        assert_eq!(infinite.code, "WFS010");
        assert!(infinite.message.contains("instruction limit"));
        assert_eq!(oversized.code, "WFS010");
        assert!(oversized.message.contains("memory limit"));
        assert!(following.is_ok());
    }
}
