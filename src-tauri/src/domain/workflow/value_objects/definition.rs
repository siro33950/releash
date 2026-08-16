use std::collections::{BTreeMap, BTreeSet};

use serde::de;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::services::{contract_schema, reference};

pub const MAX_NODES_PER_WORKFLOW: usize = 256;
pub const MAX_FANOUT_CHILDREN: usize = 64;

/// root node の規約名。YAML に entry フィールドは持たず、loader が設定する。
pub const MAIN_ENTRY_NODE_NAME: &str = "main";

/// node 名として使用禁止の予約語（kind 名とフィールド名）。
pub const RESERVED_NODE_NAMES: [&str; 15] = [
    "command",
    "session",
    "fanout",
    "sequence",
    "input",
    "artifact",
    "completion",
    "worktree",
    "inputs",
    "rules",
    "on_failure",
    "items",
    "entry",
    "output",
    "children",
];

pub fn is_reserved_node_name(name: &str) -> bool {
    RESERVED_NODE_NAMES.contains(&name)
}

fn default_entry_node() -> String {
    MAIN_ENTRY_NODE_NAME.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, SchemaDef>,
    #[serde(
        deserialize_with = "deserialize_node_catalog",
        serialize_with = "serialize_node_catalog"
    )]
    pub nodes: Vec<NodeDefinition>,
    /// root node 名。YAML には露出せず、deserialize 時は常に `main`。
    #[serde(skip, default = "default_entry_node")]
    pub entry: String,
}

impl Default for WorkflowDefinition {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            builtin: false,
            schemas: BTreeMap::new(),
            nodes: Vec::new(),
            entry: default_entry_node(),
        }
    }
}

impl WorkflowDefinition {
    pub fn entry_node(&self) -> Option<&NodeDefinition> {
        self.entry_index().map(|index| &self.nodes[index])
    }

    pub fn entry_index(&self) -> Option<usize> {
        self.nodes.iter().position(|node| node.name == self.entry)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SchemaDef {
    Object {
        properties: BTreeMap<String, SchemaDef>,
        required: BTreeSet<String>,
    },
    Array {
        items: String,
    },
    String {
        r#enum: Option<Vec<String>>,
    },
    Boolean,
    Integer,
    Number,
}

impl Serialize for SchemaDef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        contract_schema::schema_def_to_json_value(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SchemaDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        contract_schema::schema_def_from_json(&value).map_err(de::Error::custom)
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum NodeKindName {
    Command,
    #[default]
    Session,
    Fanout,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Command(CommandSpec),
    Session(SessionSpec),
    Fanout(FanoutSpec),
}

#[cfg(test)]
impl Default for NodeKind {
    fn default() -> Self {
        Self::Session(SessionSpec::default())
    }
}

impl NodeKind {
    pub fn name(&self) -> NodeKindName {
        match self {
            Self::Command(_) => NodeKindName::Command,
            Self::Session(_) => NodeKindName::Session,
            Self::Fanout(_) => NodeKindName::Fanout,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub command: String,
}

/// Node 自身が持つ完了の定義。全 Node 種別で宣言可・省略可。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeCompletion {
    #[default]
    Auto,
    Approval,
}

impl NodeCompletion {
    fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct FacetRefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_one_or_many_strings",
        serialize_with = "serialize_one_or_many_strings",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub knowledge: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

impl FacetRefs {
    pub fn is_empty(&self) -> bool {
        self.policy.is_none() && self.knowledge.is_empty() && self.instruction.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionSpec {
    #[serde(
        deserialize_with = "deserialize_provider_kind",
        serialize_with = "serialize_provider_kind"
    )]
    pub provider: ProviderKind,
    /// provider CLI へそのまま渡す model 指定。値域は provider CLI が定める。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// provider CLI へそのまま渡す permission 指定。値域は provider CLI が定める。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(default, skip_serializing_if = "FacetRefs::is_empty")]
    pub facets: FacetRefs,
}

#[cfg(test)]
impl Default for SessionSpec {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Claude,
            model: None,
            permission: None,
            facets: FacetRefs::default(),
        }
    }
}

fn deserialize_provider_kind<'de, D>(deserializer: D) -> Result<ProviderKind, D::Error>
where
    D: Deserializer<'de>,
{
    match String::deserialize(deserializer)?.as_str() {
        "claude" => Ok(ProviderKind::Claude),
        "codex" => Ok(ProviderKind::Codex),
        value => Err(de::Error::unknown_variant(value, &["claude", "codex"])),
    }
}

fn serialize_provider_kind<S>(provider: &ProviderKind, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct FanoutSpec {
    #[serde(
        deserialize_with = "deserialize_one_or_many_strings",
        serialize_with = "serialize_one_or_many_strings"
    )]
    pub child: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<ItemsSource>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemsSource {
    Literal(Vec<Value>),
    ArtifactField { node: String, field: String },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawOneOrManyStrings {
    One(String),
    Many(Vec<String>),
}

fn deserialize_one_or_many_strings<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    match RawOneOrManyStrings::deserialize(deserializer)? {
        RawOneOrManyStrings::One(value) => Ok(vec![value]),
        RawOneOrManyStrings::Many(values) => Ok(values),
    }
}

fn serialize_one_or_many_strings<S>(values: &[String], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match values {
        [value] => serializer.serialize_str(value),
        values => values.serialize(serializer),
    }
}

impl<'de> Deserialize<'de> for ItemsSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawItemsSource {
            Literal(Vec<Value>),
            ArtifactField(String),
        }

        match RawItemsSource::deserialize(deserializer)? {
            RawItemsSource::Literal(items) => Ok(Self::Literal(items)),
            RawItemsSource::ArtifactField(value) => match reference::parse_reference(&value) {
                Ok(reference::ArtifactReference::Node {
                    node,
                    field: Some(field),
                }) => Ok(Self::ArtifactField { node, field }),
                _ => Err(de::Error::custom(
                    "fanout.items must be a literal array or a <node>.<field> Artifact reference",
                )),
            },
        }
    }
}

impl Serialize for ItemsSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Literal(items) => items.serialize(serializer),
            Self::ArtifactField { node, field } => {
                serializer.serialize_str(&format!("{node}.{field}"))
            }
        }
    }
}

/// Node の Interface パラメータ。文字列（型なし）または `名前: Contract`（型あり）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputParam {
    pub name: String,
    pub contract: Option<String>,
}

impl<'de> Deserialize<'de> for InputParam {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawInputParam {
            Name(String),
            Typed(BTreeMap<String, String>),
        }

        match RawInputParam::deserialize(deserializer)? {
            RawInputParam::Name(name) => Ok(Self {
                name,
                contract: None,
            }),
            RawInputParam::Typed(map) => {
                let mut entries = map.into_iter();
                match (entries.next(), entries.next()) {
                    (Some((name, contract)), None) => Ok(Self {
                        name,
                        contract: Some(contract),
                    }),
                    _ => Err(de::Error::custom(
                        "input entry must be a parameter name or a single `<name>: <Contract>` pair",
                    )),
                }
            }
        }
    }
}

impl Serialize for InputParam {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.contract {
            None => serializer.serialize_str(&self.name),
            Some(contract) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(&self.name, contract)?;
                map.end()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(Default))]
pub struct NodeDefinition {
    pub name: String,
    pub kind: NodeKind,
    pub artifact: Option<String>,
    pub input: Vec<InputParam>,
    pub inputs: Vec<String>,
    pub rules: Vec<Rule>,
    pub completion: NodeCompletion,
}

/// nodes マップの値（node 名は親マップのキー）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNodeBody {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    session: Option<SessionSpec>,
    #[serde(default)]
    fanout: Option<FanoutSpec>,
    #[serde(default)]
    artifact: Option<String>,
    #[serde(default)]
    input: Vec<InputParam>,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    rules: Vec<Rule>,
    #[serde(default)]
    completion: NodeCompletion,
}

impl RawNodeBody {
    fn into_node_definition<E>(self, name: String) -> Result<NodeDefinition, E>
    where
        E: de::Error,
    {
        let kind_count = self.command.is_some() as usize
            + self.session.is_some() as usize
            + self.fanout.is_some() as usize;
        if kind_count != 1 {
            return Err(E::custom(format!(
                "node '{name}' must contain exactly one kind block: command, session, or fanout"
            )));
        }
        let kind = if let Some(command) = self.command {
            NodeKind::Command(CommandSpec { command })
        } else if let Some(session) = self.session {
            NodeKind::Session(session)
        } else if let Some(fanout) = self.fanout {
            NodeKind::Fanout(fanout)
        } else {
            unreachable!("kind_count checked above")
        };
        Ok(NodeDefinition {
            name,
            kind,
            artifact: self.artifact,
            input: self.input,
            inputs: self.inputs,
            rules: self.rules,
            completion: self.completion,
        })
    }
}

fn deserialize_node_catalog<'de, D>(deserializer: D) -> Result<Vec<NodeDefinition>, D::Error>
where
    D: Deserializer<'de>,
{
    struct CatalogVisitor;

    impl<'de> de::Visitor<'de> for CatalogVisitor {
        type Value = Vec<NodeDefinition>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a mapping of node name to node definition")
        }

        fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
        where
            A: de::MapAccess<'de>,
        {
            let mut nodes: Vec<NodeDefinition> = Vec::new();
            let mut seen = BTreeSet::new();
            while let Some((name, body)) = access.next_entry::<String, RawNodeBody>()? {
                if !seen.insert(name.clone()) {
                    return Err(de::Error::custom(format!(
                        "node name '{name}' is duplicated"
                    )));
                }
                nodes.push(body.into_node_definition(name)?);
            }
            Ok(nodes)
        }
    }

    deserializer.deserialize_map(CatalogVisitor)
}

fn serialize_node_catalog<S>(nodes: &[NodeDefinition], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut map = serializer.serialize_map(Some(nodes.len()))?;
    for node in nodes {
        map.serialize_entry(&node.name, node)?;
    }
    map.end()
}

impl Serialize for NodeDefinition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        match &self.kind {
            NodeKind::Command(spec) => map.serialize_entry("command", &spec.command)?,
            NodeKind::Session(spec) => map.serialize_entry("session", spec)?,
            NodeKind::Fanout(spec) => map.serialize_entry("fanout", spec)?,
        }
        if !self.input.is_empty() {
            map.serialize_entry("input", &self.input)?;
        }
        serialize_option(&mut map, "artifact", &self.artifact)?;
        if !self.completion.is_auto() {
            map.serialize_entry("completion", &self.completion)?;
        }
        if !self.inputs.is_empty() {
            map.serialize_entry("inputs", &self.inputs)?;
        }
        if !self.rules.is_empty() {
            map.serialize_entry("rules", &self.rules)?;
        }
        map.end()
    }
}

fn serialize_option<M, T>(map: &mut M, key: &'static str, value: &Option<T>) -> Result<(), M::Error>
where
    M: SerializeMap,
    T: Serialize,
{
    if let Some(value) = value {
        map.serialize_entry(key, value)?;
    }
    Ok(())
}

impl NodeDefinition {
    pub fn has_facet_refs(&self) -> bool {
        self.session()
            .is_some_and(|session| !session.facets.is_empty())
    }

    pub fn kind_name(&self) -> NodeKindName {
        self.kind.name()
    }

    pub fn is_command(&self) -> bool {
        matches!(self.kind, NodeKind::Command(_))
    }

    pub fn is_session(&self) -> bool {
        matches!(self.kind, NodeKind::Session(_))
    }

    pub fn requires_approval_completion(&self) -> bool {
        self.completion == NodeCompletion::Approval
    }

    /// 配線（children の inputs）が未導入のため、fanout items の要素型は
    /// `input` が単一の型付きパラメータの場合にのみ確定する。
    pub fn sole_typed_input_contract(&self) -> Option<&str> {
        match self.input.as_slice() {
            [param] => param.contract.as_deref(),
            _ => None,
        }
    }

    pub fn is_fanout(&self) -> bool {
        matches!(self.kind, NodeKind::Fanout(_))
    }

    pub fn command(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Command(spec) => Some(spec.command.as_str()),
            _ => None,
        }
    }

    pub fn session(&self) -> Option<&SessionSpec> {
        match &self.kind {
            NodeKind::Session(spec) => Some(spec),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn session_mut(&mut self) -> Option<&mut SessionSpec> {
        match &mut self.kind {
            NodeKind::Session(spec) => Some(spec),
            _ => None,
        }
    }

    pub fn fanout(&self) -> Option<&FanoutSpec> {
        match &self.kind {
            NodeKind::Fanout(spec) => Some(spec),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    When {
        on: String,
        then: String,
        next: String,
    },
    Switch {
        on: String,
        cases: BTreeMap<String, String>,
        next: Option<String>,
    },
    LoopGuard {
        max_iterations: u32,
        on_exhausted: String,
        reset_on: Option<String>,
    },
    Next(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct WhenRule {
    on: String,
    then: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SwitchRule {
    on: String,
    cases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct LoopGuardRule {
    max_iterations: u32,
    on_exhausted: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reset_on: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    #[serde(default)]
    when: Option<WhenRule>,
    #[serde(default)]
    switch: Option<SwitchRule>,
    #[serde(default)]
    loop_guard: Option<LoopGuardRule>,
    #[serde(default)]
    next: Option<String>,
}

impl<'de> Deserialize<'de> for Rule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRule::deserialize(deserializer)?;
        let discriminator_count = raw.when.is_some() as usize
            + raw.switch.is_some() as usize
            + raw.loop_guard.is_some() as usize;
        match (raw.when, raw.switch, raw.loop_guard, raw.next) {
            (Some(when), None, None, Some(next)) => Ok(Self::When {
                on: when.on,
                then: when.then,
                next,
            }),
            (Some(_), None, None, None) => {
                Err(de::Error::custom("when rule requires sibling next"))
            }
            (None, Some(switch), None, next) => Ok(Self::Switch {
                on: switch.on,
                cases: switch.cases,
                next,
            }),
            (None, None, Some(loop_guard), None) => Ok(Self::LoopGuard {
                max_iterations: loop_guard.max_iterations,
                on_exhausted: loop_guard.on_exhausted,
                reset_on: loop_guard.reset_on,
            }),
            (None, None, None, Some(next)) => Ok(Self::Next(next)),
            (None, None, Some(_), Some(_)) => {
                Err(de::Error::custom("loop_guard rule cannot include next"))
            }
            (None, None, None, None) => Err(de::Error::custom(
                "rule must contain one of when, switch, loop_guard, or next",
            )),
            _ if discriminator_count > 1 => Err(de::Error::custom(
                "rule discriminator keys when, switch, and loop_guard are mutually exclusive",
            )),
            _ => Err(de::Error::custom("invalid rule shape")),
        }
    }
}

impl Serialize for Rule {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::When { on, then, next } => {
                map.serialize_entry(
                    "when",
                    &WhenRule {
                        on: on.clone(),
                        then: then.clone(),
                    },
                )?;
                map.serialize_entry("next", next)?;
            }
            Self::Switch { on, cases, next } => {
                map.serialize_entry(
                    "switch",
                    &SwitchRule {
                        on: on.clone(),
                        cases: cases.clone(),
                    },
                )?;
                serialize_option(&mut map, "next", next)?;
            }
            Self::LoopGuard {
                max_iterations,
                on_exhausted,
                reset_on,
            } => {
                map.serialize_entry(
                    "loop_guard",
                    &LoopGuardRule {
                        max_iterations: *max_iterations,
                        on_exhausted: on_exhausted.clone(),
                        reset_on: reset_on.clone(),
                    },
                )?;
            }
            Self::Next(next) => {
                map.serialize_entry("next", next)?;
            }
        }
        map.end()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSummary {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub is_running: bool,
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn test_node_definition_facet参照を検出する() {
        let mut node = NodeDefinition::default();
        assert!(!node.has_facet_refs());
        if let Some(session) = node.session_mut() {
            session.facets.knowledge = vec!["releash-thread-cli".to_string()];
        }
        assert!(node.has_facet_refs());
    }

    #[test]
    fn test_entry解決_mainが先頭以外でもrootとして解決される() {
        let node = |name: &str| NodeDefinition {
            name: name.to_string(),
            ..Default::default()
        };
        let workflow = WorkflowDefinition {
            nodes: vec![node("helper"), node("main"), node("done")],
            ..Default::default()
        };

        assert_eq!(workflow.entry_index(), Some(1));
        assert_eq!(workflow.entry_node().map(|n| n.name.as_str()), Some("main"));
    }

    #[test]
    fn test_entry解決_entryと同名nodeが無ければ解決しない() {
        let workflow = WorkflowDefinition {
            nodes: vec![NodeDefinition {
                name: "helper".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        assert_eq!(workflow.entry_index(), None);
        assert!(workflow.entry_node().is_none());
    }
}
