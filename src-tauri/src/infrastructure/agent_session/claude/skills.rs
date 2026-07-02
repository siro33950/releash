use std::path::{Path, PathBuf};

use crate::domain::agent_session::services::{
    filter_agent_skills_for_query, parse_skill_frontmatter,
};
use crate::domain::agent_session::value_objects::SkillEntry;

pub(crate) fn scan_claude_agent_skills(
    cwd: &Path,
    query: Option<&str>,
    limit: Option<usize>,
) -> Vec<SkillEntry> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut skills = Vec::new();
    if let Some(home) = home {
        skills.extend(scan_dir(&home.join(".claude").join("skills"), "personal"));
    }
    skills.extend(scan_dir(&cwd.join(".claude").join("skills"), "project"));
    filter_agent_skills_for_query(skills, query, limit)
}

fn scan_dir(dir: &Path, scope: &str) -> Vec<SkillEntry> {
    let mut skills = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return skills;
    };
    for entry in entries.flatten() {
        let skill_md = entry.path().join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(skill_md) else {
            continue;
        };
        let Some(frontmatter) = parse_skill_frontmatter(&content) else {
            continue;
        };
        if frontmatter.name.is_empty() {
            continue;
        }
        skills.push(SkillEntry {
            name: frontmatter.name,
            description: frontmatter.description,
            scope: scope.to_string(),
        });
    }
    skills
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_claude_agent_skills_project_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp
            .path()
            .join(".claude/skills/claude-project-review-unique");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: claude-project-review-unique\ndescription: Review code\n---\n",
        )
        .unwrap();

        let skills =
            scan_claude_agent_skills(tmp.path(), Some("claude-project-review-unique"), Some(5));

        assert_eq!(
            skills,
            vec![SkillEntry {
                name: "claude-project-review-unique".to_string(),
                description: "Review code".to_string(),
                scope: "project".to_string(),
            }]
        );
    }
}
