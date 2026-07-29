use std::path::Path;

use crate::domain::agent_session::services::{
    filter_agent_skills_for_query, parse_skill_frontmatter,
};
use crate::domain::agent_session::value_objects::SkillEntry;
use crate::infrastructure::agent_session::claude::skills::{read_skill_files, ClaudeSkillSource};

pub(crate) fn scan_claude_agent_skills(
    cwd: &Path,
    query: Option<&str>,
    limit: Option<usize>,
) -> Vec<SkillEntry> {
    let skills = read_skill_files(cwd)
        .into_iter()
        .filter_map(|file| {
            let frontmatter = parse_skill_frontmatter(&file.content)?;
            if frontmatter.name.is_empty() {
                return None;
            }
            Some(SkillEntry {
                name: frontmatter.name,
                description: frontmatter.description,
                scope: match file.source {
                    ClaudeSkillSource::User => "personal",
                    ClaudeSkillSource::Project => "project",
                }
                .to_string(),
            })
        })
        .collect();
    filter_agent_skills_for_query(skills, query, limit)
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
