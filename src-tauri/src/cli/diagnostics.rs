use std::path::{Path, PathBuf};

use super::api_client;
use serde::Deserialize;

use super::common::{self, CliError, CliSuccess};
use crate::adaptor::protocol::workflow::{DiagnosticReport, Severity};

pub(super) fn cmd_diagnostics(
    data_dir: &Path,
    dir: Option<PathBuf>,
    json: bool,
) -> Result<CliSuccess, CliError> {
    let dir = dir
        .map(|dir| -> Result<PathBuf, CliError> {
            let cwd = std::env::current_dir()
                .map_err(|error| CliError::Other(format!("resolve current directory: {error}")))?;
            let dir = absolutize(dir, &cwd);
            ensure_existing_target_dir(&dir)?;
            Ok(dir)
        })
        .transpose()?;
    let dir = dir.as_deref().map(target_dir_str).transpose()?;
    let value =
        api_client::read_without_fallback(data_dir, |client| client.workflow_diagnostics(dir))?;
    let report = DiagnosticReport::deserialize(&value)
        .map_err(|error| CliError::Other(format!("decode workflow diagnostics: {error}")))?;
    let exit_code = diagnostics_exit_code(&report);
    let stdout = if json {
        format_json(&value)?
    } else {
        format_human_readable(&report)
    };
    Ok(CliSuccess::with_exit_code(stdout, exit_code))
}

/// `--dir` を process の cwd 基準で絶対 path へ解決する。local API へは絶対 path
/// だけを渡すため、診断対象がアプリ process の cwd に依存しない。
fn absolutize(dir: PathBuf, cwd: &Path) -> PathBuf {
    if dir.is_absolute() {
        dir
    } else {
        cwd.join(dir)
    }
}

fn ensure_existing_target_dir(path: &Path) -> Result<(), CliError> {
    if !path.exists() {
        return Err(CliError::NotFound(format!(
            "directory does not exist: {}",
            path.display()
        )));
    }
    Ok(())
}

fn target_dir_str(path: &Path) -> Result<&str, CliError> {
    path.to_str().ok_or_else(|| {
        CliError::InvalidInput(format!(
            "diagnostics target directory must be valid UTF-8: {}",
            path.display()
        ))
    })
}

/// severity error の item が 1 件以上あれば非 zero。
/// workflow_summaries / facet_summaries の error_count は合算しない。
/// add_diagnostic_to_workflow_and_facet が 1 件の item を workflow 側と facet 側の両方の
/// summary へ計上するため、合算すると二重計上になる。
fn diagnostics_exit_code(report: &DiagnosticReport) -> i32 {
    if report
        .items
        .iter()
        .any(|item| item.severity == Severity::Error)
    {
        common::DIAGNOSTIC_ERRORS_EXIT_CODE
    } else {
        0
    }
}

fn format_json(value: &serde_json::Value) -> Result<String, CliError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| CliError::Other(format!("serialize workflow diagnostics: {error}")))?;
    Ok(format!("{text}\n"))
}

fn format_human_readable(report: &DiagnosticReport) -> String {
    let mut output = String::new();
    let mut error_count = 0;
    let mut info_count = 0;
    for item in &report.items {
        let severity = match item.severity {
            Severity::Error => {
                error_count += 1;
                "error"
            }
            Severity::Info => {
                info_count += 1;
                "info"
            }
        };
        let location = item.span.as_ref().map_or_else(String::new, |span| {
            span.source.as_ref().map_or_else(
                || format!(" {}:{}", span.start_line, span.start_col),
                |source| format!(" {source}:{}:{}", span.start_line, span.start_col),
            )
        });
        let mut targets = Vec::new();
        if let Some(workflow_name) = &item.workflow_name {
            targets.push(format!("workflow={workflow_name}"));
        }
        if let Some(node_name) = &item.node_name {
            targets.push(format!("node={node_name}"));
        }
        if let (Some(facet_kind), Some(facet_key)) = (&item.facet_kind, &item.facet_key) {
            targets.push(format!("facet={facet_kind}/{facet_key}"));
        }
        if let Some(field) = &item.field {
            targets.push(format!("field={field}"));
        }
        let target = if targets.is_empty() {
            String::new()
        } else {
            format!(" [{}]", targets.join(", "))
        };
        output.push_str(&format!(
            "{severity} {}{location}{target}: {}\n",
            item.code, item.message
        ));
    }
    output.push('\n');
    output.push_str(&format!("{error_count} error, {info_count} info\n"));
    output
}

#[cfg(test)]
#[path = "diagnostics_test.rs"]
mod diagnostics_tests;
