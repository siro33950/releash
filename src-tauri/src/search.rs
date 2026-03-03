use ignore::WalkBuilder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use tree_sitter_tags::{TagsConfiguration, TagsContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: String,
    pub line_number: usize,
    pub line_content: String,
    pub match_start: usize,
    pub match_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    pub total_matches: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionLocation {
    pub path: String,
    pub line_number: usize,
    pub column: usize,
    pub line_content: String,
    pub kind: String,
}

// === tree-sitter-tags configuration cache ===

/// JS base patterns + TypeScript-specific patterns for the TSX grammar.
/// tree-sitter-typescript's TAGS_QUERY only contains TS-specific nodes
/// (function_signature, interface_declaration, etc.) and lacks JS base
/// patterns like function_declaration, class_declaration, variable_declarator.
const JS_TS_TAGS_QUERY: &str = r#"
(method_definition
  name: (property_identifier) @name) @definition.method

[
  (class
    name: (_) @name)
  (class_declaration
    name: (_) @name)
] @definition.class

[
  (function_expression
    name: (identifier) @name)
  (function_declaration
    name: (identifier) @name)
  (generator_function
    name: (identifier) @name)
  (generator_function_declaration
    name: (identifier) @name)
] @definition.function

(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression)]) @definition.function)

(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: [(arrow_function) (function_expression)]) @definition.function)

(assignment_expression
  left: [
    (identifier) @name
    (member_expression
      property: (property_identifier) @name)
  ]
  right: [(arrow_function) (function_expression)]
) @definition.function

(pair
  key: (property_identifier) @name
  value: [(arrow_function) (function_expression)]) @definition.function

(lexical_declaration
  (variable_declarator
    name: (identifier) @name)) @definition.constant

(variable_declaration
  (variable_declarator
    name: (identifier) @name)) @definition.constant

(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (member_expression
    property: (property_identifier) @name)) @reference.call

(new_expression
  constructor: (_) @name) @reference.class

(export_statement value: (assignment_expression left: (identifier) @name)) @definition.constant

(function_signature
  name: (identifier) @name) @definition.function

(method_signature
  name: (property_identifier) @name) @definition.method

(abstract_method_signature
  name: (property_identifier) @name) @definition.method

(abstract_class_declaration
  name: (type_identifier) @name) @definition.class

(module
  name: (identifier) @name) @definition.module

(interface_declaration
  name: (type_identifier) @name) @definition.interface

(type_alias_declaration
  name: (type_identifier) @name) @definition.type

(enum_declaration
  name: (identifier) @name) @definition.enum

; enum members without value (e.g. Red in enum Color { Red, Green })
(enum_body
    name: (property_identifier) @name) @definition.enum_variant

; enum members with value (e.g. Down = 1)
(enum_assignment
    name: (property_identifier) @name) @definition.enum_variant

(type_annotation
  (type_identifier) @name) @reference.type
"#;

/// Extended Rust TAGS_QUERY that adds missing patterns from the built-in
/// tree-sitter-rust tags.scm: enum variants, trait method signatures,
/// const items, and static items.
const RUST_TAGS_QUERY: &str = r#"
; ADT definitions
(struct_item
    name: (type_identifier) @name) @definition.struct

(enum_item
    name: (type_identifier) @name) @definition.enum

(union_item
    name: (type_identifier) @name) @definition.union

; type aliases
(type_item
    name: (type_identifier) @name) @definition.type

; method definitions (inside impl/trait blocks)
(declaration_list
    (function_item
        name: (identifier) @name) @definition.method)

; function definitions
(function_item
    name: (identifier) @name) @definition.function

; trait definitions
(trait_item
    name: (type_identifier) @name) @definition.trait

; module definitions
(mod_item
    name: (identifier) @name) @definition.module

; macro definitions
(macro_definition
    name: (identifier) @name) @definition.macro

; --- Additional patterns not in built-in tags.scm ---

; enum variant definitions (e.g. AgentState::Waiting)
(enum_variant
    name: (identifier) @name) @definition.enum_variant

; trait method signatures (fn declarations without body)
(declaration_list
    (function_signature_item
        name: (identifier) @name) @definition.method)

; const items
(const_item
    name: (identifier) @name) @definition.constant

; static items
(static_item
    name: (identifier) @name) @definition.constant

; --- References ---

(call_expression
    function: (identifier) @name) @reference.call

(call_expression
    function: (field_expression
        field: (field_identifier) @name)) @reference.call

(macro_invocation
    macro: (identifier) @name) @reference.call

; implementations
(impl_item
    trait: (type_identifier) @name) @reference.implementation

(impl_item
    type: (type_identifier) @name
    !trait) @reference.implementation
"#;

/// Python custom TAGS_QUERY — GitHub code-navigation compliant.
/// Distinguishes methods from functions, detects class-level assignments as
/// enum variants, and module-level assignments as constants.
const PYTHON_TAGS_QUERY: &str = r#"
; Methods inside a class (defined before function so dedup keeps method)
(class_definition
  body: (block
    (function_definition
      name: (identifier) @name) @definition.method))

; Decorated methods (@staticmethod, @classmethod, etc.)
(class_definition
  body: (block
    (decorated_definition
      definition: (function_definition
        name: (identifier) @name) @definition.method)))

; Function definitions (same @name position as method is deduped)
(function_definition
  name: (identifier) @name) @definition.function

; Class definitions
(class_definition
  name: (identifier) @name) @definition.class

; Enum variants (class-level assignments)
; NOTE: Matches all class-level assignments, not just Enum subclasses
; (tree-sitter has no type information)
(class_definition
  body: (block
    (expression_statement
      (assignment
        left: (identifier) @name) @definition.enum_variant)))

; Module-level constants
(module
  (expression_statement
    (assignment
      left: (identifier) @name) @definition.constant))

; Function calls
(call
  function: [
    (identifier) @name
    (attribute
      attribute: (identifier) @name)
  ]) @reference.call
"#;

/// Go custom TAGS_QUERY — GitHub code-navigation compliant.
/// Distinguishes struct/interface from generic type, adds const/var definitions.
const GO_TAGS_QUERY: &str = r#"
; Function definitions
(function_declaration
  name: (identifier) @name) @definition.function

; Method definitions
(method_declaration
  name: (field_identifier) @name) @definition.method

; Struct types (defined before generic type so dedup keeps struct)
(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (struct_type))) @definition.struct

; Interface types
(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (interface_type))) @definition.interface

; Other type definitions (type aliases, etc.; struct/interface deduped away)
(type_declaration
  (type_spec
    name: (type_identifier) @name)) @definition.type

; Constant declarations (including iota)
(const_declaration
  (const_spec
    name: (identifier) @name)) @definition.constant

; Variable declarations
(var_declaration
  (var_spec
    name: (identifier) @name)) @definition.variable

; Interface method signatures
(interface_type
  (method_elem
    name: (field_identifier) @name)) @definition.method

; Function calls
(call_expression
  function: [
    (identifier) @name
    (parenthesized_expression (identifier) @name)
    (selector_expression field: (field_identifier) @name)
    (parenthesized_expression (selector_expression field: (field_identifier) @name))
  ]) @reference.call

; Type references
(type_identifier) @name @reference.type
"#;

static JS_TS_CONFIG: OnceLock<TagsConfiguration> = OnceLock::new();
static RUST_CONFIG: OnceLock<TagsConfiguration> = OnceLock::new();
static PYTHON_CONFIG: OnceLock<TagsConfiguration> = OnceLock::new();
static GO_CONFIG: OnceLock<TagsConfiguration> = OnceLock::new();

fn js_ts_config() -> &'static TagsConfiguration {
    JS_TS_CONFIG.get_or_init(|| {
        TagsConfiguration::new(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            JS_TS_TAGS_QUERY,
            tree_sitter_typescript::LOCALS_QUERY,
        )
        .expect("Failed to create JS/TS tags config")
    })
}

fn get_tags_config(language: &str) -> Option<&'static TagsConfiguration> {
    match language {
        "typescript" | "typescriptreact" | "javascript" | "javascriptreact" => Some(js_ts_config()),
        "rust" => Some(RUST_CONFIG.get_or_init(|| {
            TagsConfiguration::new(tree_sitter_rust::LANGUAGE.into(), RUST_TAGS_QUERY, "")
                .expect("Failed to create Rust tags config")
        })),
        "python" => Some(PYTHON_CONFIG.get_or_init(|| {
            TagsConfiguration::new(tree_sitter_python::LANGUAGE.into(), PYTHON_TAGS_QUERY, "")
                .expect("Failed to create Python tags config")
        })),
        "go" => Some(GO_CONFIG.get_or_init(|| {
            TagsConfiguration::new(tree_sitter_go::LANGUAGE.into(), GO_TAGS_QUERY, "")
                .expect("Failed to create Go tags config")
        })),
        _ => None,
    }
}

fn tags_config_for_extension(ext: &str) -> Option<&'static TagsConfiguration> {
    match ext {
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(js_ts_config()),
        "rs" => get_tags_config("rust"),
        "py" => get_tags_config("python"),
        "go" => get_tags_config("go"),
        _ => None,
    }
}

// === Text search (regex, unchanged) ===

fn search_files_inner(
    root_path: String,
    pattern: String,
    case_sensitive: Option<bool>,
    is_regex: Option<bool>,
    max_results: Option<usize>,
) -> Result<SearchResult, String> {
    let case_sensitive = case_sensitive.unwrap_or(false);
    let is_regex = is_regex.unwrap_or(false);
    let max_results = max_results.unwrap_or(1000);

    let regex_pattern = if is_regex {
        if case_sensitive {
            pattern.clone()
        } else {
            format!("(?i){}", pattern)
        }
    } else {
        let escaped = regex::escape(&pattern);
        if case_sensitive {
            escaped
        } else {
            format!("(?i){}", escaped)
        }
    };

    let re = Regex::new(&regex_pattern).map_err(|e| format!("Invalid pattern: {}", e))?;

    let root = Path::new(&root_path);
    let mut matches = Vec::new();
    let mut total_matches: usize = 0;
    let mut truncated = false;

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (line_idx, line) in content.lines().enumerate() {
            for m in re.find_iter(line) {
                total_matches += 1;
                if matches.len() < max_results {
                    matches.push(SearchMatch {
                        path: relative.clone(),
                        line_number: line_idx + 1,
                        line_content: line.to_string(),
                        match_start: m.start(),
                        match_end: m.end(),
                    });
                } else {
                    truncated = true;
                }
            }
        }
    }

    Ok(SearchResult {
        matches,
        total_matches,
        truncated,
    })
}

#[tauri::command]
pub async fn search_files(
    root_path: String,
    pattern: String,
    case_sensitive: Option<bool>,
    is_regex: Option<bool>,
    max_results: Option<usize>,
) -> Result<SearchResult, String> {
    tokio::task::spawn_blocking(move || {
        search_files_inner(root_path, pattern, case_sensitive, is_regex, max_results)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

// === Definition search (tree-sitter-tags) ===

fn language_extensions(language: &str) -> Vec<&'static str> {
    match language {
        "typescript" => vec!["ts", "tsx"],
        "typescriptreact" => vec!["ts", "tsx"],
        "javascript" => vec!["js", "jsx"],
        "javascriptreact" => vec!["js", "jsx"],
        "rust" => vec!["rs"],
        "python" => vec!["py"],
        "go" => vec!["go"],
        _ => vec![],
    }
}

fn is_go_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "byte"
            | "complex64"
            | "complex128"
            | "error"
            | "float32"
            | "float64"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "rune"
            | "string"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
            | "any"
            | "comparable"
    )
}

fn collect_definition_tags(
    ctx: &mut TagsContext,
    config: &TagsConfiguration,
    content: &[u8],
    relative_path: &str,
    symbol: &str,
    results: &mut Vec<DefinitionLocation>,
) {
    let (tags, _) = match ctx.generate_tags(config, content, None) {
        Ok(r) => r,
        Err(_) => return,
    };

    let before_len = results.len();
    let content_str = std::str::from_utf8(content).unwrap_or("");
    let lines: Vec<&str> = content_str.lines().collect();

    for tag_result in tags {
        let tag = match tag_result {
            Ok(t) => t,
            Err(_) => continue,
        };

        if !tag.is_definition {
            continue;
        }

        let name = match std::str::from_utf8(&content[tag.name_range.clone()]) {
            Ok(n) => n,
            Err(_) => continue,
        };

        if name != symbol {
            continue;
        }

        let kind = config.syntax_type_name(tag.syntax_type_id).to_string();
        let line_number = tag.span.start.row + 1;

        let line_content = lines
            .get(tag.span.start.row)
            .copied()
            .unwrap_or("")
            .to_string();

        // Convert byte offset to character offset for Monaco compatibility
        let column = line_content
            .get(..tag.span.start.column)
            .map(|prefix| prefix.chars().count())
            .unwrap_or(tag.span.start.column)
            + 1;

        results.push(DefinitionLocation {
            path: relative_path.to_string(),
            line_number,
            column,
            line_content,
            kind,
        });
    }

    // Deduplicate tags at the same position (e.g., arrow function matches
    // both @definition.function and @definition.constant). Keep the first
    // match which is the more specific pattern.
    // Only dedup within newly added items to avoid affecting prior results.
    let mut new_items: Vec<DefinitionLocation> = results.drain(before_len..).collect();
    new_items.sort_by_key(|d| (d.line_number, d.column));
    new_items.dedup_by(|a, b| {
        a.path == b.path && a.line_number == b.line_number && a.column == b.column
    });
    results.extend(new_items);
}

fn find_definition_inner(
    root_path: String,
    symbol: String,
    language: String,
) -> Result<Vec<DefinitionLocation>, String> {
    let extensions = language_extensions(&language);

    let Some(tags_config) = get_tags_config(&language) else {
        return find_definition_regex(root_path, symbol, language);
    };

    let root = Path::new(&root_path);
    let mut results = Vec::new();
    let mut ctx = TagsContext::new();

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        if !extensions.is_empty() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !extensions.contains(&ext) {
                continue;
            }
        }

        let content = match fs::read(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        collect_definition_tags(
            &mut ctx,
            tags_config,
            &content,
            &relative,
            &symbol,
            &mut results,
        );
    }

    // node_modules .d.ts fallback for JS/TS
    if results.is_empty()
        && matches!(
            language.as_str(),
            "typescript" | "typescriptreact" | "javascript" | "javascriptreact"
        )
    {
        let node_modules = root.join("node_modules");
        if node_modules.is_dir() {
            let walker = WalkBuilder::new(&node_modules)
                .git_ignore(false)
                .git_global(false)
                .git_exclude(false)
                .max_depth(Some(5))
                .build();

            for entry in walker.flatten() {
                if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                    continue;
                }
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.ends_with(".d.ts") {
                    continue;
                }

                let content = match fs::read(path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                collect_definition_tags(
                    &mut ctx,
                    tags_config,
                    &content,
                    &relative,
                    &symbol,
                    &mut results,
                );
            }
        }
    }

    Ok(results)
}

fn get_definition_patterns(language: &str) -> Vec<(&'static str, String)> {
    match language {
        "typescript" | "typescriptreact" | "javascript" | "javascriptreact" => {
            vec![
                (
                    "function",
                    r"\b(?:export\s+)?(?:async\s+)?function\s+{symbol}\b".to_string(),
                ),
                (
                    "variable",
                    r"\b(?:export\s+)?(?:const|let|var)\s+{symbol}\b".to_string(),
                ),
                ("class", r"\b(?:export\s+)?class\s+{symbol}\b".to_string()),
                (
                    "interface",
                    r"\b(?:export\s+)?interface\s+{symbol}\b".to_string(),
                ),
                ("type", r"\b(?:export\s+)?type\s+{symbol}\b".to_string()),
                ("enum", r"\b(?:export\s+)?enum\s+{symbol}\b".to_string()),
            ]
        }
        "rust" => {
            vec![
                (
                    "function",
                    r"\b(?:pub\s+)?(?:async\s+)?fn\s+{symbol}\b".to_string(),
                ),
                ("struct", r"\b(?:pub\s+)?struct\s+{symbol}\b".to_string()),
                ("enum", r"\b(?:pub\s+)?enum\s+{symbol}\b".to_string()),
                ("trait", r"\b(?:pub\s+)?trait\s+{symbol}\b".to_string()),
                ("type", r"\b(?:pub\s+)?type\s+{symbol}\b".to_string()),
                ("const", r"\b(?:pub\s+)?const\s+{symbol}\b".to_string()),
                ("mod", r"\b(?:pub\s+)?mod\s+{symbol}\b".to_string()),
            ]
        }
        "python" => {
            vec![
                ("function", r"\bdef\s+{symbol}\b".to_string()),
                ("class", r"\bclass\s+{symbol}\b".to_string()),
            ]
        }
        "go" => {
            vec![
                ("function", r"\bfunc\s+{symbol}\b".to_string()),
                ("type", r"\btype\s+{symbol}\b".to_string()),
                ("var", r"\bvar\s+{symbol}\b".to_string()),
                ("const", r"\bconst\s+{symbol}\b".to_string()),
            ]
        }
        _ => {
            vec![
                ("function", r"\bfunction\s+{symbol}\b".to_string()),
                ("class", r"\bclass\s+{symbol}\b".to_string()),
            ]
        }
    }
}

fn find_definition_regex(
    root_path: String,
    symbol: String,
    language: String,
) -> Result<Vec<DefinitionLocation>, String> {
    let escaped_symbol = regex::escape(&symbol);
    let patterns = get_definition_patterns(&language);
    let extensions = language_extensions(&language);

    let mut regexes: Vec<(&str, Regex)> = Vec::new();
    for (kind, pattern_template) in &patterns {
        let pattern = pattern_template.replace("{symbol}", &escaped_symbol);
        let re = Regex::new(&pattern).map_err(|e| format!("Invalid pattern: {}", e))?;
        regexes.push((kind, re));
    }

    let root = Path::new(&root_path);
    let mut results = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        if !extensions.is_empty() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !extensions.contains(&ext) {
                continue;
            }
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for (line_idx, line) in content.lines().enumerate() {
            for (kind, re) in &regexes {
                if let Some(m) = re.find(line) {
                    // Convert byte offset to character offset for Monaco compatibility
                    let column = line[..m.start()].chars().count() + 1;
                    results.push(DefinitionLocation {
                        path: relative.clone(),
                        line_number: line_idx + 1,
                        column,
                        line_content: line.to_string(),
                        kind: kind.to_string(),
                    });
                }
            }
        }
    }

    Ok(results)
}

#[tauri::command]
pub async fn find_definition(
    root_path: String,
    symbol: String,
    language: String,
) -> Result<Vec<DefinitionLocation>, String> {
    log::debug!(
        "find_definition called: root_path={}, symbol={}, language={}",
        root_path,
        symbol,
        language
    );
    let result =
        tokio::task::spawn_blocking(move || find_definition_inner(root_path, symbol, language))
            .await
            .map_err(|e| format!("task join error: {e}"))?;
    log::debug!(
        "find_definition result: {} items",
        result.as_ref().map_or(0, |v| v.len())
    );
    result
}

// === Reference search (tree-sitter-tags) ===

fn find_references_inner(root_path: String, symbol: String) -> Result<Vec<SearchMatch>, String> {
    let root = Path::new(&root_path);
    let mut results = Vec::new();
    let mut ctx = TagsContext::new();

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let Some(config) = tags_config_for_extension(ext) else {
            continue;
        };

        let content = match fs::read(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let (tags, _) = match ctx.generate_tags(config, &content, None) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let content_str = std::str::from_utf8(&content).unwrap_or("");
        let content_lines: Vec<&str> = content_str.lines().collect();

        for tag_result in tags {
            let tag = match tag_result {
                Ok(t) => t,
                Err(_) => continue,
            };

            let name = match std::str::from_utf8(&content[tag.name_range.clone()]) {
                Ok(n) => n,
                Err(_) => continue,
            };

            if name != symbol {
                continue;
            }

            // Skip Go builtin types in reference.type captures to reduce noise
            if ext == "go" && !tag.is_definition {
                let kind = config.syntax_type_name(tag.syntax_type_id);
                if kind == "type" && is_go_builtin_type(name) {
                    continue;
                }
            }

            let line_number = tag.span.start.row + 1;
            let line_content = content_lines
                .get(tag.span.start.row)
                .copied()
                .unwrap_or("")
                .to_string();

            let line_start = if tag.name_range.start == 0 {
                0
            } else {
                content[..tag.name_range.start]
                    .iter()
                    .rposition(|&b| b == b'\n')
                    .map(|p| p + 1)
                    .unwrap_or(0)
            };
            // Convert byte offsets to character offsets for Monaco compatibility
            let match_start = String::from_utf8_lossy(&content[line_start..tag.name_range.start])
                .chars()
                .count();
            let match_end = String::from_utf8_lossy(&content[line_start..tag.name_range.end])
                .chars()
                .count();

            results.push(SearchMatch {
                path: relative.clone(),
                line_number,
                line_content,
                match_start,
                match_end,
            });
        }
    }

    Ok(results)
}

#[tauri::command]
pub async fn find_references(
    root_path: String,
    symbol: String,
) -> Result<Vec<SearchMatch>, String> {
    tokio::task::spawn_blocking(move || find_references_inner(root_path, symbol))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

// === Document symbols (for outline view) ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSymbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
}

fn list_document_symbols_inner(
    file_path: String,
    language: String,
    root_path: Option<String>,
) -> Result<Vec<DocumentSymbol>, String> {
    let Some(tags_config) = get_tags_config(&language) else {
        return Ok(Vec::new());
    };

    // Validate file_path is within root_path to prevent path traversal
    if let Some(ref root) = root_path {
        let canonical_root =
            std::fs::canonicalize(root).map_err(|e| format!("ルートパス解決失敗: {e}"))?;
        let canonical_file =
            std::fs::canonicalize(&file_path).map_err(|e| format!("ファイルパス解決失敗: {e}"))?;
        if canonical_file.strip_prefix(&canonical_root).is_err() {
            return Err("ファイルパスが許可されたルート外です".to_string());
        }
    }

    let content =
        fs::read_to_string(&file_path).map_err(|e| format!("ファイル読み込み失敗: {e}"))?;
    let content_bytes = content.as_bytes();

    let mut ctx = TagsContext::new();
    let (tags, _) = ctx
        .generate_tags(tags_config, content_bytes, None)
        .map_err(|e| format!("タグ生成失敗: {e}"))?;

    let content_lines: Vec<&str> = content.lines().collect();
    let mut symbols = Vec::new();
    for tag_result in tags {
        let tag = match tag_result {
            Ok(t) => t,
            Err(_) => continue,
        };

        if !tag.is_definition {
            continue;
        }

        let name = match std::str::from_utf8(&content_bytes[tag.name_range.clone()]) {
            Ok(n) => n.to_string(),
            Err(_) => continue,
        };

        let kind = tags_config.syntax_type_name(tag.syntax_type_id).to_string();

        let line_content = content_lines.get(tag.span.start.row).copied().unwrap_or("");
        let column = line_content
            .get(..tag.span.start.column)
            .map(|prefix| prefix.chars().count())
            .unwrap_or(tag.span.start.column)
            + 1;

        symbols.push(DocumentSymbol {
            name,
            kind,
            line: tag.span.start.row + 1,
            column,
            end_line: tag.span.end.row + 1,
        });
    }

    // Sort by line number, deduplicate same position
    symbols.sort_by_key(|s| (s.line, s.column));
    symbols.dedup_by(|a, b| a.line == b.line && a.column == b.column && a.name == b.name);

    Ok(symbols)
}

#[tauri::command]
pub async fn list_document_symbols(
    file_path: String,
    language: String,
    root_path: Option<String>,
) -> Result<Vec<DocumentSymbol>, String> {
    tokio::task::spawn_blocking(move || list_document_symbols_inner(file_path, language, root_path))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();

        git2::Repository::init(dir.path()).unwrap();

        fs::write(
            dir.path().join("hello.ts"),
            "export function greet(name: string) {\n  return `Hello, ${name}!`;\n}\n\nconst greeting = greet('World');\n",
        )
        .unwrap();

        fs::write(
            dir.path().join("main.ts"),
            "import { greet } from './hello';\n\nconsole.log(greet('Test'));\n",
        )
        .unwrap();

        fs::write(
            dir.path().join("app.rs"),
            "pub fn main() {\n    println!(\"hello\");\n}\n\npub struct App {\n    name: String,\n}\n",
        )
        .unwrap();

        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(
            dir.path().join("node_modules/pkg/index.js"),
            "module.exports = {};\n",
        )
        .unwrap();

        fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();

        dir
    }

    #[test]
    fn test_search_files_basic() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result = search_files_inner(root, "greet".to_string(), None, None, None).unwrap();
        assert!(result.matches.len() >= 3);
        assert!(!result.truncated);
    }

    #[test]
    fn test_search_files_case_insensitive() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            search_files_inner(root, "GREET".to_string(), Some(false), None, None).unwrap();
        assert!(result.matches.len() >= 3);
    }

    #[test]
    fn test_search_files_case_sensitive() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result = search_files_inner(root, "GREET".to_string(), Some(true), None, None).unwrap();
        assert_eq!(result.matches.len(), 0);
    }

    #[test]
    fn test_search_files_regex() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            search_files_inner(root, r"greet\(".to_string(), None, Some(true), None).unwrap();
        assert!(result.matches.len() >= 2);
    }

    #[test]
    fn test_search_files_respects_gitignore() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            search_files_inner(root, "module.exports".to_string(), None, None, None).unwrap();
        assert_eq!(result.matches.len(), 0);
    }

    #[test]
    fn test_search_files_max_results() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result = search_files_inner(root, "greet".to_string(), None, None, Some(1)).unwrap();
        assert_eq!(result.matches.len(), 1);
        assert!(result.truncated);
        assert!(result.total_matches > 1);
    }

    #[test]
    fn test_find_definition_typescript() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            find_definition_inner(root, "greet".to_string(), "typescript".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "function");
        assert_eq!(result[0].line_number, 1);
        assert!(result[0].path.ends_with("hello.ts"));
    }

    #[test]
    fn test_find_definition_rust() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            find_definition_inner(root.clone(), "main".to_string(), "rust".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "function");

        let result = find_definition_inner(root, "App".to_string(), "rust".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "struct");
    }

    #[test]
    fn test_find_definition_typescript_const() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result =
            find_definition_inner(root, "greeting".to_string(), "typescript".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find const variable definition");
        assert!(result[0].path.ends_with("hello.ts"));
    }

    #[test]
    fn test_find_definition_python() {
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        fs::write(
            dir.path().join("app.py"),
            r#"from enum import Enum

def hello(name):
    return f'Hello, {name}!'

class MyApp:
    def run(self):
        pass

    @staticmethod
    def create():
        return MyApp()

class Color(Enum):
    RED = 1
    GREEN = 2
    BLUE = 3

MAX_SIZE = 100
"#,
        )
        .unwrap();

        let root = dir.path().to_string_lossy().to_string();

        // Top-level function
        let result =
            find_definition_inner(root.clone(), "hello".to_string(), "python".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "function");

        // Class
        let result =
            find_definition_inner(root.clone(), "MyApp".to_string(), "python".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "class");

        // Method (should be "method", not "function")
        let result =
            find_definition_inner(root.clone(), "run".to_string(), "python".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "method");

        // Decorated method (@staticmethod)
        let result =
            find_definition_inner(root.clone(), "create".to_string(), "python".to_string())
                .unwrap();
        assert!(!result.is_empty(), "Should find decorated method 'create'");
        assert_eq!(result[0].kind, "method");

        // Enum variant (class-level assignment)
        let result =
            find_definition_inner(root.clone(), "RED".to_string(), "python".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find enum variant RED");
        assert_eq!(result[0].kind, "enum_variant");

        // Module-level constant
        let result =
            find_definition_inner(root, "MAX_SIZE".to_string(), "python".to_string()).unwrap();
        assert!(
            !result.is_empty(),
            "Should find module-level constant MAX_SIZE"
        );
        assert_eq!(result[0].kind, "constant");
    }

    #[test]
    fn test_find_definition_go() {
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        fs::write(
            dir.path().join("main.go"),
            r#"package main

func Hello(name string) string {
	return "Hello, " + name
}

type Config struct {
	Name string
}

type Handler interface {
	Handle(req string) string
}

const MaxRetries = 3

var counter int
"#,
        )
        .unwrap();

        let root = dir.path().to_string_lossy().to_string();

        // Function
        let result =
            find_definition_inner(root.clone(), "Hello".to_string(), "go".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "function");

        // Struct (should be "struct", not generic "type")
        let result =
            find_definition_inner(root.clone(), "Config".to_string(), "go".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "struct");

        // Interface
        let result =
            find_definition_inner(root.clone(), "Handler".to_string(), "go".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find interface Handler");
        assert_eq!(result[0].kind, "interface");

        // Constant
        let result =
            find_definition_inner(root.clone(), "MaxRetries".to_string(), "go".to_string())
                .unwrap();
        assert!(!result.is_empty(), "Should find const MaxRetries");
        assert_eq!(result[0].kind, "constant");

        // Variable
        let result = find_definition_inner(root, "counter".to_string(), "go".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find var counter");
        assert_eq!(result[0].kind, "variable");
    }

    #[test]
    fn test_find_definition_regex_fallback() {
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        fs::write(
            dir.path().join("app.xyz"),
            "function doStuff() {\n  return 42;\n}\n",
        )
        .unwrap();

        let root = dir.path().to_string_lossy().to_string();

        let result =
            find_definition_inner(root, "doStuff".to_string(), "unknown_lang".to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].kind, "function");
    }

    #[test]
    fn test_find_definition_node_modules_dts_fallback() {
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        fs::write(
            dir.path().join("main.ts"),
            "import { useState } from 'react';\n",
        )
        .unwrap();

        fs::create_dir_all(dir.path().join("node_modules/@types/react")).unwrap();
        fs::write(
            dir.path().join("node_modules/@types/react/index.d.ts"),
            "export function useState<S>(initialState: S): [S, (s: S) => void];\n",
        )
        .unwrap();
        fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();

        let root = dir.path().to_string_lossy().to_string();

        let result =
            find_definition_inner(root, "useState".to_string(), "typescript".to_string()).unwrap();
        assert!(
            !result.is_empty(),
            "Should find definition in node_modules .d.ts"
        );
        assert!(result[0].path.contains("node_modules"));
    }

    #[test]
    fn test_find_references() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result = find_references_inner(root, "greet".to_string()).unwrap();
        assert!(
            result.len() >= 2,
            "Should find at least definition + references, got: {}",
            result.len()
        );
    }

    #[test]
    fn test_find_references_includes_definition_and_references() {
        let dir = setup_test_dir();
        let root = dir.path().to_string_lossy().to_string();

        let result = find_references_inner(root, "greet".to_string()).unwrap();
        let files: Vec<&str> = result.iter().map(|r| r.path.as_str()).collect();
        assert!(
            files.iter().any(|f| f.ends_with("hello.ts")),
            "Should include definition file"
        );
    }

    #[test]
    fn test_find_references_unsupported_extension_skipped() {
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        fs::write(dir.path().join("data.json"), r#"{"greet": "hello"}"#).unwrap();

        let root = dir.path().to_string_lossy().to_string();
        let result = find_references_inner(root, "greet".to_string()).unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_find_definition_rust_complex() {
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        fs::write(
            dir.path().join("lib.rs"),
            r#"use std::sync::OnceLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SearchMatch {
    pub path: String,
}

impl SearchMatch {
    pub fn new(path: String) -> Self {
        Self { path }
    }

    pub fn display(&self) -> String {
        format!("{}", self.path)
    }
}

pub trait Searchable {
    fn search(&self, query: &str) -> Vec<SearchMatch>;
}

pub enum SearchError {
    NotFound,
    InvalidQuery(String),
}

pub async fn search_files(root: String) -> Result<(), String> {
    Ok(())
}

pub fn find_definition(symbol: &str) -> Option<SearchMatch> {
    None
}

const MAX_RESULTS: usize = 1000;

mod inner {
    pub fn helper() {}
}

type MyResult<T> = std::result::Result<T, String>;
"#,
        )
        .unwrap();

        let root = dir.path().to_string_lossy().to_string();

        // Test struct definition
        let result =
            find_definition_inner(root.clone(), "SearchMatch".to_string(), "rust".to_string())
                .unwrap();
        assert!(
            !result.is_empty(),
            "Should find struct SearchMatch definition"
        );

        // Test impl method
        let result =
            find_definition_inner(root.clone(), "new".to_string(), "rust".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find impl method 'new'");

        // Test trait definition
        let result =
            find_definition_inner(root.clone(), "Searchable".to_string(), "rust".to_string())
                .unwrap();
        assert!(!result.is_empty(), "Should find trait Searchable");

        // Test enum definition
        let result =
            find_definition_inner(root.clone(), "SearchError".to_string(), "rust".to_string())
                .unwrap();
        assert!(!result.is_empty(), "Should find enum SearchError");

        // Test async function
        let result =
            find_definition_inner(root.clone(), "search_files".to_string(), "rust".to_string())
                .unwrap();
        assert!(!result.is_empty(), "Should find async fn search_files");

        // Test regular function
        let result = find_definition_inner(
            root.clone(),
            "find_definition".to_string(),
            "rust".to_string(),
        )
        .unwrap();
        assert!(!result.is_empty(), "Should find fn find_definition");

        // Test module
        let result =
            find_definition_inner(root.clone(), "inner".to_string(), "rust".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find mod inner");

        // Test function inside module
        let result =
            find_definition_inner(root.clone(), "helper".to_string(), "rust".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find fn helper inside mod");

        // Test enum variant
        let result =
            find_definition_inner(root.clone(), "NotFound".to_string(), "rust".to_string())
                .unwrap();
        assert!(!result.is_empty(), "Should find enum variant NotFound");
        assert_eq!(result[0].kind, "enum_variant");

        // Test enum variant with value
        let result =
            find_definition_inner(root.clone(), "InvalidQuery".to_string(), "rust".to_string())
                .unwrap();
        assert!(
            !result.is_empty(),
            "Should find enum variant InvalidQuery(String)"
        );

        // Test trait method signature
        let result =
            find_definition_inner(root.clone(), "search".to_string(), "rust".to_string()).unwrap();
        assert!(
            !result.is_empty(),
            "Should find trait method signature 'search'"
        );

        // Test const item
        let result =
            find_definition_inner(root, "MAX_RESULTS".to_string(), "rust".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find const MAX_RESULTS");
        assert_eq!(result[0].kind, "constant");
    }

    #[test]
    fn test_find_definition_typescript_enum_member() {
        let dir = TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        fs::write(
            dir.path().join("colors.ts"),
            "export enum Color {\n  Red,\n  Green,\n  Blue,\n}\n",
        )
        .unwrap();

        let root = dir.path().to_string_lossy().to_string();

        // enum declaration itself
        let result =
            find_definition_inner(root.clone(), "Color".to_string(), "typescript".to_string())
                .unwrap();
        assert!(!result.is_empty(), "Should find enum Color");
        assert_eq!(result[0].kind, "enum");

        // enum member
        let result =
            find_definition_inner(root.clone(), "Red".to_string(), "typescript".to_string())
                .unwrap();
        assert!(!result.is_empty(), "Should find enum member Red");
        assert_eq!(result[0].kind, "enum_variant");

        let result =
            find_definition_inner(root, "Blue".to_string(), "typescript".to_string()).unwrap();
        assert!(!result.is_empty(), "Should find enum member Blue");
        assert_eq!(result[0].kind, "enum_variant");
    }

    #[test]
    fn test_rust_tags_generation_debug() {
        let config = get_tags_config("rust").unwrap();
        let mut ctx = TagsContext::new();

        let source =
            b"pub fn hello() {}\npub struct Foo {}\nimpl Foo {\n    pub fn bar(&self) {}\n}\n";

        let (tags, had_error) = ctx.generate_tags(config, source, None).unwrap();
        assert!(!had_error, "generate_tags should not report errors");

        let mut tag_names = Vec::new();
        for tag_result in tags {
            let tag = tag_result.unwrap();
            let name = std::str::from_utf8(&source[tag.name_range.clone()]).unwrap();
            tag_names.push((name.to_string(), tag.is_definition));
        }

        let def_names: Vec<&str> = tag_names
            .iter()
            .filter(|(_, is_def)| *is_def)
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(def_names.contains(&"hello"), "Should find fn hello");
        assert!(def_names.contains(&"Foo"), "Should find struct Foo");
        assert!(def_names.contains(&"bar"), "Should find impl method bar");
    }

    #[test]
    fn test_find_definition_rust_actual_crate() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        // Use PROJECT ROOT (parent of src-tauri), like the actual app does
        let root = std::path::Path::new(manifest_dir)
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();

        // SearchMatch struct is defined in search.rs
        let result =
            find_definition_inner(root.clone(), "SearchMatch".to_string(), "rust".to_string())
                .unwrap();
        assert!(
            !result.is_empty(),
            "Should find SearchMatch in actual crate source"
        );

        // find_definition function
        let result = find_definition_inner(
            root.clone(),
            "find_definition".to_string(),
            "rust".to_string(),
        )
        .unwrap();
        assert!(
            !result.is_empty(),
            "Should find find_definition fn in actual crate source"
        );

        // Test generate_tags on the actual search.rs
        let config = get_tags_config("rust").unwrap();
        let mut ctx = TagsContext::new();
        let search_rs_path = format!("{}/src-tauri/src/search.rs", root);
        let search_rs = fs::read(&search_rs_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {}", search_rs_path, e));
        let (tags, _) = ctx
            .generate_tags(config, &search_rs, None)
            .expect("generate_tags should succeed");
        let mut def_count = 0;
        let mut ref_count = 0;
        for tag_result in tags {
            match tag_result {
                Ok(tag) => {
                    if tag.is_definition {
                        def_count += 1;
                    } else {
                        ref_count += 1;
                    }
                }
                Err(_) => {}
            }
        }
        assert!(def_count > 0, "Should find definitions in search.rs");
    }
}
