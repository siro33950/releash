use std::fs;
use std::path::{Path, PathBuf};

use crate::adaptor::gateway::workflow::{
    builtin,
    facet::{self, FacetKind},
};

const RELEASH_STUB: &str = r#"---@meta

---@class ReleashSource
---@class ReleashNode: ReleashSource
---@class ReleashChild
---@class ReleashRule
---@class ReleashOnFailure
---@class ReleashInput: ReleashSource
---@class ReleashSchema
---@class ReleashFacet
---@class ReleashProvider
---@class ReleashCompletion
---@class ReleashWorkflow

---@class ReleashCommandOptions
---@field name? string
---@field command string
---@field artifact? ReleashSchema
---@field input? ReleashInput[]
---@field completion? ReleashCompletion

---@class ReleashSessionFacets
---@field policy? ReleashFacet
---@field knowledge? ReleashFacet[]
---@field instruction? ReleashFacet

---@class ReleashSessionOptions
---@field name? string
---@field provider ReleashProvider
---@field model? string
---@field permission? string
---@field facets? ReleashSessionFacets
---@field artifact? ReleashSchema
---@field input? ReleashInput[]
---@field completion? ReleashCompletion

---@class ReleashChildOptions
---@field node ReleashNode
---@field inputs? table<string, ReleashSource>
---@field rules? ReleashRule[]
---@field on_failure? ReleashOnFailure

---@class ReleashFanoutOptions
---@field name? string
---@field children ReleashChild[]
---@field items? ReleashSource|table
---@field artifact? ReleashSchema
---@field input? ReleashInput[]
---@field completion? ReleashCompletion

---@class ReleashSequenceOptions
---@field name? string
---@field entry? ReleashNode
---@field output? ReleashNode
---@field children ReleashChild[]
---@field artifact? ReleashSchema
---@field input? ReleashInput[]
---@field completion? ReleashCompletion

---@class ReleashWhenOptions
---@field on ReleashSource
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
---@field approval ReleashCompletion

---@class ReleashModule
---@field command fun(options: ReleashCommandOptions): ReleashNode
---@field session fun(options: ReleashSessionOptions): ReleashNode
---@field fanout fun(options: ReleashFanoutOptions): ReleashNode
---@field sequence fun(options: ReleashSequenceOptions): ReleashNode
---@field child fun(options: ReleashChildOptions): ReleashChild
---@field next fun(node: ReleashNode): ReleashRule
---@field when fun(options: ReleashWhenOptions): ReleashRule
---@field switch fun(options: ReleashSwitchOptions): ReleashRule
---@field loop_guard fun(options: ReleashLoopGuardOptions): ReleashRule
---@field retry fun(count: integer): ReleashOnFailure
---@field ignore ReleashOnFailure
---@field input fun(name: string, contract?: ReleashSchema): ReleashInput
---@field request ReleashSource
---@field items ReleashSource
---@field completion ReleashCompletionModule
---@field provider ReleashProviderModule
---@field schema ReleashSchemaModule
---@field workflow fun(options: ReleashWorkflowOptions): ReleashWorkflow

---@type ReleashModule
local releash = {}
return releash
"#;

const LUARC: &str = r#"{
  "runtime.version": "Lua 5.4",
  "workspace.library": [
    ".releash"
  ]
}
"#;

pub(crate) fn generate_editor_support(workflows_dir: &Path) -> std::io::Result<()> {
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

fn generate_facet_stub(base_dir: &Path) -> std::io::Result<String> {
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
        let summaries = facet::list_facet_summaries(kind, base_dir)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        for summary in summaries {
            let path = facet_document_path(base_dir, kind, &summary.key, summary.builtin);
            let description = summary.description.replace(['\r', '\n'], " ");
            let key = lua_doc_field(&summary.key);
            output.push_str(&format!(
                "---@field {key} ReleashFacet {description} ([本文](file://{}))\n",
                path.display()
            ));
        }
        output.push('\n');
    }
    output.push_str(
        "---@class ReleashFacetModule\n---@field instruction ReleashInstructionFacets\n---@field policy ReleashPolicyFacets\n---@field knowledge ReleashKnowledgeFacets\n\n---@type ReleashFacetModule\nlocal facets = {}\nreturn facets\n",
    );
    Ok(output)
}

fn generate_builtin_facet_documents(base_dir: &Path) -> std::io::Result<()> {
    for kind in [
        FacetKind::Instruction,
        FacetKind::Policy,
        FacetKind::Knowledge,
    ] {
        let kind_dir = base_dir.join(".releash/facets").join(kind.dir_name());
        fs::create_dir_all(&kind_dir)?;
        for key in builtin::list_builtin_facet_keys(kind) {
            let content = builtin::get_builtin_facet(kind, key).ok_or_else(|| {
                std::io::Error::other(format!(
                    "builtin facet content is missing: {}/{key}",
                    kind.dir_name()
                ))
            })?;
            write_if_changed(&kind_dir.join(format!("{key}.md")), content)?;
        }
    }
    Ok(())
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generates_idempotent_stubs_and_preserves_existing_luarc() {
        let directory = TempDir::new().unwrap();
        let instructions = directory.path().join("instructions");
        fs::create_dir_all(&instructions).unwrap();
        fs::write(instructions.join("custom.md"), "# Custom facet\nBody").unwrap();
        fs::write(directory.path().join(".luarc.json"), "{\"custom\":true}").unwrap();

        generate_editor_support(directory.path()).unwrap();
        let first = fs::read_to_string(directory.path().join(".releash/facets.lua")).unwrap();
        let builtin_path = directory.path().join(".releash/facets/policies/coding.md");
        let first_builtin = fs::read_to_string(&builtin_path).unwrap();
        generate_editor_support(directory.path()).unwrap();

        assert_eq!(
            fs::read_to_string(directory.path().join(".luarc.json")).unwrap(),
            "{\"custom\":true}"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join(".releash/facets.lua")).unwrap(),
            first
        );
        assert_eq!(fs::read_to_string(&builtin_path).unwrap(), first_builtin);
        assert_eq!(
            first_builtin,
            builtin::get_builtin_facet(FacetKind::Policy, "coding").unwrap()
        );
        assert!(first.contains("custom ReleashFacet Custom facet"));
        assert!(first.contains(&format!(
            "file://{}",
            instructions.join("custom.md").display()
        )));
        assert!(first.contains(&format!("file://{}", builtin_path.display())));
        let releash = fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();
        assert!(releash.contains("---@class ReleashNode: ReleashSource"));
        assert!(releash.contains("---@field sequence fun(options: ReleashSequenceOptions)"));
        assert!(releash.contains("---@field workflow fun(options: ReleashWorkflowOptions)"));
        assert!(releash.contains("---@field on_true ReleashNode"));
        assert!(!releash.contains("---@field equals ReleashNode"));
    }

    #[test]
    fn custom_facet_with_builtin_key_links_to_custom_document() {
        let directory = TempDir::new().unwrap();
        let policies = directory.path().join("policies");
        fs::create_dir_all(&policies).unwrap();
        let custom_path = policies.join("coding.md");
        fs::write(&custom_path, "# Custom coding\nBody").unwrap();

        generate_editor_support(directory.path()).unwrap();

        let facets = fs::read_to_string(directory.path().join(".releash/facets.lua")).unwrap();
        let generated_builtin = directory.path().join(".releash/facets/policies/coding.md");
        assert!(facets.contains("coding ReleashFacet Custom coding"));
        assert!(facets.contains(&format!("file://{}", custom_path.display())));
        assert!(!facets.contains(&format!("file://{}", generated_builtin.display())));
    }

    #[test]
    fn generated_builtin_document_is_not_a_runtime_facet_source() {
        let directory = TempDir::new().unwrap();
        generate_editor_support(directory.path()).unwrap();
        let generated_builtin = directory.path().join(".releash/facets/policies/coding.md");
        let expected = builtin::get_builtin_facet(FacetKind::Policy, "coding")
            .unwrap()
            .to_string();

        fs::write(&generated_builtin, "stale generated content").unwrap();
        assert_eq!(
            facet::load_facet(FacetKind::Policy, "coding", directory.path()).unwrap(),
            expected
        );

        fs::remove_file(generated_builtin).unwrap();
        assert_eq!(
            facet::load_facet(FacetKind::Policy, "coding", directory.path()).unwrap(),
            expected
        );
    }

    #[test]
    fn generates_luarc_only_when_absent() {
        let directory = TempDir::new().unwrap();

        generate_editor_support(directory.path()).unwrap();

        assert_eq!(
            fs::read_to_string(directory.path().join(".luarc.json")).unwrap(),
            LUARC
        );
    }
}
