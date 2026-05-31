//! Fast-path PDF text extraction and Markdown reflow heuristics.
//!
//! These are pure (non-IO, non-Chrome) helpers used before falling back to the
//! vision pipeline, plus the Markdown normalization applied to both paths.

use common::error::AppError;

const FAST_PATH_MIN_LEN: usize = 150;
const FAST_PATH_MIN_ASCII_RATIO: f64 = 0.7;

/// Runs `pdf-extract` on the PDF bytes and validates the result with simple heuristics.
/// Returns `Ok(None)` when the text layer is missing or too noisy.
pub(super) async fn try_fast_path(pdf_bytes: Vec<u8>) -> Result<Option<String>, AppError> {
    let extraction = tokio::task::spawn_blocking(move || {
        pdf_extract::extract_text_from_mem(&pdf_bytes).map(|s| s.trim().to_string())
    })
    .await?
    .map_err(|err| AppError::Processing(format!("Failed to extract text from PDF: {err}")))?;

    if extraction.is_empty() {
        return Ok(None);
    }

    if !looks_good_enough(&extraction) {
        return Ok(None);
    }

    Ok(Some(normalize_fast_text(&extraction)))
}

/// Heuristic that determines whether the fast-path text looks like well-formed prose.
#[allow(clippy::cast_precision_loss)]
fn looks_good_enough(text: &str) -> bool {
    if text.len() < FAST_PATH_MIN_LEN {
        return false;
    }

    let total_chars = text.chars().count() as f64;
    if total_chars == 0.0 {
        return false;
    }

    let ascii_chars = text.chars().filter(char::is_ascii).count() as f64;
    let ascii_ratio = ascii_chars / total_chars;
    if ascii_ratio < FAST_PATH_MIN_ASCII_RATIO {
        return false;
    }

    let letters = text.chars().filter(|c| c.is_alphabetic()).count() as f64;
    let letter_ratio = letters / total_chars;
    letter_ratio > 0.3
}

/// Normalizes fast-path output so downstream consumers see consistent Markdown.
fn normalize_fast_text(text: &str) -> String {
    reflow_markdown(text)
}

/// Cleans, trims, and reflows Markdown created by the LLM path.
pub(super) fn post_process(markdown: &str) -> String {
    let cleaned = markdown.replace('\r', "");
    let trimmed = cleaned.trim();
    reflow_markdown(trimmed)
}

/// Joins hard-wrapped paragraph text while preserving structural Markdown lines.
fn reflow_markdown(input: &str) -> String {
    let mut paragraphs = Vec::new();
    let mut buffer: Vec<String> = Vec::new();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !buffer.is_empty() {
                paragraphs.push(buffer.join(" "));
                buffer.clear();
            }
            continue;
        }

        if is_structural_line(trimmed) {
            if !buffer.is_empty() {
                paragraphs.push(buffer.join(" "));
                buffer.clear();
            }
            paragraphs.push(trimmed.to_string());
            continue;
        }

        buffer.push(trimmed.to_string());
    }

    if !buffer.is_empty() {
        paragraphs.push(buffer.join(" "));
    }

    paragraphs.join("\n\n")
}

/// Detects whether a line is structural Markdown that should remain on its own.
fn is_structural_line(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    line.starts_with('#')
        || line.starts_with('-')
        || line.starts_with('*')
        || line.starts_with('>')
        || line.starts_with("```")
        || line.starts_with('~')
        || line.starts_with("| ")
        || line.starts_with("+-")
        || lowered.chars().next().is_some_and(|c| c.is_ascii_digit()) && lowered.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_good_enough_short_text() {
        assert!(!looks_good_enough("too short"));
    }

    #[test]
    fn test_looks_good_enough_ascii_text() {
        let text = "This is a reasonably long ASCII text that should pass the heuristic. \
        It contains multiple sentences and a decent amount of letters to satisfy the threshold.";
        assert!(looks_good_enough(text));
    }

    #[test]
    fn test_reflow_markdown_preserves_lists() {
        let input = "Item one\nItem two\n\n- Bullet\n- Another";
        let output = reflow_markdown(input);
        assert!(output.contains("Item one Item two"));
        assert!(output.contains("- Bullet"));
    }
}
