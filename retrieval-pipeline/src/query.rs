/// Normalize raw input into FTS-friendly terms and return the token count.
pub fn normalize_fts_terms(input: &str) -> (String, usize) {
    const STOPWORDS: &[&str] = &["the", "a", "an", "of", "in", "on", "and", "or", "to", "for"];
    let mut cleaned = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_alphanumeric() {
            cleaned.extend(ch.to_lowercase());
        } else if ch.is_whitespace() {
            cleaned.push(' ');
        }
    }
    let mut tokens = Vec::with_capacity(cleaned.len().div_ceil(3));
    for token in cleaned.split_whitespace() {
        if !STOPWORDS.contains(&token) && !token.is_empty() {
            tokens.push(token.to_string());
        }
    }
    let normalized = tokens.join(" ");
    (normalized, tokens.len())
}

#[cfg(test)]
mod tests {
    use super::normalize_fts_terms;

    #[test]
    fn strips_stopwords_and_lowercases() {
        let (query, count) = normalize_fts_terms("The Cucumber and Tomatoes");
        assert_eq!(query, "cucumber tomatoes");
        assert_eq!(count, 2);
    }

    #[test]
    fn returns_empty_for_stopwords_only() {
        let (query, count) = normalize_fts_terms("the and or");
        assert!(query.is_empty());
        assert_eq!(count, 0);
    }
}
