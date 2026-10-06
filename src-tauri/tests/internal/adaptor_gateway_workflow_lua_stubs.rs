use releash_lib::test_support::integration::workflow::generate_editor_support;

#[test]
pub fn test_worktreeのstub_生成物が型とhandleと全builderの公開契約を持つ() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();
    // Then
    assert!(stub.lines().any(|line| line == "---@class ReleashWorktree"));
    let class = |name: &str| {
        stub.split(&format!("---@class {name}\n"))
            .nth(1)
            .unwrap()
            .split("---@class ")
            .next()
            .unwrap()
    };
    for handle in ["shared", "isolated"] {
        assert!(class("ReleashWorktreeModule")
            .contains(&format!("---@field {handle} ReleashWorktree\n")));
    }
    assert!(class("ReleashModule").contains("---@field worktree ReleashWorktreeModule\n"));
    for builder in ["Command", "Session", "Fanout", "Sequence"] {
        assert!(
            class(&format!("Releash{builder}Options"))
                .contains("---@field worktree? ReleashWorktree\n"),
            "{builder}"
        );
        let result = if builder == "Session" {
            "ReleashSession"
        } else {
            "ReleashNode"
        };
        assert!(
            class("ReleashModule").contains(&format!(
                "---@field {} fun(options: Releash{builder}Options): {result}\n",
                builder.to_lowercase()
            )),
            "{builder}"
        );
    }
}

#[test]
pub fn test_lua述語stub_生成物の型とbuilderが公開apiと一致する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();
    // Then
    assert!(stub.contains("---@class ReleashPredicate"));
    assert!(
        stub.contains("---@class ReleashWhenOptions\n---@field on ReleashSource|ReleashPredicate")
    );
    for builder in ["all", "any"] {
        assert!(stub.contains(&format!("---@field {builder} fun(elements: (ReleashSource|ReleashPredicate)[]): ReleashPredicate")));
    }
    assert!(stub.contains("---@class ReleashSwitchOptions\n---@field on ReleashSource\n"));
}

#[test]
pub fn test_completionのstub_要求tableと承認handleの型を区別する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();
    // Then
    assert!(stub
        .contains("---@class ReleashCompletion\n---@field require ReleashCompletionRequirement\n"));
    assert!(stub.contains("---@field approval ReleashCompletionRequirement\n"));
    assert_eq!(
        stub.matches("---@field completion? ReleashCompletion\n")
            .count(),
        4
    );
}

#[test]
pub fn test_completionのstub_delegateメソッドとchild段を再生成する() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(".releash/releash.lua");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "old stub").unwrap();
    // When
    generate_editor_support(directory.path()).unwrap();
    let stub = std::fs::read_to_string(path).unwrap();
    // Then
    assert!(stub
        .contains("---@field delegate fun(options: ReleashDelegateOptions) Session handles only;"));
    assert!(stub.contains("---@field child ReleashSource Only on a Session with delegate."));
    assert!(stub.contains("---@class ReleashDelegateOptions\n---@field child ReleashNode\n"));
    assert!(stub.contains("---@field inputs? table<string, ReleashSource> Parent Input"));
    assert!(stub.contains("---@field when ReleashSource|ReleashPredicate Parent Artifact"));
    assert!(stub.contains("---@field max_iterations integer At least 1."));
    let class = |name: &str| {
        stub.split(&format!("---@class {name}\n"))
            .nth(1)
            .unwrap()
            .split("---@class ")
            .next()
            .unwrap()
    };
    let node = class("ReleashNode: ReleashSource");
    assert!(!node.contains("---@field delegate "));
    assert!(!node.contains("---@field child "));
    let session = class("ReleashSession: ReleashNode");
    assert!(session.contains("---@field delegate fun(options: ReleashDelegateOptions)"));
    assert!(session.contains("---@field child ReleashSource"));
    assert!(class("ReleashModule")
        .contains("---@field session fun(options: ReleashSessionOptions): ReleashSession\n"));
    for builder in ["Command", "Sequence", "Fanout"] {
        assert!(class("ReleashModule").contains(&format!(
            "---@field {} fun(options: Releash{builder}Options): ReleashNode\n",
            builder.to_lowercase()
        )));
    }
}
pub(crate) mod tests {

    use releash_lib::test_support::integration::workflow::facet_document_url;
    use releash_lib::test_support::integration::workflow::generate_editor_support;
    use releash_lib::test_support::integration::workflow::FacetKind;
    use releash_lib::test_support::integration::workflow::LUARC;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    pub fn generates_idempotent_stubs_and_preserves_existing_luarc() {
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
            releash_lib::test_support::integration::workflow::get_builtin_facet(
                FacetKind::Policy,
                "coding"
            )
            .unwrap()
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
        assert!(releash.contains("---@field env? table<string, ReleashSource>"));
        assert!(!releash.contains("---@field equals ReleashNode"));
    }

    #[test]
    pub fn test_編集支援生成_permission補完を4値unionに限定する() {
        // Given
        let directory = TempDir::new().unwrap();

        // When
        generate_editor_support(directory.path()).unwrap();
        let releash = fs::read_to_string(directory.path().join(".releash/releash.lua")).unwrap();

        // Then
        let permission_aliases = releash
            .lines()
            .filter(|line| line.starts_with("---@alias ReleashPermission "))
            .collect::<Vec<_>>();
        assert_eq!(
            permission_aliases,
            vec!["---@alias ReleashPermission \"manual\" | \"auto\" | \"bypass\" | \"read-only\""]
        );

        let permission_fields = releash
            .lines()
            .filter(|line| line.starts_with("---@field permission? "))
            .collect::<Vec<_>>();
        assert_eq!(
            permission_fields,
            vec!["---@field permission? ReleashPermission"]
        );
    }

    #[test]
    pub fn facet_links_percent_encode_paths_with_spaces() {
        let directory = TempDir::new().unwrap();
        let base = directory.path().join("Application Support");
        let instructions = base.join("instructions");
        fs::create_dir_all(&instructions).unwrap();
        fs::write(instructions.join("custom.md"), "# Custom facet\nBody").unwrap();

        generate_editor_support(&base).unwrap();

        let facets = fs::read_to_string(base.join(".releash/facets.lua")).unwrap();
        assert!(facets.contains("Application%20Support"));
        assert!(!facets.contains("Application Support"));
    }

    #[test]
    pub fn facet_document_url_absolutizes_relative_paths() {
        let url = facet_document_url(Path::new("releash/workflows/policies/coding.md")).unwrap();

        assert!(url.starts_with("file:///"), "{url}");
        assert!(
            url.ends_with("/releash/workflows/policies/coding.md"),
            "{url}"
        );
    }

    #[test]
    pub fn custom_facet_with_builtin_key_links_to_custom_document() {
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
    pub fn generated_builtin_document_is_not_a_runtime_facet_source() {
        let directory = TempDir::new().unwrap();
        generate_editor_support(directory.path()).unwrap();
        let generated_builtin = directory.path().join(".releash/facets/policies/coding.md");
        let expected = releash_lib::test_support::integration::workflow::get_builtin_facet(
            FacetKind::Policy,
            "coding",
        )
        .unwrap()
        .to_string();

        fs::write(&generated_builtin, "stale generated content").unwrap();
        assert_eq!(
            releash_lib::test_support::integration::workflow::load_facet(
                FacetKind::Policy,
                "coding",
                directory.path()
            )
            .unwrap(),
            expected
        );

        fs::remove_file(generated_builtin).unwrap();
        assert_eq!(
            releash_lib::test_support::integration::workflow::load_facet(
                FacetKind::Policy,
                "coding",
                directory.path()
            )
            .unwrap(),
            expected
        );
    }

    #[test]
    pub fn generates_luarc_only_when_absent() {
        let directory = TempDir::new().unwrap();

        generate_editor_support(directory.path()).unwrap();

        assert_eq!(
            fs::read_to_string(directory.path().join(".luarc.json")).unwrap(),
            LUARC
        );
    }
}
