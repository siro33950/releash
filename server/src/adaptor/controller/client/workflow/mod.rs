pub(crate) mod shared;
pub(crate) use shared::register_shared;

#[cfg(any(test, feature = "test-support"))]
use crate::adaptor::gateway::workflow::builtin;
#[cfg(any(test, feature = "test-support"))]
use crate::adaptor::gateway::workflow::facet::FacetKind;
#[cfg(any(test, feature = "test-support"))]
use crate::adaptor::gateway::workflow::schema::FacetSummary as GatewayFacetSummary;
#[cfg(any(test, feature = "test-support"))]
use std::path::Path;

pub(crate) mod definition;
pub(crate) mod diagnostics;
pub(crate) mod facet;
pub(crate) mod output;
pub(crate) mod runtime;
#[cfg(test)]
pub(crate) mod session_errors;

#[cfg(test)]
use self::session_errors::redacted_workflow_tab_error;

#[cfg(any(test, feature = "test-support"))]
fn parse_facet_kind(kind: &str) -> Result<FacetKind, String> {
    match kind {
        "policy" => Ok(FacetKind::Policy),
        "knowledge" => Ok(FacetKind::Knowledge),
        "instruction" => Ok(FacetKind::Instruction),
        _ => Err(format!("Unknown facet kind: {kind}")),
    }
}

// ---- ファセットコマンドの内部実装（テスト可能な純粋関数として切り出し） ----
//
// Tauri コマンドはこれらの inner 関数に委譲する。インテグレーションを
// テンポラリディレクトリ上で再現することで、3 種それぞれの正常経路到達と、
// 廃止済み種別および未知種別での I/O 非発生を直接検証できるようにする。

#[cfg(any(test, feature = "test-support"))]
pub fn list_facets_inner(kind: &str, base_dir: &Path) -> Result<Vec<String>, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::list_facets(facet_kind, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(any(test, feature = "test-support"))]
pub fn get_facet_inner(kind: &str, key: &str, base_dir: &Path) -> Result<String, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::load_facet(facet_kind, key, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(any(test, feature = "test-support"))]
pub fn save_facet_inner(
    kind: &str,
    key: &str,
    content: &str,
    is_new: bool,
    base_dir: &Path,
) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    if builtin::is_builtin_facet(facet_kind, key) {
        return Err("ビルトインファセットは編集できません".to_string());
    }
    validate_template_variables(content)?;
    if is_new {
        let existing = crate::adaptor::gateway::workflow::facet::list_facets(facet_kind, base_dir)
            .map_err(|e| e.to_string())?;
        if existing.contains(&key.to_string()) {
            return Err(format!("ファセット '{key}' は既に存在します"));
        }
    }
    crate::adaptor::gateway::workflow::facet::save_facet(facet_kind, key, content, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(any(test, feature = "test-support"))]
pub fn delete_facet_inner(kind: &str, key: &str, base_dir: &Path) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    if builtin::is_builtin_facet(facet_kind, key) {
        return Err("ビルトインファセットは削除できません".to_string());
    }
    crate::adaptor::gateway::workflow::facet::delete_facet(facet_kind, key, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(any(test, feature = "test-support"))]
pub fn list_facet_summaries_inner(
    kind: &str,
    base_dir: &Path,
) -> Result<Vec<GatewayFacetSummary>, String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::list_facet_summaries(facet_kind, base_dir)
        .map_err(|e| e.to_string())
}

#[cfg(any(test, feature = "test-support"))]
pub fn duplicate_facet_inner(
    kind: &str,
    source_key: &str,
    new_key: &str,
    base_dir: &Path,
) -> Result<(), String> {
    let facet_kind = parse_facet_kind(kind)?;
    crate::adaptor::gateway::workflow::facet::validate_facet_key(new_key)
        .map_err(|e| e.to_string())?;
    let existing = crate::adaptor::gateway::workflow::facet::list_facets(facet_kind, base_dir)
        .map_err(|e| e.to_string())?;
    if existing.contains(&new_key.to_string()) {
        return Err(format!("ファセット '{new_key}' は既に存在します"));
    }
    let content =
        crate::adaptor::gateway::workflow::facet::load_facet(facet_kind, source_key, base_dir)
            .map_err(|e| e.to_string())?;
    crate::adaptor::gateway::workflow::facet::save_facet(facet_kind, new_key, &content, base_dir)
        .map_err(|e| e.to_string())
}

/// `open_facet_in_editor` の中核ロジック。エディタ起動はテストで差し替え可能にするため
/// `opener` を引数で受け取る（production では実エディタ起動を渡す）。
#[cfg(any(test, feature = "test-support"))]
pub fn open_facet_in_editor_inner<F>(
    kind: &str,
    key: &str,
    base_dir: &Path,
    opener: F,
) -> Result<(), String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let facet_kind = parse_facet_kind(kind)?;
    if builtin::is_builtin_facet(facet_kind, key) {
        return Err("ビルトインファセットは外部エディタで開けません".to_string());
    }
    let file_path =
        crate::adaptor::gateway::workflow::facet::resolve_facet_path(facet_kind, key, base_dir)
            .map_err(|e| e.to_string())?;
    let path_str = file_path.to_string_lossy().to_string();
    opener(&path_str)
}

#[cfg(test)]
fn validation_error_string(
    e: crate::domain::workflow::services::validation::ValidationError,
) -> String {
    format!("validation_error: {e}")
}

// ---- ワークフロー実行コマンド ----

#[cfg(test)]
fn parse_execution_origin(
    value: Option<String>,
) -> Result<crate::domain::workflow::ExecutionOrigin, String> {
    value
        .as_deref()
        .map(crate::domain::workflow::ExecutionOrigin::from_public_value)
        .unwrap_or(Ok(crate::domain::workflow::ExecutionOrigin::DesktopUi))
        .map_err(|error| error.to_string())
}

/// `execution_id` の形式検証（path traversal / 不正文字対策）。
/// UUID（RFC 4122）形式のみ許容する。
fn validate_execution_id(
    execution_id: &str,
) -> Result<(), crate::adaptor::presenter::error::AppError> {
    uuid::Uuid::parse_str(execution_id)
        .map(|_| ())
        .map_err(|_| {
            crate::adaptor::presenter::error::AppError::invalid_request(
                "Invalid execution_id format (must be UUID)",
            )
        })
}

// ---- [05] read-only execution 観測 API ----
//
// `execution_id` 主語で workflow execution を観測する read-only API。

// ---- 新規コマンド ----

#[cfg(any(test, feature = "test-support"))]
fn validate_template_variables(content: &str) -> Result<(), String> {
    let errors =
        crate::adaptor::gateway::workflow::workflow_host::prompt_rendering::find_undefined_template_variables(
            content,
        );
    if !errors.is_empty() {
        return Err(format!(
            "未定義のテンプレート変数が含まれています: {}",
            errors.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_tests;
