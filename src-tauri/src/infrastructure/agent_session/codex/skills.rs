use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexSkillSource {
    User,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexSkillFile {
    pub source: CodexSkillSource,
    pub content: String,
}

pub(crate) fn read_skill_files(cwd: &Path) -> Vec<CodexSkillFile> {
    let mut files = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        files.extend(read_dir(
            &home.join(".agents").join("skills"),
            CodexSkillSource::User,
        ));
    }
    files.extend(read_dir(
        &cwd.join(".agents").join("skills"),
        CodexSkillSource::Project,
    ));
    files
}

fn read_dir(dir: &Path, source: CodexSkillSource) -> Vec<CodexSkillFile> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(skill_md) else {
            continue;
        };
        files.push(CodexSkillFile { source, content });
    }
    files
}
