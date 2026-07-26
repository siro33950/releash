use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn rust_sources_below(path: &Path, sources: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("failed to read an entry below {}: {error}", path.display())
        });
        let entry_path = entry.path();
        if entry_path.is_dir() {
            rust_sources_below(&entry_path, sources);
        } else if entry_path.extension().and_then(|value| value.to_str()) == Some("rs") {
            sources.push(entry_path);
        }
    }
}

fn enum_declaration_count(source: &str, enum_name: &str) -> usize {
    source
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .map(|line| {
            line.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .filter(|token| !token.is_empty())
                .collect::<Vec<_>>()
        })
        .map(|tokens| {
            tokens
                .windows(2)
                .filter(|tokens| tokens[0] == "enum" && tokens[1] == enum_name)
                .count()
        })
        .sum()
}

#[test]
fn issue_1499_d1_contract_is_not_redefined() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must have a repository parent");
    let d1_design = fs::read_to_string(repository_root.join("docs/specs/issues-1445/design.md"))
        .expect("the D1 design contract must be readable");
    let requirements =
        fs::read_to_string(repository_root.join("docs/specs/issues-1499/requirements.md"))
            .expect("the #1499 requirements must be readable");
    let behavior = fs::read_to_string(repository_root.join("docs/specs/issues-1499/behavior.md"))
        .expect("the #1499 behavior contract must be readable");

    assert!(
        d1_design.contains("本 Issue は **design gate**")
            && d1_design.contains("runtime code / 永続化 / UI")
            && d1_design.contains("実装は含まず"),
        "D1 must remain a design-only contract"
    );
    assert!(
        requirements.contains("R-019")
            && requirements.contains("D1 #1445で確定したdesign-only")
            && requirements.contains("境界を再定義しない"),
        "#1499 must retain its explicit D1 non-redefinition requirement"
    );
    assert!(
        behavior.contains("issue_1499_d1_contract_is_not_redefined")
            && behavior.contains("frontend action enablementを再定義しない"),
        "B-077 must retain the exact D1 trace-matrix gate"
    );

    let issue_1499_owned_roots = [
        repository_root.join("src-tauri/src/domain/local_event"),
        repository_root.join("src-tauri/src/adaptor/gateway/local_event_store"),
        repository_root.join("src-tauri/src/usecase/agent_session/operation"),
    ];
    let mut sources = Vec::new();
    for root in &issue_1499_owned_roots {
        rust_sources_below(root, &mut sources);
    }
    sources.push(repository_root.join("src-tauri/src/usecase/shutdown_coordinator.rs"));

    let d1_declarations = [
        "enum AgentMode",
        "struct AgentSessionConfiguration",
        "enum AgentSessionConfigurationState",
        "struct AgentGoalState",
        "enum GoalCapabilitySupport",
        "enum ModeCapabilitySupport",
        "struct ReasoningEffortCapability",
        "struct AgentLaunch",
        "enum AgentLaunch",
    ];
    for source_path in sources {
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", source_path.display()));
        for declaration in d1_declarations {
            assert!(
                !source.lines().any(|line| line.contains(declaration)),
                "#1499-owned module {} redefines the D1 declaration {declaration}",
                source_path.display()
            );
        }
    }
}

#[test]
fn b104_canonical_message_part_has_one_production_definition() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must have a repository parent");
    let production_root = repository_root.join("src-tauri/src");
    let canonical = production_root.join("domain/agent_session/entities/message_part.rs");
    let mut sources = Vec::new();
    rust_sources_below(&production_root, &mut sources);

    let mut declarations = Vec::new();
    for source_path in sources {
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", source_path.display()));
        for _ in 0..enum_declaration_count(&source, "MessagePart") {
            declarations.push(source_path.clone());
        }
    }

    // Versioned persistence/public DTOs and test snapshots deliberately use
    // distinct names; only the semantic domain enum is unique by exact name.
    assert_eq!(
        declarations,
        vec![canonical],
        "production code must define the MessagePart semantic enum only in the canonical domain module"
    );
}

#[test]
fn f06_repository_port_contains_no_opaque_persistence_json() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must have a repository parent");
    let checks = [
        (
            "src-tauri/src/domain/local_event/record.rs",
            vec![
                (
                    "raw: String",
                    "domain repository records must be closed semantic values, not raw JSON",
                ),
                (
                    "pub fn as_str(&self)",
                    "domain repository records must not expose persistence text",
                ),
            ],
        ),
        (
            "src-tauri/src/usecase/shutdown_coordinator.rs",
            vec![(
                "shutdown_target_recovery_result_v1",
                "the usecase must not own a versioned persistence schema",
            )],
        ),
    ];
    let mut violations = Vec::new();

    for (relative_path, forbidden) in checks {
        let source_path = repository_root.join(relative_path);
        let source = fs::read_to_string(&source_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", source_path.display()));
        for (needle, contract) in forbidden {
            if source.contains(needle) {
                violations.push(format!("{relative_path}: {contract} (`{needle}`)"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "opaque/versioned persistence JSON crossed the repository boundary:\n{}",
        violations.join("\n")
    );
}

#[test]
fn close_quit_decision_table_has_one_complete_row_per_typed_surface() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri must have a repository parent");
    let decision_table = fs::read_to_string(
        repository_root
            .join("specs/milestone-84-agent-chat-stabilization/close-quit-decision-table.md"),
    )
    .expect("the close/quit decision table must be readable");

    let table = decision_table
        .split("## Surface decision table")
        .nth(1)
        .and_then(|tail| tail.split("## Application shutdown contract").next())
        .expect("the surface decision table must have stable section boundaries");
    let mut rows = table.lines().filter(|line| {
        line.starts_with('|')
            && !line.starts_with("| ---")
            && !line.starts_with("| Surface / action")
    });

    let expected_surfaces = [
        "chat tab close",
        "chat panel close",
        "workflow node tab close",
        "workspace close",
        "window close",
        "active Session close",
        "Idle Session close",
        "active open Session archive",
        "Idle open Session archive",
        "closed Session archive",
        "backend switch",
        "Cmd-Q / menu / Dock / Tray Quit",
        "cooperative OS logout / shutdown",
        "programmatic exit / restart",
        "concurrent quit",
        "cooperative quit during SQLite startup failure",
        "hard kill / power loss",
    ];
    let expected = expected_surfaces.into_iter().collect::<HashSet<_>>();
    let mut observed = HashSet::new();

    for row in rows.by_ref() {
        let cells = row
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        assert_eq!(
            cells.len(),
            6,
            "every surface row must define all six semantic columns: {row}"
        );
        assert!(
            cells.iter().all(|cell| !cell.is_empty()),
            "decision-table cells cannot inherit an omitted contract: {row}"
        );
        let surface = cells[0];
        assert!(
            observed.insert(surface.to_string()),
            "typed surface appears more than once: {surface}"
        );
        assert!(
            expected.contains(surface),
            "unexpected typed surface in decision table: {surface}"
        );
    }

    assert_eq!(
        observed,
        expected.into_iter().map(str::to_string).collect(),
        "the decision table must contain every typed surface exactly once"
    );
}
