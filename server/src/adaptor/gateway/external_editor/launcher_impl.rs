use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::domain::external_editor::EditorError;
use crate::domain::external_editor::EditorLauncherGateway;

#[derive(Clone)]
pub struct NativeEditorLauncherGateway;

impl EditorLauncherGateway for NativeEditorLauncherGateway {
    fn open_paths(
        &self,
        paths: &[std::path::PathBuf],
        editor: &str,
        label: &str,
    ) -> Result<(), EditorError> {
        open_paths_with(paths, editor, label, |mut command| {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            let mut child = command.spawn()?;
            std::thread::spawn(move || {
                if let Err(error) = child.wait() {
                    log::warn!("Editor launcher wait failed: {error}");
                }
            });
            Ok(())
        })
    }

    fn open_path(&self, path: &str, editor: &str, label: &str) -> Result<(), EditorError> {
        open_path_with(path, editor, label, |path, editor| match editor {
            Some(editor) => open::with_detached(path, editor),
            None => open::that_detached(path),
        })
    }
}

pub fn open_path_with(
    path: &str,
    editor: &str,
    label: &str,
    open: impl FnOnce(&str, Option<&str>) -> std::io::Result<()>,
) -> Result<(), EditorError> {
    if editor.is_empty() {
        std::fs::metadata(path)
            .and_then(|_| open(path, None))
            .map_err(|e| EditorError::Launch(format!("{label}を開けませんでした: {e}")))
    } else {
        open(path, Some(editor))
            .map_err(|e| EditorError::Launch(format!("エディタで{label}を開けませんでした: {e}")))
    }
}

fn open_paths_with(
    paths: &[PathBuf],
    editor: &str,
    label: &str,
    spawn: impl FnOnce(Command) -> std::io::Result<()>,
) -> Result<(), EditorError> {
    let launch = || {
        let (first, remaining) = paths.split_first().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "開くパスがありません")
        })?;
        let mut command = if editor.is_empty() {
            for path in paths {
                std::fs::metadata(path)?;
            }
            open::commands(first).into_iter().next().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "開くコマンドが見つかりません")
            })?
        } else {
            open::with_command(first, editor)
        };
        command.args(remaining);
        spawn(command)
    };
    launch().map_err(|error| {
        EditorError::Launch(format!("エディタで{label}を開けませんでした: {error}"))
    })
}

#[cfg(test)]
#[path = "launcher_impl_test.rs"]
mod launcher_impl_tests;
