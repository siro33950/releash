use std::collections::{BTreeMap, BTreeSet};

use serde::de;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::services::contract_schema;

use super::FieldPath;

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
    "env",
    "worktree",
    "inputs",
    "rules",
    "on_failure",
    "items",
    "entry",
    "children",
];

pub fn is_reserved_node_name(name: &str) -> bool {
    RESERVED_NODE_NAMES.contains(&name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentVariableNameError {
    Invalid(String),
    Reserved(String),
}

impl std::fmt::Display for EnvironmentVariableNameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(name) => write!(
                formatter,
                "environment variable name '{name}' must match [A-Za-z_][A-Za-z0-9_]*"
            ),
            Self::Reserved(name) => write!(
                formatter,
                "environment variable name '{name}' is reserved for the workflow engine"
            ),
        }
    }
}

impl std::error::Error for EnvironmentVariableNameError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnvironmentVariableName(String);

impl EnvironmentVariableName {
    pub fn new(name: impl Into<String>) -> Result<Self, EnvironmentVariableNameError> {
        let name = name.into();
        let mut characters = name.chars();
        let valid = characters
            .next()
            .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
            && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
        if !valid {
            return Err(EnvironmentVariableNameError::Invalid(name));
        }
        if name.starts_with("RELEASH_") {
            return Err(EnvironmentVariableNameError::Reserved(name));
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for EnvironmentVariableName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for EnvironmentVariableName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputParameterRef {
    parameter: String,
    field_path: FieldPath,
}

impl InputParameterRef {
    pub fn new(reference: impl AsRef<str>) -> Result<Self, String> {
        let reference = reference.as_ref();
        let Ok((parameter, field_path)) = FieldPath::from_reference(reference) else {
            return Err(format!(
                "input parameter reference '{reference}' must be `<parameter>` or `<parameter>.<field>...`"
            ));
        };
        Ok(Self {
            parameter,
            field_path,
        })
    }

    pub fn parameter(&self) -> &str {
        &self.parameter
    }

    pub fn field_path(&self) -> &FieldPath {
        &self.field_path
    }

    pub fn as_string(&self) -> String {
        self.field_path
            .to_reference(&self.parameter)
            .expect("InputParameterRef parameter is validated at construction")
    }
}

impl Serialize for InputParameterRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.as_string())
    }
}

impl<'de> Deserialize<'de> for InputParameterRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeNamespaceError {
    Duplicate(String),
    Reserved(String),
}

impl std::fmt::Display for NodeNamespaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(name) => write!(formatter, "node name '{name}' is duplicated"),
            Self::Reserved(name) => write!(formatter, "node name '{name}' is reserved"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeNamespace {
    names: BTreeSet<String>,
}

impl NodeNamespace {
    pub fn register(&mut self, name: impl Into<String>) -> Result<String, NodeNamespaceError> {
        let name = name.into();
        if !self.names.insert(name.clone()) {
            return Err(NodeNamespaceError::Duplicate(name));
        }
        Ok(name)
    }

    pub fn register_explicit(
        &mut self,
        name: impl Into<String>,
    ) -> Result<String, NodeNamespaceError> {
        let name = name.into();
        if is_reserved_node_name(&name) {
            return Err(NodeNamespaceError::Reserved(name));
        }
        self.register(name)
    }

    pub fn register_synthesized(
        &mut self,
        owner: &str,
        child_index: usize,
    ) -> Result<String, NodeNamespaceError> {
        self.register(format!("{owner}#{child_index}"))
    }

    #[cfg(test)]
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }
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

    pub fn node_by_name(&self, name: &str) -> Option<&NodeDefinition> {
        self.nodes.iter().find(|node| node.name == name)
    }

    /// root node が sequence の場合の children（テスト用の互換入口）。
    #[cfg(test)]
    pub fn root_sequence(&self) -> Option<&SequenceSpec> {
        self.entry_node().and_then(NodeDefinition::sequence)
    }

    /// 実行開始 node。root が sequence なら実効 entry（entry 指定 or children
    /// 先頭）の子、それ以外は root 自身。
    pub fn initial_execution_node_index(&self) -> Option<usize> {
        let entry_index = self.entry_index()?;
        match self.nodes[entry_index].sequence() {
            Some(sequence) => {
                let name = sequence.entry_child_name()?;
                self.nodes.iter().position(|node| node.name == name)
            }
            None => Some(entry_index),
        }
    }

    pub fn initial_execution_node(&self) -> Option<&NodeDefinition> {
        self.initial_execution_node_index()
            .map(|index| &self.nodes[index])
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
    Sequence,
}

impl NodeKindName {
    pub fn is_composite_kind(self) -> bool {
        matches!(self, Self::Fanout | Self::Sequence)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Command(CommandSpec),
    Session(SessionSpec),
    Fanout(FanoutSpec),
    Sequence(SequenceSpec),
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
            Self::Sequence(_) => NodeKindName::Sequence,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub command: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<EnvironmentVariableName, InputParameterRef>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionPermission {
    Manual,
    Auto,
    Bypass,
    ReadOnly,
}

impl SessionPermission {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
            Self::Bypass => "bypass",
            Self::ReadOnly => "read-only",
        }
    }
}

impl std::fmt::Display for SessionPermission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for SessionPermission {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "manual" => Ok(Self::Manual),
            "auto" => Ok(Self::Auto),
            "bypass" => Ok(Self::Bypass),
            "read-only" => Ok(Self::ReadOnly),
            _ => Err(format!(
                "invalid session permission '{value}'; expected one of: manual, auto, bypass, read-only"
            )),
        }
    }
}

impl Serialize for SessionPermission {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SessionPermission {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<SessionPermission>,
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

/// fanout 合成子。children は sequence と同形式の children エントリのリスト。
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct FanoutSpec {
    pub children: Vec<ChildEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<ItemsSource>,
}

/// sequence 合成子。children エントリの並びが隣接辺（rules 省略時の既定辺）を定める。
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct SequenceSpec {
    /// 開始 node（children のエントリ名）。省略時はリスト先頭。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    pub children: Vec<ChildEntry>,
}

/// children エントリの実効辺。
#[derive(Debug, Clone, PartialEq)]
pub enum EffectiveRules<'a> {
    /// エントリに明示された rules（空スライス = 明示終端）。
    Rules(&'a [Rule]),
    /// rules 省略時の隣接辺（リストの次のエントリへ）。
    AdjacentNext(&'a str),
    /// リスト末尾（隣接辺なし）、または children に載らない node。
    Terminal,
}

impl SequenceSpec {
    /// 実効 entry（entry 指定 or children 先頭）。
    pub fn entry_child_name(&self) -> Option<&str> {
        self.entry
            .as_deref()
            .or_else(|| self.children.first().map(|child| child.name.as_str()))
    }

    pub fn child_entry(&self, name: &str) -> Option<&ChildEntry> {
        self.children.iter().find(|child| child.name == name)
    }

    /// エントリの実効辺。rules 明示があればそれ（空 = 終端）、無ければ隣接辺、
    /// 末尾・children 外は終端。
    pub fn effective_rules(&self, name: &str) -> EffectiveRules<'_> {
        let Some(index) = self.children.iter().position(|child| child.name == name) else {
            return EffectiveRules::Terminal;
        };
        match &self.children[index].rules {
            Some(rules) => EffectiveRules::Rules(rules),
            None => match self.children.get(index + 1) {
                Some(next) => EffectiveRules::AdjacentNext(next.name.as_str()),
                None => EffectiveRules::Terminal,
            },
        }
    }
}

/// children エントリの on_failure（この子が失敗したときの扱い）。
/// 宣言なし（`ChildEntry.on_failure = None`）は中断（resume / 手動 Retry 待ち）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    /// 失敗しても続行する。fanout では失敗子を結果の配列から除く。
    Ignore,
    /// 新しい attempt で最大 n 回自動再実行し、尽きたら既定（中断）へ。
    Retry(u32),
}

impl Serialize for OnFailure {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Ignore => serializer.serialize_str("ignore"),
            Self::Retry(count) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("retry", count)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for OnFailure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OnFailureVisitor;

        impl<'de> de::Visitor<'de> for OnFailureVisitor {
            type Value = OnFailure;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("\"ignore\" or a mapping with a single `retry: <n>` field")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    "ignore" => Ok(OnFailure::Ignore),
                    other => Err(de::Error::custom(format!(
                        "on_failure must be `ignore` or `retry: <n>`, got `{other}`"
                    ))),
                }
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let Some(key) = access.next_key::<String>()? else {
                    return Err(de::Error::custom(
                        "on_failure must be `ignore` or `retry: <n>`",
                    ));
                };
                if key != "retry" {
                    return Err(de::Error::custom(format!(
                        "on_failure must be `ignore` or `retry: <n>`, got field `{key}`"
                    )));
                }
                let count = access.next_value::<u32>()?;
                if count == 0 {
                    return Err(de::Error::custom(
                        "on_failure retry count must be at least 1",
                    ));
                }
                if access.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(
                        "on_failure retry must be the only field in its mapping",
                    ));
                }
                Ok(OnFailure::Retry(count))
            }
        }

        deserializer.deserialize_any(OnFailureVisitor)
    }
}

/// 合成子（sequence / fanout）の children エントリ（正規化後）。
/// インライン宣言・無名エントリは load 時にカタログへ登録され、エントリは常に
/// カタログ参照名を持つ。
#[derive(Debug, Clone, PartialEq)]
pub struct ChildEntry {
    /// カタログ参照名。無名エントリは合成内部名（`<合成子名>#<index>`）。
    pub name: String,
    /// `<パラメータ名>: <供給元>`。YAML の記述順を保持する。
    pub inputs: Vec<(String, InputSourceRef)>,
    /// None = 隣接辺 auto（リストの次へ、末尾は終端）。Some(空) = 明示終端。
    pub rules: Option<Vec<Rule>>,
    /// None = 既定（中断して resume / 手動 Retry 待ち）。
    pub on_failure: Option<OnFailure>,
}

impl ChildEntry {
    pub fn reference(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            inputs: Vec::new(),
            rules: None,
            on_failure: None,
        }
    }

    fn has_treatment(&self) -> bool {
        !self.inputs.is_empty() || self.rules.is_some() || self.on_failure.is_some()
    }
}

impl Serialize for ChildEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if !self.has_treatment() {
            return serializer.serialize_str(&self.name);
        }
        struct Treatment<'a>(&'a ChildEntry);
        impl Serialize for Treatment<'_> {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let mut map = serializer.serialize_map(None)?;
                if !self.0.inputs.is_empty() {
                    map.serialize_entry("inputs", &InputsMap(&self.0.inputs))?;
                }
                if let Some(rules) = &self.0.rules {
                    map.serialize_entry("rules", rules)?;
                }
                if let Some(on_failure) = &self.0.on_failure {
                    map.serialize_entry("on_failure", on_failure)?;
                }
                map.end()
            }
        }
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(&self.name, &Treatment(self))?;
        map.end()
    }
}

struct InputsMap<'a>(&'a [(String, InputSourceRef)]);

impl Serialize for InputsMap<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (parameter, source) in self.0 {
            map.serialize_entry(parameter, source)?;
        }
        map.end()
    }
}

/// children エントリの inputs 供給元。分類（request / items / 兄弟 / 自パラメータ）
/// はスコープ文脈が要るため検証・束縛時に行い、ここでは記述そのものを保持する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSourceRef(String);

impl InputSourceRef {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn raw(&self) -> &str {
        &self.0
    }

    /// 最初の `.` より前（field パスなしなら全体）。
    pub fn root(&self) -> &str {
        self.0.split('.').next().unwrap_or(&self.0)
    }
}

impl Serialize for InputSourceRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for InputSourceRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        if raw.trim().is_empty() {
            return Err(de::Error::custom("inputs source must not be empty"));
        }
        Ok(Self::new(raw))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemsSource {
    Literal(Vec<Value>),
    ArtifactField { node: String, field_path: FieldPath },
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
            RawItemsSource::ArtifactField(value) => match FieldPath::from_reference(&value) {
                Ok((node, field_path)) if !field_path.is_empty() => {
                    Ok(Self::ArtifactField { node, field_path })
                }
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
            Self::ArtifactField { node, field_path } => {
                serializer.serialize_str(&format!("{node}.{}", field_path.as_string()))
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
    pub completion: NodeCompletion,
    /// 未対応（#85 まで）。受理して保持し、load 時に Diagnostic を出す。
    pub worktree: Option<String>,
}

/// nodes マップの値（node 名は親マップのキー）。配線（inputs / rules）は持たない。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNodeBody {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    env: Option<BTreeMap<EnvironmentVariableName, InputParameterRef>>,
    #[serde(default)]
    session: Option<SessionSpec>,
    #[serde(default)]
    fanout: Option<RawFanoutSpec>,
    #[serde(default)]
    sequence: Option<RawSequenceSpec>,
    #[serde(default)]
    artifact: Option<String>,
    #[serde(default)]
    input: Vec<InputParam>,
    #[serde(default)]
    completion: NodeCompletion,
    #[serde(default)]
    worktree: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFanoutSpec {
    children: Vec<RawChildElement>,
    #[serde(default)]
    items: Option<ItemsSource>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSequenceSpec {
    #[serde(default)]
    entry: Option<String>,
    children: Vec<RawChildElement>,
}

/// children リスト要素の4形式（①文字列参照 / ②名前+扱い / ③インライン宣言 / ④無名）。
#[derive(Debug, Clone)]
enum RawChildElement {
    Reference(String),
    Entry {
        name: Option<String>,
        body: Box<RawChildBody>,
    },
}

/// children エントリの本体。node 系フィールド（カタログへ分配）と
/// 扱い系フィールド（inputs / rules / on_failure。エントリに残る）を併せて受ける。
#[derive(Debug, Clone, Default)]
struct RawChildBody {
    node: RawNodeBody,
    inputs: Option<Vec<(String, InputSourceRef)>>,
    rules: Option<Vec<Rule>>,
    on_failure: Option<OnFailure>,
    // input / completion は RawNodeBody 側の既定値と区別できないため、
    // 重複キー検出は観測フラグで行う（serde_json 直接経路では上流に
    // 重複キー拒否が無く、この関数が唯一の防御になる）。
    input_seen: bool,
    completion_seen: bool,
}

impl RawChildBody {
    fn has_kind(&self) -> bool {
        self.node.command.is_some()
            || self.node.session.is_some()
            || self.node.fanout.is_some()
            || self.node.sequence.is_some()
    }

    fn has_node_fields(&self) -> bool {
        self.node.env.is_some()
            || self.node.artifact.is_some()
            || !self.node.input.is_empty()
            || self.node.completion != NodeCompletion::default()
            || self.node.worktree.is_some()
    }
}

fn apply_child_body_field<'de, A>(
    body: &mut RawChildBody,
    key: &str,
    map: &mut A,
) -> Result<(), A::Error>
where
    A: de::MapAccess<'de>,
{
    fn set_once<'de, A, T>(map: &mut A, slot: &mut Option<T>, key: &str) -> Result<(), A::Error>
    where
        A: de::MapAccess<'de>,
        T: Deserialize<'de>,
    {
        if slot.is_some() {
            return Err(de::Error::custom(format!("duplicate field `{key}`")));
        }
        *slot = Some(map.next_value()?);
        Ok(())
    }

    match key {
        "command" => set_once(map, &mut body.node.command, key),
        "env" => set_once(map, &mut body.node.env, key),
        "session" => set_once(map, &mut body.node.session, key),
        "fanout" => set_once(map, &mut body.node.fanout, key),
        "sequence" => set_once(map, &mut body.node.sequence, key),
        "artifact" => set_once(map, &mut body.node.artifact, key),
        "worktree" => set_once(map, &mut body.node.worktree, key),
        "input" => {
            if body.input_seen {
                return Err(de::Error::custom("duplicate field `input`"));
            }
            body.input_seen = true;
            body.node.input = map.next_value()?;
            Ok(())
        }
        "completion" => {
            if body.completion_seen {
                return Err(de::Error::custom("duplicate field `completion`"));
            }
            body.completion_seen = true;
            body.node.completion = map.next_value()?;
            Ok(())
        }
        "inputs" => {
            if body.inputs.is_some() {
                return Err(de::Error::custom("duplicate field `inputs`"));
            }
            body.inputs = Some(map.next_value_seed(InputsMapSeed)?);
            Ok(())
        }
        "rules" => set_once(map, &mut body.rules, key),
        "on_failure" => set_once(map, &mut body.on_failure, key),
        unknown => Err(de::Error::custom(format!(
            "unknown field `{unknown}` in children entry"
        ))),
    }
}

struct InputsMapSeed;

impl<'de> de::DeserializeSeed<'de> for InputsMapSeed {
    type Value = Vec<(String, InputSourceRef)>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct InputsVisitor;

        impl<'de> de::Visitor<'de> for InputsVisitor {
            type Value = Vec<(String, InputSourceRef)>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a mapping of parameter name to input source")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut entries: Vec<(String, InputSourceRef)> = Vec::new();
                while let Some((parameter, source)) =
                    access.next_entry::<String, InputSourceRef>()?
                {
                    if entries.iter().any(|(existing, _)| *existing == parameter) {
                        return Err(de::Error::custom(format!(
                            "inputs parameter '{parameter}' is duplicated"
                        )));
                    }
                    entries.push((parameter, source));
                }
                Ok(entries)
            }
        }

        deserializer.deserialize_map(InputsVisitor)
    }
}

impl<'de> Deserialize<'de> for RawChildBody {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BodyVisitor;

        impl<'de> de::Visitor<'de> for BodyVisitor {
            type Value = RawChildBody;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a children entry body mapping")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut body = RawChildBody::default();
                while let Some(key) = access.next_key::<String>()? {
                    apply_child_body_field(&mut body, &key, &mut access)?;
                }
                Ok(body)
            }
        }

        deserializer.deserialize_map(BodyVisitor)
    }
}

impl<'de> Deserialize<'de> for RawChildElement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ElementVisitor;

        impl<'de> de::Visitor<'de> for ElementVisitor {
            type Value = RawChildElement;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a node name or a children entry mapping")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(RawChildElement::Reference(value.to_string()))
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let Some(first_key) = access.next_key::<String>()? else {
                    return Err(de::Error::custom("children entry must not be empty"));
                };
                if is_reserved_node_name(&first_key) {
                    // ④ 無名エントリ: 予約語キー始まりのマップ全体が本体。
                    let mut body = RawChildBody::default();
                    apply_child_body_field(&mut body, &first_key, &mut access)?;
                    while let Some(key) = access.next_key::<String>()? {
                        apply_child_body_field(&mut body, &key, &mut access)?;
                    }
                    Ok(RawChildElement::Entry {
                        name: None,
                        body: Box::new(body),
                    })
                } else {
                    // ②③ 名前付きエントリ: 単一の名前キーの値が本体。
                    let body = access.next_value::<RawChildBody>()?;
                    if access.next_key::<de::IgnoredAny>()?.is_some() {
                        return Err(de::Error::custom(format!(
                            "children entry '{first_key}' must be the only key in its mapping"
                        )));
                    }
                    Ok(RawChildElement::Entry {
                        name: Some(first_key),
                        body: Box::new(body),
                    })
                }
            }
        }

        deserializer.deserialize_any(ElementVisitor)
    }
}

/// 正規化の作業状態。単一名前空間の登録簿と、カタログへ追記するインライン宣言。
struct CatalogNormalizer {
    namespace: NodeNamespace,
    inline_nodes: Vec<NodeDefinition>,
}

impl CatalogNormalizer {
    fn new(top_level_names: BTreeSet<String>) -> Self {
        let mut namespace = NodeNamespace::default();
        for name in top_level_names {
            namespace
                .register(name)
                .expect("top-level node names were already deduplicated");
        }
        Self {
            namespace,
            inline_nodes: Vec::new(),
        }
    }

    fn register<E>(&mut self, name: &str) -> Result<(), E>
    where
        E: de::Error,
    {
        self.namespace.register(name).map(|_| ()).map_err(E::custom)
    }

    fn normalize_children<E>(
        &mut self,
        owner: &str,
        elements: Vec<RawChildElement>,
    ) -> Result<Vec<ChildEntry>, E>
    where
        E: de::Error,
    {
        let mut children = Vec::with_capacity(elements.len());
        for (index, element) in elements.into_iter().enumerate() {
            match element {
                RawChildElement::Reference(name) => children.push(ChildEntry::reference(name)),
                RawChildElement::Entry { name, body } => {
                    let has_kind = body.has_kind();
                    let RawChildBody {
                        node,
                        inputs,
                        rules,
                        on_failure,
                        ..
                    } = *body;
                    let entry_name = match (&name, has_kind) {
                        // ② 参照 + 扱い: node 系フィールドは書けない。
                        (Some(reference_name), false) => {
                            let placeholder = RawChildBody {
                                node: node.clone(),
                                ..RawChildBody::default()
                            };
                            if placeholder.has_node_fields() {
                                return Err(E::custom(format!(
                                    "children entry '{reference_name}' without a kind block can only declare inputs and rules"
                                )));
                            }
                            reference_name.clone()
                        }
                        // ③ インライン宣言: 単一名前空間へ登録し参照へ正規化。
                        (Some(inline_name), true) => {
                            self.register(inline_name)?;
                            let node_definition =
                                self.raw_body_to_node(inline_name.clone(), node)?;
                            self.inline_nodes.push(node_definition);
                            inline_name.clone()
                        }
                        // ④ 無名エントリ: 合成内部名を生成して登録。
                        (None, true) => {
                            let synthesized = self
                                .namespace
                                .register_synthesized(owner, index)
                                .map_err(E::custom)?;
                            let node_definition =
                                self.raw_body_to_node(synthesized.clone(), node)?;
                            self.inline_nodes.push(node_definition);
                            synthesized
                        }
                        (None, false) => {
                            return Err(E::custom(format!(
                                "children entry of '{owner}' must contain a kind block or reference a catalog node by name"
                            )));
                        }
                    };
                    children.push(ChildEntry {
                        name: entry_name,
                        inputs: inputs.unwrap_or_default(),
                        rules,
                        on_failure,
                    });
                }
            }
        }
        Ok(children)
    }

    fn raw_body_to_node<E>(&mut self, name: String, body: RawNodeBody) -> Result<NodeDefinition, E>
    where
        E: de::Error,
    {
        let kind_count = body.command.is_some() as usize
            + body.session.is_some() as usize
            + body.fanout.is_some() as usize
            + body.sequence.is_some() as usize;
        if kind_count != 1 {
            return Err(E::custom(format!(
                "node '{name}' must contain exactly one kind block: command, session, fanout, or sequence"
            )));
        }
        let kind = if let Some(command) = body.command {
            NodeKind::Command(CommandSpec {
                command,
                env: body.env.unwrap_or_default(),
            })
        } else if let Some(session) = body.session {
            if body.env.is_some() {
                return Err(E::custom("env can only be declared by command nodes"));
            }
            NodeKind::Session(session)
        } else if let Some(fanout) = body.fanout {
            if body.env.is_some() {
                return Err(E::custom("env can only be declared by command nodes"));
            }
            NodeKind::Fanout(FanoutSpec {
                children: self.normalize_children(&name, fanout.children)?,
                items: fanout.items,
            })
        } else if let Some(sequence) = body.sequence {
            if body.env.is_some() {
                return Err(E::custom("env can only be declared by command nodes"));
            }
            NodeKind::Sequence(SequenceSpec {
                entry: sequence.entry,
                children: self.normalize_children(&name, sequence.children)?,
            })
        } else {
            unreachable!("kind_count checked above")
        };
        Ok(NodeDefinition {
            name,
            kind,
            artifact: body.artifact,
            input: body.input,
            completion: body.completion,
            worktree: body.worktree,
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
            let mut raw_nodes: Vec<(String, RawNodeBody)> = Vec::new();
            let mut seen = BTreeSet::new();
            while let Some((name, body)) = access.next_entry::<String, RawNodeBody>()? {
                if !seen.insert(name.clone()) {
                    return Err(de::Error::custom(format!(
                        "node name '{name}' is duplicated"
                    )));
                }
                raw_nodes.push((name, body));
            }

            // 正規化: カタログ + 参照へ。インライン宣言・無名エントリは
            // 単一名前空間へ登録され、宣言順にカタログ末尾へ並ぶ。
            let mut normalizer = CatalogNormalizer::new(seen);
            let mut nodes = Vec::with_capacity(raw_nodes.len());
            for (name, body) in raw_nodes {
                nodes.push(normalizer.raw_body_to_node(name, body)?);
            }
            nodes.extend(normalizer.inline_nodes);
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
            NodeKind::Command(spec) => {
                map.serialize_entry("command", &spec.command)?;
                if !spec.env.is_empty() {
                    map.serialize_entry("env", &spec.env)?;
                }
            }
            NodeKind::Session(spec) => map.serialize_entry("session", spec)?,
            NodeKind::Fanout(spec) => map.serialize_entry("fanout", spec)?,
            NodeKind::Sequence(spec) => map.serialize_entry("sequence", spec)?,
        }
        if !self.input.is_empty() {
            map.serialize_entry("input", &self.input)?;
        }
        serialize_option(&mut map, "artifact", &self.artifact)?;
        if !self.completion.is_auto() {
            map.serialize_entry("completion", &self.completion)?;
        }
        serialize_option(&mut map, "worktree", &self.worktree)?;
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

    pub fn is_fanout(&self) -> bool {
        matches!(self.kind, NodeKind::Fanout(_))
    }

    pub fn is_sequence(&self) -> bool {
        matches!(self.kind, NodeKind::Sequence(_))
    }

    pub fn is_composite(&self) -> bool {
        self.is_fanout() || self.is_sequence()
    }

    pub fn command(&self) -> Option<&str> {
        self.command_spec().map(|spec| spec.command.as_str())
    }

    pub fn command_spec(&self) -> Option<&CommandSpec> {
        match &self.kind {
            NodeKind::Command(spec) => Some(spec),
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

    pub fn sequence(&self) -> Option<&SequenceSpec> {
        match &self.kind {
            NodeKind::Sequence(spec) => Some(spec),
            _ => None,
        }
    }

    /// 宣言済み input パラメータ名の一覧。
    pub fn input_parameter_names(&self) -> impl Iterator<Item = &str> {
        self.input.iter().map(|param| param.name.as_str())
    }

    pub fn input_parameter(&self, name: &str) -> Option<&InputParam> {
        self.input.iter().find(|param| param.name == name)
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

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowSummary {
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub is_running: bool,
    pub source_format: WorkflowSourceFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowSourceFormat {
    #[default]
    Yaml,
    Lua,
}

#[cfg(test)]
mod definition_tests {
    use super::*;

    #[test]
    fn test_環境変数名_形式とengine予約prefixを検証する() {
        assert_eq!(
            EnvironmentVariableName::new("DOC_2").unwrap().as_str(),
            "DOC_2"
        );
        assert_eq!(
            EnvironmentVariableName::new("_DOC").unwrap().as_str(),
            "_DOC"
        );
        assert!(matches!(
            EnvironmentVariableName::new("2DOC"),
            Err(EnvironmentVariableNameError::Invalid(_))
        ));
        assert!(matches!(
            EnvironmentVariableName::new("DOC-NAME"),
            Err(EnvironmentVariableNameError::Invalid(_))
        ));
        assert!(matches!(
            EnvironmentVariableName::new("RELEASH_WORKTREE_PATH"),
            Err(EnvironmentVariableNameError::Reserved(_))
        ));
    }

    #[test]
    fn test_inputパラメータ参照_パラメータと多段fieldを受理する() {
        let parameter = InputParameterRef::new("document").unwrap();
        assert_eq!(parameter.parameter(), "document");
        assert!(parameter.field_path().is_empty());

        let field = InputParameterRef::new("document.body.text").unwrap();
        assert_eq!(field.parameter(), "document");
        assert_eq!(field.field_path().segments(), ["body", "text"]);
        assert_eq!(field.as_string(), "document.body.text");
        assert!(InputParameterRef::new("document bad").is_err());
    }

    #[test]
    fn test_command_env_空mapを省略し既存snapshotを空mapとして読む() {
        let snapshot = r#"{
            "name":"wf",
            "description":"",
            "nodes":{"main":{"command":"true"}}
        }"#;

        let workflow = serde_json::from_str::<WorkflowDefinition>(snapshot).unwrap();
        let command = workflow.nodes[0].command_spec().unwrap();
        assert!(command.env.is_empty());

        let serialized = serde_json::to_value(&workflow).unwrap();
        assert_eq!(serialized["nodes"]["main"]["command"], "true");
        assert!(serialized["nodes"]["main"].get("env").is_none());
    }

    #[test]
    fn test_command_env_宣言をdefinition_snapshotで往復する() {
        let workflow = serde_saphyr::from_str::<WorkflowDefinition>(
            r#"name: wf
description: test
nodes:
  main:
    command: printf
    env:
      DOC: document.body
    input:
      - document
"#,
        )
        .unwrap();

        let serialized = serde_json::to_string(&workflow).unwrap();
        let restored = serde_json::from_str::<WorkflowDefinition>(&serialized).unwrap();

        assert_eq!(restored, workflow);
        assert_eq!(
            restored.nodes[0]
                .command_spec()
                .unwrap()
                .env
                .get(&EnvironmentVariableName::new("DOC").unwrap())
                .map(InputParameterRef::as_string),
            Some("document.body".to_string())
        );
    }

    #[test]
    fn test_fanout_items_多段artifact参照を直列化表記で往復する() {
        // Given
        let source = "plan.payload.targets";

        // When
        let items =
            serde_json::from_value::<ItemsSource>(Value::String(source.to_string())).unwrap();
        let serialized = serde_json::to_value(&items).unwrap();

        // Then
        assert_eq!(
            items,
            ItemsSource::ArtifactField {
                node: "plan".to_string(),
                field_path: FieldPath::new(["payload", "targets"]),
            }
        );
        assert_eq!(serialized, Value::String(source.to_string()));
    }

    #[test]
    fn test_session_permission_4値は文字列構築とserdeで同じ値を往復する() {
        let cases = [
            ("manual", SessionPermission::Manual),
            ("auto", SessionPermission::Auto),
            ("bypass", SessionPermission::Bypass),
            ("read-only", SessionPermission::ReadOnly),
        ];

        for (serialized, expected) in cases {
            assert_eq!(serialized.parse::<SessionPermission>().unwrap(), expected);
            assert_eq!(expected.as_str(), serialized);
            assert_eq!(
                serde_json::to_string(&expected).unwrap(),
                format!("\"{serialized}\"")
            );
            assert_eq!(
                serde_json::from_str::<SessionPermission>(&format!("\"{serialized}\"")).unwrap(),
                expected
            );
            assert_eq!(
                serde_saphyr::from_str::<SessionPermission>(serialized).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn test_session_permission_未知値とprovider固有値は文字列構築とserdeで拒否する() {
        for invalid in [
            "unknown",
            "acceptEdits",
            "danger-full-access",
            "workspace-write",
            "bypassPermissions",
            "plan",
        ] {
            assert!(invalid.parse::<SessionPermission>().is_err());
            assert!(serde_json::from_str::<SessionPermission>(&format!("\"{invalid}\"")).is_err());
            assert!(serde_saphyr::from_str::<SessionPermission>(invalid).is_err());
        }
    }

    // serde_json の直接デシリアライズ（イベント payload 復元経路）は上流に
    // 重複キー拒否が無いため、children エントリ本体の全フィールドが
    // 自前の重複検出で守られていることを検証する。
    #[test]
    fn test_子エントリ本体は重複completionキーを拒否する() {
        let error = serde_json::from_str::<RawChildBody>(
            r#"{"command":"echo hi","completion":"auto","completion":"approval"}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate field `completion`"));
    }

    #[test]
    fn test_子エントリ本体は空リスト後の重複inputキーを拒否する() {
        let error = serde_json::from_str::<RawChildBody>(
            r#"{"command":"echo hi","input":[],"input":["spec"]}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate field `input`"));
    }

    #[test]
    fn test_onfailure_ignoreとretryが子エントリの扱いとしてパースされる() {
        let body = serde_json::from_str::<RawChildBody>(r#"{"on_failure":"ignore"}"#).unwrap();
        assert_eq!(body.on_failure, Some(OnFailure::Ignore));

        let body = serde_json::from_str::<RawChildBody>(r#"{"on_failure":{"retry":3}}"#).unwrap();
        assert_eq!(body.on_failure, Some(OnFailure::Retry(3)));
    }

    #[test]
    fn test_onfailure_不正な値を拒否する() {
        let error = serde_json::from_str::<OnFailure>(r#""abort""#).unwrap_err();
        assert!(error.to_string().contains("on_failure must be"));

        let error = serde_json::from_str::<OnFailure>(r#"{"retry":0}"#).unwrap_err();
        assert!(error.to_string().contains("at least 1"));

        let error = serde_json::from_str::<OnFailure>(r#"{"retry":1,"backoff":true}"#).unwrap_err();
        assert!(error.to_string().contains("only field"));

        let error = serde_json::from_str::<RawChildBody>(
            r#"{"on_failure":"ignore","on_failure":"ignore"}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate field `on_failure`"));
    }

    #[test]
    fn test_onfailure_子エントリのserializeで往復する() {
        let ignore = ChildEntry {
            on_failure: Some(OnFailure::Ignore),
            ..ChildEntry::reference("flaky")
        };
        assert_eq!(
            serde_json::to_value(&ignore).unwrap(),
            serde_json::json!({"flaky": {"on_failure": "ignore"}})
        );

        let retry = ChildEntry {
            on_failure: Some(OnFailure::Retry(2)),
            ..ChildEntry::reference("flaky")
        };
        assert_eq!(
            serde_json::to_value(&retry).unwrap(),
            serde_json::json!({"flaky": {"on_failure": {"retry": 2}}})
        );

        // 扱いなしは文字列参照へ畳まれる（現行のまま）。
        assert_eq!(
            serde_json::to_value(ChildEntry::reference("plain")).unwrap(),
            serde_json::json!("plain")
        );
    }

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

    #[test]
    fn test_実効辺_rules省略は隣接辺で末尾は終端() {
        let sequence = SequenceSpec {
            entry: None,
            children: vec![
                ChildEntry::reference("first"),
                ChildEntry::reference("second"),
            ],
        };

        assert_eq!(
            sequence.effective_rules("first"),
            EffectiveRules::AdjacentNext("second")
        );
        assert_eq!(sequence.effective_rules("second"), EffectiveRules::Terminal);
        assert_eq!(
            sequence.effective_rules("unlisted"),
            EffectiveRules::Terminal
        );
    }

    #[test]
    fn test_実効辺_明示rulesが隣接辺より優先され空rulesは終端() {
        let sequence = SequenceSpec {
            entry: None,
            children: vec![
                ChildEntry {
                    on_failure: None,
                    name: "first".to_string(),
                    inputs: Vec::new(),
                    rules: Some(vec![Rule::Next("third".to_string())]),
                },
                ChildEntry {
                    on_failure: None,
                    name: "second".to_string(),
                    inputs: Vec::new(),
                    rules: Some(Vec::new()),
                },
                ChildEntry::reference("third"),
            ],
        };

        assert_eq!(
            sequence.effective_rules("first"),
            EffectiveRules::Rules(&[Rule::Next("third".to_string())][..])
        );
        assert_eq!(
            sequence.effective_rules("second"),
            EffectiveRules::Rules(&[][..])
        );
    }

    #[test]
    fn test_entry解決_sequenceのentry省略時はリスト先頭() {
        let sequence = SequenceSpec {
            entry: None,
            children: vec![
                ChildEntry::reference("first"),
                ChildEntry::reference("second"),
            ],
        };
        assert_eq!(sequence.entry_child_name(), Some("first"));

        let explicit = SequenceSpec {
            entry: Some("second".to_string()),
            ..sequence
        };
        assert_eq!(explicit.entry_child_name(), Some("second"));
    }

    #[test]
    fn test_node名前空間_明示名と自動生成名を同じ空間で一意にする() {
        let mut namespace = NodeNamespace::default();

        assert_eq!(namespace.register_explicit("prepare").unwrap(), "prepare");
        assert_eq!(namespace.register_synthesized("main", 0).unwrap(), "main#0");
        assert!(namespace.contains("prepare"));
        assert!(namespace.contains("main#0"));
        assert_eq!(
            namespace.register("main#0"),
            Err(NodeNamespaceError::Duplicate("main#0".to_string()))
        );
    }

    #[test]
    fn test_node名前空間_明示名では予約語を拒否する() {
        let mut namespace = NodeNamespace::default();

        assert_eq!(
            namespace.register_explicit("children"),
            Err(NodeNamespaceError::Reserved("children".to_string()))
        );
        assert!(!namespace.contains("children"));
        assert_eq!(
            namespace.register_explicit("artifact"),
            Err(NodeNamespaceError::Reserved("artifact".to_string()))
        );
        assert!(!namespace.contains("artifact"));
        assert!(namespace.register_explicit("output").is_ok());
        assert!(namespace.contains("output"));
    }
}
