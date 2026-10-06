use releash_lib::test_support::integration::workflow::delete_facet;
use releash_lib::test_support::integration::workflow::list_facet_summaries;
use releash_lib::test_support::integration::workflow::list_facets;
use releash_lib::test_support::integration::workflow::load_facet;
use releash_lib::test_support::integration::workflow::resolve_facet_path;
use releash_lib::test_support::integration::workflow::save_facet;
use releash_lib::test_support::integration::workflow::FacetError;
use releash_lib::test_support::integration::workflow::FacetKind;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup_facet_files(dir: &Path) {
    let policies = dir.join("policies");
    let knowledge = dir.join("knowledge");
    let instructions = dir.join("instructions");
    for d in [&policies, &knowledge, &instructions] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(policies.join("coding.md"), "Follow best practices.").unwrap();
    fs::write(policies.join("review.md"), "Review carefully.").unwrap();
    fs::write(knowledge.join("architecture.md"), "The system uses Tauri.").unwrap();
    fs::write(instructions.join("implement.md"), "Implement the feature.").unwrap();
}

// --- validate_facet_key ---

#[test]
pub fn test_facet読取_読めない上書きをbuiltinへ切り替えず一覧と診断にも失敗を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let key = releash_lib::test_support::integration::workflow::list_builtin_facet_keys(
        FacetKind::Instruction,
    )[0];
    let path = directory
        .path()
        .join(FacetKind::Instruction.dir_name())
        .join(format!("{key}.md"));
    fs::create_dir_all(&path).unwrap();
    // When
    let content = load_facet(FacetKind::Instruction, key, directory.path());
    let summaries = list_facet_summaries(FacetKind::Instruction, directory.path());
    let diagnosis = releash_lib::test_support::integration::workflow::diagnose_all(
        directory.path(),
        directory.path(),
    );
    // Then
    assert!(content.is_err());
    assert!(summaries.is_err());
    assert!(diagnosis.is_err());
}

// --- load_facet ---

#[test]
pub fn load_existing_facet() {
    let tmp = TempDir::new().unwrap();
    setup_facet_files(tmp.path());
    let content = load_facet(FacetKind::Policy, "coding", tmp.path()).unwrap();
    assert_eq!(content, "Follow best practices.");
}

#[test]
pub fn load_missing_facet_returns_not_found() {
    let tmp = TempDir::new().unwrap();
    let result = load_facet(FacetKind::Policy, "unknown", tmp.path());
    assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
}

#[test]
pub fn load_facet_with_invalid_key() {
    let tmp = TempDir::new().unwrap();
    let result = load_facet(FacetKind::Policy, "../evil", tmp.path());
    assert!(matches!(result.unwrap_err(), FacetError::InvalidKey { .. }));
}

// --- save_facet ---

#[test]
pub fn save_new_facet() {
    let tmp = TempDir::new().unwrap();
    save_facet(FacetKind::Policy, "new-one", "content", tmp.path()).unwrap();
    let path = tmp.path().join("policies/new-one.md");
    assert!(path.exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), "content");
}

#[test]
pub fn save_overwrites_existing() {
    let tmp = TempDir::new().unwrap();
    save_facet(FacetKind::Knowledge, "test", "v1", tmp.path()).unwrap();
    save_facet(FacetKind::Knowledge, "test", "v2", tmp.path()).unwrap();
    let content = load_facet(FacetKind::Knowledge, "test", tmp.path()).unwrap();
    assert_eq!(content, "v2");
}

#[test]
pub fn save_with_invalid_key() {
    let tmp = TempDir::new().unwrap();
    let result = save_facet(FacetKind::Knowledge, "", "content", tmp.path());
    assert!(matches!(result.unwrap_err(), FacetError::InvalidKey { .. }));
}

// --- delete_facet ---

#[test]
pub fn delete_existing_facet() {
    let tmp = TempDir::new().unwrap();
    save_facet(FacetKind::Knowledge, "deleteme", "content", tmp.path()).unwrap();
    delete_facet(FacetKind::Knowledge, "deleteme", tmp.path()).unwrap();
    assert!(!tmp.path().join("knowledge/deleteme.md").exists());
}

#[test]
pub fn delete_missing_facet_returns_not_found() {
    let tmp = TempDir::new().unwrap();
    let result = delete_facet(FacetKind::Knowledge, "nope", tmp.path());
    assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
}

// --- list_facets ---

#[test]
pub fn list_facets_sorted() {
    let tmp = TempDir::new().unwrap();
    setup_facet_files(tmp.path());
    let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
    let mut expected = releash_lib::test_support::integration::workflow::list_builtin_facet_keys(
        FacetKind::Knowledge,
    )
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    expected.push("architecture".to_string());
    expected.sort();
    expected.dedup();
    assert_eq!(keys, expected);
}

#[test]
pub fn list_facets_empty_dir() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("knowledge")).unwrap();
    let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
    // custom dir は空でも builtin Knowledge facets は含まれる
    let mut expected = releash_lib::test_support::integration::workflow::list_builtin_facet_keys(
        FacetKind::Knowledge,
    )
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(keys, expected);
}

#[test]
pub fn list_facets_nonexistent_dir() {
    let tmp = TempDir::new().unwrap();
    let keys = list_facets(FacetKind::Knowledge, tmp.path()).unwrap();
    // custom dir が存在しなくても builtin Knowledge facets は含まれる
    let mut expected = releash_lib::test_support::integration::workflow::list_builtin_facet_keys(
        FacetKind::Knowledge,
    )
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(keys, expected);
}

// --- extract_description ---

// --- list_facet_summaries ---

#[test]
pub fn list_facet_summaries_merges_builtin_and_custom() {
    let tmp = TempDir::new().unwrap();
    let policies = tmp.path().join("policies");
    fs::create_dir_all(&policies).unwrap();
    fs::write(
        policies.join("custom-policy.md"),
        "# Custom Policy\nContent",
    )
    .unwrap();

    let summaries = list_facet_summaries(FacetKind::Policy, tmp.path()).unwrap();
    let builtin_count = releash_lib::test_support::integration::workflow::list_builtin_facet_keys(
        FacetKind::Policy,
    )
    .len();
    assert_eq!(summaries.len(), builtin_count + 1);

    let custom = summaries.iter().find(|s| s.key == "custom-policy").unwrap();
    assert!(!custom.builtin);
    assert_eq!(custom.description, "Custom Policy");

    let coding = summaries.iter().find(|s| s.key == "coding").unwrap();
    assert!(coding.builtin);
}

// --- resolve_facet_path ---

#[test]
pub fn resolve_facet_path_existing() {
    let tmp = TempDir::new().unwrap();
    save_facet(FacetKind::Policy, "test", "content", tmp.path()).unwrap();
    let path = resolve_facet_path(FacetKind::Policy, "test", tmp.path()).unwrap();
    assert!(path.exists());
}

#[test]
pub fn resolve_facet_path_missing() {
    let tmp = TempDir::new().unwrap();
    let result = resolve_facet_path(FacetKind::Policy, "nope", tmp.path());
    assert!(matches!(result.unwrap_err(), FacetError::NotFound { .. }));
}

// --- 重複チェック用ヘルパーテスト ---

#[test]
pub fn list_facets_detects_existing_custom_key() {
    let tmp = TempDir::new().unwrap();
    save_facet(FacetKind::Policy, "my-policy", "content", tmp.path()).unwrap();
    let existing = list_facets(FacetKind::Policy, tmp.path()).unwrap();
    assert!(existing.contains(&"my-policy".to_string()));
}

#[test]
pub fn list_facets_includes_builtin_keys() {
    let tmp = TempDir::new().unwrap();
    let existing = list_facets(FacetKind::Policy, tmp.path()).unwrap();
    // ビルトインのポリシーキーが含まれる
    assert!(!existing.is_empty());
}

#[test]
pub fn delete_builtin_facet_is_protected() {
    let tmp = TempDir::new().unwrap();
    // ビルトインキーの削除はBuiltinProtectedエラー
    let builtin_keys = releash_lib::test_support::integration::workflow::list_builtin_facet_keys(
        FacetKind::Policy,
    );
    if let Some(key) = builtin_keys.first() {
        let result = delete_facet(FacetKind::Policy, key, tmp.path());
        assert!(matches!(
            result.unwrap_err(),
            FacetError::BuiltinProtected { .. }
        ));
    }
}

// --- Artifact template rendering ---
