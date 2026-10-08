use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::adaptor::gateway::workflow::{
    builtin,
    facet::{self, FacetError, FacetKind},
};

/// 編集支援ファイルの生成で発生しうるエラー。I/O 失敗と facet カタログの失敗を
/// 型で分ける。
#[derive(Debug)]
pub enum StubGenerationError {
    Io(std::io::Error),
    Facet(FacetError),
    MissingBuiltinFacet { kind: FacetKind, key: String },
    InvalidDocumentPath { path: PathBuf },
}

impl fmt::Display for StubGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/Oエラー: {error}"),
            Self::Facet(error) => write!(formatter, "facet一覧の取得に失敗: {error}"),
            Self::MissingBuiltinFacet { kind, key } => write!(
                formatter,
                "ビルトインファセット '{key}' ({}) の本文が見つかりません",
                kind.dir_name()
            ),
            Self::InvalidDocumentPath { path } => write!(
                formatter,
                "ファセット本文のパスを file URL へ変換できません: {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StubGenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Facet(error) => Some(error),
            Self::MissingBuiltinFacet { .. } | Self::InvalidDocumentPath { .. } => None,
        }
    }
}

impl From<std::io::Error> for StubGenerationError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<FacetError> for StubGenerationError {
    fn from(error: FacetError) -> Self {
        Self::Facet(error)
    }
}

const RELEASH_STUB: &str = r#"---@meta

---@class ReleashPredicate
---@class ReleashSource
---@class ReleashNode: ReleashSource
---@class ReleashSession: ReleashNode
---@field delegate fun(options: ReleashDelegateOptions) Session handles only; requires an Artifact Contract; declare once with node.delegate{...}.
---@field child ReleashSource Only on a Session with delegate. Session/Command fields are direct; Sequence/Fanout fields traverse the merged map.
---@class ReleashChild
---@class ReleashRule
---@class ReleashInput: ReleashSource
---@class ReleashSchema
---@class ReleashFacet
---@class ReleashProvider
---@class ReleashWorktree
---@class ReleashWorktreeModule
---@field shared ReleashWorktree
---@field isolated ReleashWorktree
---@class ReleashCompletionRequirement
---@class ReleashCompletion
---@field require ReleashCompletionRequirement
---@class ReleashWorkflow

---@alias ReleashPermission "manual" | "auto" | "bypass" | "read-only"

---@class ReleashCommandOptions
---@field name? string
---@field command string
---@field env? table<string, ReleashSource>
---@field artifact? ReleashSchema
---@field input? ReleashInput[]
---@field completion? ReleashCompletion
---@field worktree? ReleashWorktree

---@class ReleashSessionFacets
---@field policy? ReleashFacet
---@field knowledge? ReleashFacet[]
---@field instruction? ReleashFacet

---@class ReleashSessionOptions
---@field name? string
---@field provider ReleashProvider
---@field model? string
---@field permission? ReleashPermission
---@field facets? ReleashSessionFacets
---@field artifact? ReleashSchema
---@field input? ReleashInput[]
---@field completion? ReleashCompletion
---@field worktree? ReleashWorktree

---@class ReleashDelegateOptions
---@field child ReleashNode
---@field inputs? table<string, ReleashSource> Parent Input, parent Artifact (including the Node itself and child fields), or releash.request.
---@field when ReleashSource|ReleashPredicate Parent Artifact or child fields; required boolean leaves.
---@field max_iterations integer At least 1.

---@class ReleashChildOptions
---@field node ReleashNode
---@field inputs? table<string, ReleashSource>
---@field rules? ReleashRule[]

---@class ReleashFanoutOptions
---@field name? string
---@field children ReleashChild[]
---@field items? ReleashSource|table
---@field artifact? ReleashSchema
---@field input? ReleashInput[]
---@field completion? ReleashCompletion
---@field worktree? ReleashWorktree

---@class ReleashSequenceOptions
---@field name? string
---@field entry? ReleashNode
---@field children ReleashChild[]
---@field input? ReleashInput[]
---@field completion? ReleashCompletion
---@field worktree? ReleashWorktree

---@class ReleashWhenOptions
---@field on ReleashSource|ReleashPredicate
---@field on_true ReleashNode
---@field next ReleashNode

---@class ReleashSwitchOptions
---@field on ReleashSource
---@field cases table<string|integer|boolean, ReleashNode>
---@field next? ReleashNode

---@class ReleashLoopGuardOptions
---@field max_iterations integer
---@field on_exhausted ReleashNode

---@class ReleashObjectSchemaOptions
---@field name? string
---@field properties table<string, ReleashSchema>
---@field required? string[]

---@class ReleashArraySchemaOptions
---@field name? string
---@field items ReleashSchema

---@class ReleashStringSchemaOptions
---@field enum? string[]

---@class ReleashWorkflowOptions
---@field name string
---@field description string
---@field main ReleashNode

---@class ReleashSchemaModule
---@field object fun(options: ReleashObjectSchemaOptions): ReleashSchema
---@field array fun(options: ReleashArraySchemaOptions): ReleashSchema
---@field string fun(options: ReleashStringSchemaOptions): ReleashSchema
---@field boolean fun(): ReleashSchema
---@field integer fun(): ReleashSchema
---@field number fun(): ReleashSchema

---@class ReleashProviderModule
---@field claude ReleashProvider
---@field codex ReleashProvider

---@class ReleashCompletionModule
---@field approval ReleashCompletionRequirement

---@class ReleashModule
---@field command fun(options: ReleashCommandOptions): ReleashNode
---@field session fun(options: ReleashSessionOptions): ReleashSession
---@field fanout fun(options: ReleashFanoutOptions): ReleashNode
---@field sequence fun(options: ReleashSequenceOptions): ReleashNode
---@field child fun(options: ReleashChildOptions): ReleashChild
---@field next fun(node: ReleashNode): ReleashRule
---@field when fun(options: ReleashWhenOptions): ReleashRule
---@field all fun(elements: (ReleashSource|ReleashPredicate)[]): ReleashPredicate
---@field any fun(elements: (ReleashSource|ReleashPredicate)[]): ReleashPredicate
---@field switch fun(options: ReleashSwitchOptions): ReleashRule
---@field loop_guard fun(options: ReleashLoopGuardOptions): ReleashRule
---@field input fun(name: string, contract?: ReleashSchema): ReleashInput
---@field request ReleashSource
---@field items ReleashSource
---@field completion ReleashCompletionModule
---@field worktree ReleashWorktreeModule
---@field provider ReleashProviderModule
---@field schema ReleashSchemaModule
---@field workflow fun(options: ReleashWorkflowOptions): ReleashWorkflow

---@type ReleashModule
local releash = {}
return releash
"#;

pub const LUARC: &str = r#"{
  "runtime.version": "Lua 5.4",
  "workspace.library": [
    ".releash"
  ]
}
"#;

pub fn generate_editor_support(workflows_dir: &Path) -> Result<(), StubGenerationError> {
    fs::create_dir_all(workflows_dir)?;
    let generated_dir = workflows_dir.join(".releash");
    fs::create_dir_all(&generated_dir)?;
    write_if_changed(&generated_dir.join("releash.lua"), RELEASH_STUB)?;
    generate_builtin_facet_documents(workflows_dir)?;
    let facets = generate_facet_stub(workflows_dir)?;
    write_if_changed(&generated_dir.join("facets.lua"), &facets)?;
    let luarc = workflows_dir.join(".luarc.json");
    if !luarc.exists() {
        fs::write(luarc, LUARC)?;
    }
    Ok(())
}

fn write_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }
    fs::write(path, content)
}

fn generate_facet_stub(base_dir: &Path) -> Result<String, StubGenerationError> {
    let mut output = String::from("---@meta\n\n---@class ReleashFacet\n\n");
    for kind in [
        FacetKind::Instruction,
        FacetKind::Policy,
        FacetKind::Knowledge,
    ] {
        let class_name = match kind {
            FacetKind::Instruction => "ReleashInstructionFacets",
            FacetKind::Policy => "ReleashPolicyFacets",
            FacetKind::Knowledge => "ReleashKnowledgeFacets",
        };
        output.push_str(&format!("---@class {class_name}\n"));
        let summaries = facet::list_facet_summaries(kind, base_dir)?;
        for summary in summaries {
            let path = facet_document_path(base_dir, kind, &summary.key, summary.builtin);
            let description = summary.description.replace(['\r', '\n'], " ");
            let key = lua_doc_field(&summary.key);
            output.push_str(&format!(
                "---@field {key} ReleashFacet {description} ([本文]({}))\n",
                facet_document_url(&path)?
            ));
        }
        output.push('\n');
    }
    output.push_str(
        "---@class ReleashFacetModule\n---@field instruction ReleashInstructionFacets\n---@field policy ReleashPolicyFacets\n---@field knowledge ReleashKnowledgeFacets\n\n---@type ReleashFacetModule\nlocal facets = {}\nreturn facets\n",
    );
    Ok(output)
}

fn generate_builtin_facet_documents(base_dir: &Path) -> Result<(), StubGenerationError> {
    for kind in [
        FacetKind::Instruction,
        FacetKind::Policy,
        FacetKind::Knowledge,
    ] {
        let kind_dir = base_dir.join(".releash/facets").join(kind.dir_name());
        fs::create_dir_all(&kind_dir)?;
        for key in builtin::list_builtin_facet_keys(kind) {
            let content = builtin::get_builtin_facet(kind, key).ok_or_else(|| {
                StubGenerationError::MissingBuiltinFacet {
                    kind,
                    key: key.to_string(),
                }
            })?;
            write_if_changed(&kind_dir.join(format!("{key}.md")), content)?;
        }
    }
    Ok(())
}

/// 生成する `file://` リンク。パス区切りの正規化と percent-encode を URL 側へ任せ、
/// 空白を含むパス（macOS の `Application Support` 等）でも有効な URL にする。
///
/// `Url::from_file_path` は相対パスを受け付けないため、先に current_dir で絶対化する
/// （`dirs::config_dir()` が解決できない環境では `workflows_dir()` が相対パスを返す）。
pub fn facet_document_url(path: &Path) -> Result<String, StubGenerationError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    url::Url::from_file_path(&absolute)
        .map(|url| url.to_string())
        .map_err(|_| StubGenerationError::InvalidDocumentPath { path: absolute })
}

fn facet_document_path(
    base_dir: &Path,
    kind: FacetKind,
    key: &str,
    builtin_facet: bool,
) -> PathBuf {
    let custom = base_dir.join(kind.dir_name()).join(format!("{key}.md"));
    if !builtin_facet || custom.exists() {
        return custom;
    }
    base_dir
        .join(".releash/facets")
        .join(kind.dir_name())
        .join(format!("{key}.md"))
}

fn lua_doc_field(key: &str) -> String {
    let mut chars = key.chars();
    if chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        key.to_string()
    } else {
        format!("[\"{}\"]", key.replace('"', "\\\""))
    }
}

#[cfg(test)]
#[path = "stubs_test.rs"]
mod stubs_tests;
