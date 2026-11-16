//! Content-free audit lines on stderr (backlog §8): ids, provider ids, durations
//! and counts. Values are restricted to a charset that cannot carry prose, so a
//! caller that passes page text by mistake gets underscores, not a leak.

use std::fmt::Display;

use ai_platform_contract::identity::TaskId;

fn sanitize(value: &str) -> String {
    value
        .chars()
        .take(64)
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-') { c } else { '_' })
        .collect()
}

pub fn format_line(event: &str, task: Option<TaskId>, fields: &[(&str, &dyn Display)]) -> String {
    let mut line = format!("ai-utility audit event={}", sanitize(event));
    if let Some(task) = task {
        line.push_str(&format!(" task={}", task.0));
    }
    for (key, value) in fields {
        line.push_str(&format!(" {}={}", sanitize(key), sanitize(&value.to_string())));
    }
    line
}

pub fn line(event: &str, task: Option<TaskId>, fields: &[(&str, &dyn Display)]) {
    eprintln!("{}", format_line(event, task, fields));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_cannot_carry_prose() {
        let line = format_line("task.finish", Some(TaskId(7)), &[("provider", &"extractive"), ("note", &"secret page text = here")]);
        assert!(line.contains("task=7") && line.contains("provider=extractive"));
        assert!(!line.contains("secret page"));
        assert!(line.contains("note=secret_page_text___here"));
    }
}
