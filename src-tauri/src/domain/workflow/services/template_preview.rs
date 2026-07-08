use std::collections::HashMap;

pub fn render_template_variables(content: &str, values: &HashMap<String, String>) -> String {
    replace_template_refs(content, |inner| values.get(inner).cloned())
}

fn replace_template_refs(content: &str, mut resolve: impl FnMut(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while !rest.is_empty() {
        let Some(open_idx) = rest.find("{{") else {
            result.push_str(rest);
            break;
        };
        result.push_str(&rest[..open_idx]);
        let after_open = &rest[open_idx + 2..];
        let Some(close_idx) = after_open.find("}}") else {
            result.push_str("{{");
            result.push_str(after_open);
            break;
        };
        let raw_inner = &after_open[..close_idx];
        match resolve(raw_inner.trim()) {
            Some(value) => result.push_str(&value),
            None => {
                result.push_str("{{");
                result.push_str(raw_inner);
                result.push_str("}}");
            }
        }
        rest = &after_open[close_idx + 2..];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_sample_template_variables_and_keeps_unknown_refs() {
        let values = HashMap::from([("request".to_string(), "write tests".to_string())]);

        assert_eq!(
            render_template_variables("Task: {{ request }} {{ missing }}", &values),
            "Task: write tests {{ missing }}"
        );
    }
}
