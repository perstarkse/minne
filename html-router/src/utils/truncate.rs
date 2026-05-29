const ELLIPSIS: &str = "…";

/// Truncates `value` to at most `max_chars` Unicode scalars, appending an ellipsis when shortened.
pub fn with_ellipsis(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return if value.is_empty() {
            String::new()
        } else {
            ELLIPSIS.to_string()
        };
    }

    let mut end_byte = value.len();
    for (count, (idx, _)) in value.char_indices().enumerate() {
        if count == max_chars {
            end_byte = idx;
            break;
        }
    }

    if end_byte == value.len() {
        return value.to_string();
    }

    format!("{}{}", &value[..end_byte], ELLIPSIS)
}

/// Returns the first non-empty line of `text`, truncated with an ellipsis when needed.
pub fn first_non_empty_line(text: &str, max_chars: usize) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(with_ellipsis(trimmed, max_chars))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{first_non_empty_line, with_ellipsis};

    #[test]
    fn leaves_short_strings_unchanged() {
        assert_eq!(with_ellipsis("hello", 10), "hello");
    }

    #[test]
    fn truncates_at_char_boundary_with_ellipsis() {
        assert_eq!(with_ellipsis("hello world", 5), "hello…");
    }

    #[test]
    fn first_non_empty_line_skips_blank_lines() {
        assert_eq!(
            first_non_empty_line("\n  \nTitle line\nBody", 20),
            Some("Title line".to_string())
        );
    }
}
