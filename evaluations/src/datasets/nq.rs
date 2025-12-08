use std::{
    collections::BTreeSet,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::warn;

use super::{ConvertedParagraph, ConvertedQuestion};

pub fn convert_nq(
    raw_path: &Path,
    include_unanswerable: bool,
    _context_token_limit: Option<usize>,
) -> Result<Vec<ConvertedParagraph>> {
    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct NqExample {
        question_text: String,
        document_title: String,
        example_id: i64,
        document_tokens: Vec<NqToken>,
        long_answer_candidates: Vec<NqLongAnswerCandidate>,
        annotations: Vec<NqAnnotation>,
    }

    #[derive(Debug, Deserialize)]
    struct NqToken {
        token: String,
        #[serde(default)]
        html_token: bool,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct NqLongAnswerCandidate {
        start_token: i32,
        end_token: i32,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct NqAnnotation {
        short_answers: Vec<NqShortAnswer>,
        #[serde(default)]
        yes_no_answer: String,
        long_answer: NqLongAnswer,
    }

    #[derive(Debug, Deserialize)]
    struct NqShortAnswer {
        start_token: i32,
        end_token: i32,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct NqLongAnswer {
        candidate_index: i32,
    }

    fn join_tokens(tokens: &[NqToken], start: usize, end: usize) -> String {
        let mut buffer = String::new();
        let end = end.min(tokens.len());
        for token in tokens.iter().skip(start).take(end.saturating_sub(start)) {
            if token.html_token {
                continue;
            }
            let text = token.token.trim();
            if text.is_empty() {
                continue;
            }
            let attach = matches!(
                text,
                "," | "." | "!" | "?" | ";" | ":" | ")" | "]" | "}" | "%" | "…" | "..."
            ) || text.starts_with('\'')
                || text == "n't"
                || text == "'s"
                || text == "'re"
                || text == "'ve"
                || text == "'d"
                || text == "'ll";

            if buffer.is_empty() || attach {
                buffer.push_str(text);
            } else {
                buffer.push(' ');
                buffer.push_str(text);
            }
        }

        buffer.trim().to_string()
    }

    let file = File::open(raw_path).with_context(|| {
        format!(
            "opening Natural Questions dataset at {}",
            raw_path.display()
        )
    })?;
    let reader = BufReader::new(file);

    let mut paragraphs = Vec::new();
    for (line_idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!(
                "reading Natural Questions line {} from {}",
                line_idx + 1,
                raw_path.display()
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let example: NqExample = serde_json::from_str(&line).with_context(|| {
            format!(
                "parsing Natural Questions JSON (line {}) at {}",
                line_idx + 1,
                raw_path.display()
            )
        })?;

        let mut answer_texts: Vec<String> = Vec::new();
        let mut short_answer_texts: Vec<String> = Vec::new();
        let mut has_short_or_yesno = false;
        let mut has_short_answer = false;
        for annotation in &example.annotations {
            for short in &annotation.short_answers {
                if short.start_token < 0 || short.end_token <= short.start_token {
                    continue;
                }
                let start = short.start_token as usize;
                let end = short.end_token as usize;
                if start >= example.document_tokens.len() || end > example.document_tokens.len() {
                    continue;
                }
                let text = join_tokens(&example.document_tokens, start, end);
                if !text.is_empty() {
                    answer_texts.push(text.clone());
                    short_answer_texts.push(text);
                    has_short_or_yesno = true;
                    has_short_answer = true;
                }
            }

            match annotation
                .yes_no_answer
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "yes" => {
                    answer_texts.push("yes".to_string());
                    has_short_or_yesno = true;
                }
                "no" => {
                    answer_texts.push("no".to_string());
                    has_short_or_yesno = true;
                }
                _ => {}
            }
        }

        let mut answers = dedupe_strings(answer_texts);
        let is_unanswerable = !has_short_or_yesno || answers.is_empty();
        if is_unanswerable {
            if !include_unanswerable {
                continue;
            }
            answers.clear();
        }

        let paragraph_id = format!("nq-{}", example.example_id);
        let question_id = format!("nq-{}", example.example_id);

        let context = join_tokens(&example.document_tokens, 0, example.document_tokens.len());
        if context.is_empty() {
            continue;
        }

        if has_short_answer && !short_answer_texts.is_empty() {
            let normalized_context = context.to_ascii_lowercase();
            let missing_answer = short_answer_texts.iter().any(|answer| {
                let needle = answer.trim().to_ascii_lowercase();
                !needle.is_empty() && !normalized_context.contains(&needle)
            });
            if missing_answer {
                warn!(
                    question_id = %question_id,
                    "Skipping Natural Questions example because answers were not found in the assembled context"
                );
                continue;
            }
        }

        if !include_unanswerable && (!has_short_answer || short_answer_texts.is_empty()) {
            // yes/no-only questions are excluded by default unless --llm-mode is used
            continue;
        }

        let question = ConvertedQuestion {
            id: question_id,
            question: example.question_text.trim().to_string(),
            answers,
            is_impossible: is_unanswerable,
        };

        paragraphs.push(ConvertedParagraph {
            id: paragraph_id,
            title: example.document_title.trim().to_string(),
            context,
            questions: vec![question],
        });
    }

    Ok(paragraphs)
}

fn dedupe_strings<I>(values: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut set = BTreeSet::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            set.insert(trimmed.to_string());
        }
    }
    set.into_iter().collect()
}
