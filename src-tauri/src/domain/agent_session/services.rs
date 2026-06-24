use super::SkillEntry;

pub(crate) fn filter_agent_skills_for_query(
    skills: Vec<SkillEntry>,
    query: Option<&str>,
    limit: Option<usize>,
) -> Vec<SkillEntry> {
    let needle = query.unwrap_or_default().trim().to_lowercase();
    let max_results = limit.unwrap_or(usize::MAX);
    skills
        .into_iter()
        .filter(|skill| {
            needle.is_empty()
                || skill.name.to_lowercase().contains(&needle)
                || skill.description.to_lowercase().contains(&needle)
        })
        .take(max_results)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_agent_skills_filters_by_query_and_limit() {
        let skills = vec![
            SkillEntry {
                name: "review".to_string(),
                description: "Review code changes".to_string(),
                scope: "project".to_string(),
            },
            SkillEntry {
                name: "docs".to_string(),
                description: "Write documentation".to_string(),
                scope: "personal".to_string(),
            },
            SkillEntry {
                name: "diagram".to_string(),
                description: "Document architecture diagrams".to_string(),
                scope: "project".to_string(),
            },
        ];

        let result = filter_agent_skills_for_query(skills.clone(), Some("doc"), Some(1));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "docs");

        let result = filter_agent_skills_for_query(skills, Some("review"), Some(20));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "review");
    }
}
