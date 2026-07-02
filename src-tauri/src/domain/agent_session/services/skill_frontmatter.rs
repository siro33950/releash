#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
}

/// Parse SKILL.md frontmatter delimited by `---` and extract `name` / `description`.
pub fn parse_skill_frontmatter(content: &str) -> Option<SkillFrontmatter> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut closed = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            closed = true;
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(unquote_frontmatter_value(value.trim()));
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            description = Some(unquote_frontmatter_value(value.trim()));
        }
    }

    if !closed {
        return None;
    }
    Some(SkillFrontmatter {
        name: name.unwrap_or_default(),
        description: description.unwrap_or_default(),
    })
}

fn unquote_frontmatter_value(value: &str) -> String {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let quote = bytes[0];
        if (quote == b'"' || quote == b'\'') && bytes[value.len() - 1] == quote {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_frontmatter_name_descriptionを読む() {
        let parsed = parse_skill_frontmatter(
            r#"---
name: review
description: "Review code changes"
---
# Body
"#,
        )
        .unwrap();

        assert_eq!(
            parsed,
            SkillFrontmatter {
                name: "review".to_string(),
                description: "Review code changes".to_string(),
            }
        );
    }

    #[test]
    fn test_skill_frontmatter_閉じdelimiterなしは無視する() {
        assert!(parse_skill_frontmatter("---\nname: review\n").is_none());
    }

    #[test]
    fn test_skill_frontmatter_frontmatterなしは無視する() {
        assert!(parse_skill_frontmatter("# Skill\nname: review\n").is_none());
    }
}
