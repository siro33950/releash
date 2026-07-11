use serde::de;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use super::domain_mapping;
use crate::domain::workflow as domain_workflow;
use crate::domain::workflow::services::contract_schema;

#[derive(Debug, Clone, PartialEq)]
pub enum SchemaDef {
    Object {
        properties: BTreeMap<String, SchemaDef>,
        required: BTreeSet<String>,
        additional_properties: bool,
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
        contract_schema::schema_def_to_json_value(&domain_mapping::schema_def_to_domain(self))
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SchemaDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        contract_schema::schema_def_from_json(&value)
            .map(schema_def_from_domain)
            .map_err(de::Error::custom)
    }
}

fn schema_def_from_domain(schema: domain_workflow::SchemaDef) -> SchemaDef {
    match schema {
        domain_workflow::SchemaDef::Object {
            properties,
            required,
            additional_properties,
        } => SchemaDef::Object {
            properties: properties
                .into_iter()
                .map(|(name, schema)| (name, schema_def_from_domain(schema)))
                .collect(),
            required,
            additional_properties,
        },
        domain_workflow::SchemaDef::Array { items } => SchemaDef::Array { items },
        domain_workflow::SchemaDef::String { r#enum } => SchemaDef::String { r#enum },
        domain_workflow::SchemaDef::Boolean => SchemaDef::Boolean,
        domain_workflow::SchemaDef::Integer => SchemaDef::Integer,
        domain_workflow::SchemaDef::Number => SchemaDef::Number,
    }
}

/// ワークフローテンプレート定義。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, SchemaDef>,
    pub nodes: Vec<NodeDefinition>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeKindName {
    Command,
    #[default]
    Session,
    Fanout,
}

impl NodeKindName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Session => "session",
            Self::Fanout => "fanout",
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
}

impl FacetRefs {
    pub fn is_empty(&self) -> bool {
        self.policy.is_none() && self.knowledge.is_none() && self.instruction.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct SessionSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(default)]
    pub gate: SessionGate,
    #[serde(default, skip_serializing_if = "FacetRefs::is_empty")]
    pub facets: FacetRefs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct FanoutSpec {
    pub parallel_children: Vec<InterimChild>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<ParallelAggregate>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Command(CommandSpec),
    Session(SessionSpec),
    Fanout(FanoutSpec),
}

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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeDefinition {
    pub name: String,
    pub kind: NodeKind,
    // #1325/#1326/#1327 で意味を移す新しい共通フィールドの位置だけ先に確保する。
    pub artifact: Option<String>,
    pub input: Option<String>,
    pub inputs: Vec<String>,
    pub collect: Option<CollectConfig>,
    pub rules: Vec<Rule>,
}

impl NodeDefinition {
    pub fn has_facet_refs(&self) -> bool {
        self.session()
            .is_some_and(|session| !session.facets.is_empty())
    }

    pub fn kind_name(&self) -> NodeKindName {
        self.kind.name()
    }

    pub fn is_approval_session(&self) -> bool {
        self.session()
            .is_some_and(|session| session.gate == SessionGate::Approval)
    }

    pub fn is_fanout(&self) -> bool {
        matches!(self.kind, NodeKind::Fanout(_))
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
    #[serde(default)]
    collect: Option<CollectConfig>,
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
            collect: raw.collect,
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
        serialize_option(&mut map, "collect", &self.collect)?;
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

/// #1322 の暫定 fanout child。子は暗黙に session 扱いで、旧 `type:` と
/// flat facet は持たない。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct InterimChild {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
    #[serde(default, skip_serializing_if = "FacetRefs::is_empty")]
    pub facets: FacetRefs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
}

impl InterimChild {
    pub fn has_facet_refs(&self) -> bool {
        !self.facets.is_empty()
    }
}

/// parallel node 完了後の集約条件（#1330 まで fanout block 内で暫定維持）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ParallelAggregate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_match: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any_match: Option<String>,
    pub then: String,
    pub r#else: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WhenRule {
    pub on: String,
    pub then: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SwitchRule {
    pub on: String,
    pub cases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LoopGuardRule {
    pub max_iterations: u32,
    pub on_exhausted: String,
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
    },
    Next(String),
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
            } => {
                map.serialize_entry(
                    "loop_guard",
                    &LoopGuardRule {
                        max_iterations: *max_iterations,
                        on_exhausted: on_exhausted.clone(),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CollectConfig {
    pub from: Vec<String>,
    pub reduce: ReduceStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReduceStrategy {
    Last,
    Concat,
    Grouped,
    AnyNeedsFix,
    AllPassed,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    #[serde(default)]
    pub is_running: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FacetSummary {
    pub key: String,
    pub kind: String,
    pub description: String,
    pub builtin: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_node() {
        let yaml = r#"
name: session-only
description: 単一セッション
nodes:
  - name: implement
    session:
      model: test-model
      permission: edit
      gate: auto
      facets:
        instruction: implement
        policy: coding
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert_eq!(node.kind_name(), NodeKindName::Session);
        let session = node.session().unwrap();
        assert_eq!(session.facets.instruction.as_deref(), Some("implement"));
        assert_eq!(session.facets.policy.as_deref(), Some("coding"));
        assert_eq!(session.gate, SessionGate::Auto);
    }

    #[test]
    fn parse_approval_gate_session_node() {
        let yaml = r#"
name: approval-only
description: 承認セッション
nodes:
  - name: approve
    session:
      permission: ask
      gate: approval
      facets:
        instruction: approve
        policy: planning
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert!(node.is_approval_session());
        assert_eq!(
            node.session().unwrap().facets.instruction.as_deref(),
            Some("approve")
        );
    }

    #[test]
    fn parse_command_node() {
        let yaml = r#"
name: command-only
description: command node
nodes:
  - name: build
    command: "cargo build"
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let node = &wf.nodes[0];
        assert_eq!(node.kind_name(), NodeKindName::Command);
        match &node.kind {
            NodeKind::Command(spec) => assert_eq!(spec.command, "cargo build"),
            other => panic!("expected command node, got {other:?}"),
        }
    }

    #[test]
    fn parse_fanout_node_with_aggregate() {
        let yaml = r#"
name: fanout
description: fanout test
nodes:
  - name: review
    fanout:
      parallel_children:
        - name: arch-review
          model: test-model
          permission: edit
          facets:
            policy: review
            instruction: architecture-review
        - name: security-review
          model: test-model
          permission: edit
          facets:
            policy: review
            instruction: security-review
      aggregate:
        all_match: LGTM
        then: report
        else: implement
"#;
        let wf: Workflow = serde_saphyr::from_str(yaml).unwrap();
        let fanout = wf.nodes[0].fanout().unwrap();
        assert_eq!(fanout.parallel_children.len(), 2);
        assert_eq!(fanout.parallel_children[0].name, "arch-review");
        assert_eq!(
            fanout.parallel_children[0].facets.policy.as_deref(),
            Some("review")
        );
        let agg = fanout.aggregate.as_ref().unwrap();
        assert_eq!(agg.all_match.as_deref(), Some("LGTM"));
        assert!(agg.any_match.is_none());
        assert_eq!(agg.then, "report");
        assert_eq!(agg.r#else, "implement");
    }

    #[test]
    fn parse_rules_tagged_enum_shapes() {
        let yaml = r#"
name: rules
description: rules test
nodes:
  - name: judge
    session:
      permission: edit
    rules:
      - when: { on: ok, then: done }
        next: fix
      - loop_guard: { max_iterations: 3, on_exhausted: give_up }
  - name: triage
    session:
      permission: edit
    rules:
      - switch:
          on: verdict
          cases:
            LGTM: done
            NEEDS_FIX: fix
        next: give_up
      - next: done
"#;
        let wf = serde_saphyr::from_str::<Workflow>(yaml).unwrap();

        assert!(matches!(
            &wf.nodes[0].rules[0],
            Rule::When { on, then, next }
                if on == "ok" && then == "done" && next == "fix"
        ));
        assert!(matches!(
            &wf.nodes[0].rules[1],
            Rule::LoopGuard {
                max_iterations: 3,
                on_exhausted
            } if on_exhausted == "give_up"
        ));
        assert!(matches!(
            &wf.nodes[1].rules[0],
            Rule::Switch { on, cases, next }
                if on == "verdict"
                    && cases.get("LGTM").map(String::as_str) == Some("done")
                    && cases.get("NEEDS_FIX").map(String::as_str) == Some("fix")
                    && next.as_deref() == Some("give_up")
        ));
        assert!(matches!(
            &wf.nodes[1].rules[1],
            Rule::Next(next) if next == "done"
        ));
    }

    #[test]
    fn rejects_legacy_rule_match_key() {
        let yaml = r#"
name: legacy-rule
description: invalid
nodes:
  - name: review
    session:
      permission: edit
    rules:
      - match: NEEDS_FIX
        next: fix
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("match"));
    }

    #[test]
    fn rejects_rule_with_multiple_discriminators() {
        let yaml = r#"
name: invalid-rule
description: invalid
nodes:
  - name: review
    session:
      permission: edit
    rules:
      - when: { on: ok, then: done }
        switch:
          on: verdict
          cases:
            LGTM: done
        next: fix
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err
            .to_string()
            .contains("discriminator keys when, switch, and loop_guard are mutually exclusive"));
    }

    #[test]
    fn rejects_when_without_sibling_next() {
        let yaml = r#"
name: invalid-when
description: invalid
nodes:
  - name: review
    session:
      permission: edit
    rules:
      - when: { on: ok, then: done }
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("when rule requires sibling next"));
    }

    #[test]
    fn rejects_node_direct_cycle_guard_and_resets_cycle_for() {
        let yaml = r#"
name: legacy-guards
description: invalid
nodes:
  - name: fix
    session:
      permission: edit
    cycle_guard:
      max_iterations: 2
    resets_cycle_for:
      - fix
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(
            err.to_string().contains("cycle_guard") || err.to_string().contains("resets_cycle_for")
        );
    }

    #[test]
    fn rejects_missing_kind_block() {
        let yaml = r#"
name: invalid
description: invalid
nodes:
  - name: missing
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("exactly one kind block"));
    }

    #[test]
    fn rejects_multiple_kind_blocks() {
        let yaml = r#"
name: invalid
description: invalid
nodes:
  - name: duplicate
    command: "echo hi"
    session:
      permission: edit
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("exactly one kind block"));
    }

    #[test]
    fn rejects_legacy_type_field() {
        let yaml = r#"
name: old-type
description: invalid
nodes:
  - name: implement
    type: agent
    instruction: implement
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("type"));
    }

    #[test]
    fn rejects_legacy_output_contract_field() {
        let yaml = r#"
name: old-output-contract
description: invalid
nodes:
  - name: review
    session:
      permission: edit
    output_contract: review-verdict
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("output_contract"));
    }

    #[test]
    fn rejects_legacy_input_contracts_field() {
        let yaml = r#"
name: old-input-contracts
description: invalid
nodes:
  - name: implement
    session:
      permission: edit
    input_contracts:
      - spec-directory
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("input_contracts"));
    }

    #[test]
    fn schemas_accept_scalar_string_contract() {
        let yaml = r#"
name: scalar-schema
description: valid
schemas:
  request_text: string
nodes:
  - name: review
    session:
      permission: edit
    input: request_text
"#;
        let workflow = serde_saphyr::from_str::<Workflow>(yaml).unwrap();
        assert!(matches!(
            workflow.schemas.get("request_text"),
            Some(SchemaDef::String { r#enum: None })
        ));
    }

    #[test]
    fn schemas_serde_matches_domain_schema_helper_for_supported_shapes() {
        for value in [
            serde_json::json!("string"),
            serde_json::json!({"type": "string", "enum": ["LGTM", "NEEDS_FIX"]}),
            serde_json::json!({"type": "array", "items": "review-item"}),
            serde_json::json!({"type": "boolean"}),
            serde_json::json!({"type": "integer"}),
            serde_json::json!({"type": "number"}),
            serde_json::json!({
                "type": "object",
                "properties": {"verdict": {"type": "string", "enum": ["LGTM"]}},
                "required": ["verdict"],
                "additionalProperties": false
            }),
        ] {
            let gateway_schema: SchemaDef = serde_json::from_value(value.clone()).unwrap();
            let domain_schema = contract_schema::schema_def_from_json(&value).unwrap();
            assert_eq!(
                domain_mapping::schema_def_to_domain(&gateway_schema),
                domain_schema
            );
        }
    }

    #[test]
    fn schemas_reject_array_extra_keywords_with_allowed_field_message() {
        let err = serde_json::from_value::<SchemaDef>(serde_json::json!({
            "type": "array",
            "items": "review-item",
            "required": []
        }))
        .unwrap_err();

        assert!(err.to_string().contains("array schema supports only items"));
    }

    #[test]
    fn schemas_reject_subset_outside_keywords() {
        let yaml = r#"
name: invalid-schema-keyword
description: invalid
schemas:
  review:
    type: object
    oneOf: []
nodes:
  - name: review
    session:
      permission: edit
"#;
        assert!(serde_saphyr::from_str::<Workflow>(yaml).is_err());
    }

    #[test]
    fn rejects_flat_session_facets() {
        let yaml = r#"
name: flat-facet
description: invalid
nodes:
  - name: implement
    session:
      permission: edit
    instruction: implement
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("instruction"));
    }

    #[test]
    fn rejects_inline_prompt() {
        let yaml = r#"
name: inline-test
description: invalid
nodes:
  - name: quick
    session:
      permission: edit
    inline_prompt: "Do a quick analysis"
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("inline_prompt"));
    }

    #[test]
    fn rejects_session_block_command_field() {
        let yaml = r#"
name: invalid-session
description: invalid
nodes:
  - name: implement
    session:
      permission: edit
      command: "cargo build"
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("command"));
    }

    #[test]
    fn rejects_command_spec_session_fields() {
        let yaml = r#"
command: "cargo build"
facets:
  instruction: implement
"#;
        let err = serde_saphyr::from_str::<CommandSpec>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("facets"));
    }

    #[test]
    fn rejects_command_node_session_fields() {
        let yaml = r#"
name: invalid-command
description: invalid
nodes:
  - name: build
    command: "cargo build"
    facets:
      instruction: implement
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("facets"));
    }

    #[test]
    fn rejects_fanout_block_session_fields() {
        let yaml = r#"
name: invalid-fanout
description: invalid
nodes:
  - name: review
    fanout:
      facets:
        instruction: review
      parallel_children: []
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("facets"));
    }

    #[test]
    fn rejects_fanout_child_legacy_type_field() {
        let yaml = r#"
name: invalid-child-type
description: invalid
nodes:
  - name: review
    fanout:
      parallel_children:
        - name: child
          type: agent
          permission: edit
          facets:
            instruction: review
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("type"));
    }

    #[test]
    fn rejects_fanout_child_flat_facet_fields() {
        let yaml = r#"
name: invalid-child-flat-facet
description: invalid
nodes:
  - name: review
    fanout:
      parallel_children:
        - name: child
          permission: edit
          policy: review
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("policy"));
    }

    #[test]
    fn rejects_fanout_child_unknown_field() {
        let yaml = r#"
name: invalid-child-unknown
description: invalid
nodes:
  - name: review
    fanout:
      parallel_children:
        - name: child
          permission: edit
          unexpected: value
"#;
        let err = serde_saphyr::from_str::<Workflow>(yaml).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("unexpected"));
    }
}
