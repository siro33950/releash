pub(crate) fn collect_configured_secret_values(
    app: &super::workflow_host::WorkflowRuntimeDependencies,
) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(config) = app.secrets.as_ref() {
        values.extend(config.configured_secret_values().unwrap_or_default());
    }
    values.extend(
        crate::domain::workflow::services::secret_masker::collect_secret_values_from_env_vars(
            std::env::vars(),
        ),
    );
    crate::domain::workflow::services::secret_masker::normalize_secret_values(values)
}
