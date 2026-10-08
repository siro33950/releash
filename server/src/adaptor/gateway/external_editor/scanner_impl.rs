use std::path::PathBuf;

use crate::domain::external_editor::services::scan_applications_in;
use crate::domain::external_editor::{EditorInfo, InstalledEditorGateway};

pub struct MacInstalledEditorGateway;

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Applications"));
    }
    dirs
}

impl InstalledEditorGateway for MacInstalledEditorGateway {
    fn scan(&self) -> Vec<EditorInfo> {
        scan_applications_in(&application_dirs())
    }
}

#[cfg(test)]
#[path = "scanner_impl_test.rs"]
mod scanner_impl_tests;
