//! Node runtime setting resolution.

/// workflow 起動時に確定する node session の継承デフォルト。
///
/// `start_workflow` の `permission_mode` 引数を capture し、以降の node / 並列子 node は
/// この値を fallback として `NodeDefinition.model` / `NodeDefinition.permission` で上書きする。
///
/// `selected_model` は spec [02] の暗黙フォールバック禁止に従い workflow デフォルトとしては
/// 持たない（各 node は `NodeDefinition.model` 必須で個別に解決する）。`backend_id` も
/// 各 node が `NodeDefinition.model` 必須から `resolve_backend_for_node_model` 経由で
/// 一意解決するため、node 指定が無い場合の fallback としてのみ保持する。
pub(crate) use crate::domain::workflow::entities::workflow_execution::WorkflowDefaults;

/// ステップ設定解決の結果。
/// ステップのmodel/permission指定と親セッション設定のマージ結果を保持する。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedNodeSettings {
    pub(crate) backend_id: Option<String>,
    pub(crate) selected_model: Option<String>,
    pub(crate) permission_mode: String,
}

/// ステップの model/permission 設定を workflow デフォルトとマージして解決する。
///
/// - permission: ステップ指定があれば採用、なければ workflow デフォルトを継承
/// - backend_id: model指定があれば resolved_backend_id を採用、なければ workflow デフォルトを継承
/// - selected_model: ステップ指定があれば採用、なければ未指定（None）として扱う。
///   Spec: workflow 経路の `model_id=None` は当該 node session の選択モデルを
///   未指定状態のままとし、workflow デフォルト model への暗黙フォールバックを行わない。
///
/// `resolved_backend_id` は、ステップにmodel指定がある場合に
/// `resolve_backend_for_node_model` で事前に解決されたbackend_id。
/// model未指定時は無視される。
pub(crate) fn resolve_node_settings(
    node_model: Option<String>,
    node_permission: Option<String>,
    resolved_backend_id: Option<String>,
    workflow_defaults: &WorkflowDefaults,
) -> ResolvedNodeSettings {
    let permission_mode =
        node_permission.unwrap_or_else(|| workflow_defaults.permission_mode.clone());
    let backend_id = if node_model.is_some() {
        resolved_backend_id
    } else {
        workflow_defaults.backend_id.clone()
    };
    let selected_model = node_model;
    ResolvedNodeSettings {
        backend_id,
        selected_model,
        permission_mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_node_settings_uses_node_model_backend_and_permission_override() {
        let result = resolve_node_settings(
            Some("opus-4".to_string()),
            Some("ask".to_string()),
            Some("claude".to_string()),
            &WorkflowDefaults {
                backend_id: Some("codex".to_string()),
                permission_mode: "edit".to_string(),
            },
        );

        assert_eq!(
            result,
            ResolvedNodeSettings {
                backend_id: Some("claude".to_string()),
                selected_model: Some("opus-4".to_string()),
                permission_mode: "ask".to_string(),
            }
        );
    }

    #[test]
    fn resolve_node_settings_inherits_defaults_without_implicit_model_fallback() {
        let result = resolve_node_settings(
            None,
            None,
            None,
            &WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "full".to_string(),
            },
        );

        assert_eq!(
            result,
            ResolvedNodeSettings {
                backend_id: Some("claude".to_string()),
                selected_model: None,
                permission_mode: "full".to_string(),
            }
        );
    }
}
