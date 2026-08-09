use std::collections::{BTreeMap, BTreeSet};

use serde::de;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::services::{contract_schema, reference};

pub const MAX_NODES_PER_WORKFLOW: usize = 256;
pub const MAX_FANOUT_CHILDREN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, SchemaDef>,
    pub nodes: Vec<NodeDefinition>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SessionGate {
    #[default]
    Auto,
    Approval,
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
    pub gate: SessionGate,
    #[serde(default, skip_serializing_if = "FacetRefs::is_empty")]
    pub facets: FacetRefs,
}

#[cfg(test)]
impl Default for SessionSpec {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Claude,
            gate: SessionGate::Auto,
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

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(Default))]
pub struct NodeDefinition {
    pub name: String,
    pub kind: NodeKind,
    pub artifact: Option<String>,
    pub input: Option<String>,
    pub inputs: Vec<String>,
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNodeDefinition {
    name: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    session: Option<SessionSpec>,
    #[serde(default)]
    fanout: Option<FanoutSpec>,
    #[serde(default)]
    artifact: Option<String>,
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default, rename = "rules")]
    rules: Vec<Rule>,
}

impl<'de> Deserialize<'de> for NodeDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawNodeDefinition::deserialize(deserializer)?;
        let kind_count = raw.command.is_some() as usize
            + raw.session.is_some() as usize
            + raw.fanout.is_some() as usize;
        if kind_count != 1 {
            return Err(de::Error::custom(format!(
                "NodeDefinition '{}' must contain exactly one kind block: command, session, or fanout",
                raw.name
            )));
        }
        let kind = if let Some(command) = raw.command {
            NodeKind::Command(CommandSpec { command })
        } else if let Some(session) = raw.session {
            NodeKind::Session(session)
        } else if let Some(fanout) = raw.fanout {
            NodeKind::Fanout(fanout)
        } else {
            unreachable!("kind_count checked above")
        };
        Ok(Self {
            name: raw.name,
            kind,
            artifact: raw.artifact,
            input: raw.input,
            inputs: raw.inputs,
            rules: raw.rules,
        })
    }
}

impl Serialize for NodeDefinition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("name", &self.name)?;
        match &self.kind {
            NodeKind::Command(spec) => map.serialize_entry("command", &spec.command)?,
            NodeKind::Session(spec) => map.serialize_entry("session", spec)?,
            NodeKind::Fanout(spec) => map.serialize_entry("fanout", spec)?,
        }
        serialize_option(&mut map, "artifact", &self.artifact)?;
        serialize_option(&mut map, "input", &self.input)?;
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

    pub fn is_approval_session(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::Session(SessionSpec {
                gate: SessionGate::Approval,
                ..
            })
        )
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
}
