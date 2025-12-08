use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{ConvertedParagraph, ConvertedQuestion};

pub fn convert_squad(raw_path: &Path) -> Result<Vec<ConvertedParagraph>> {
    #[derive(Debug, Deserialize)]
    struct SquadDataset {
        data: Vec<SquadArticle>,
    }

    #[derive(Debug, Deserialize)]
    struct SquadArticle {
        title: String,
        paragraphs: Vec<SquadParagraph>,
    }

    #[derive(Debug, Deserialize)]
    struct SquadParagraph {
        context: String,
        qas: Vec<SquadQuestion>,
    }

    #[derive(Debug, Deserialize)]
    struct SquadQuestion {
        id: String,
        question: String,
        answers: Vec<SquadAnswer>,
        #[serde(default)]
        is_impossible: bool,
    }

    #[derive(Debug, Deserialize)]
    struct SquadAnswer {
        text: String,
    }

    let raw = fs::read_to_string(raw_path)
        .with_context(|| format!("reading raw SQuAD dataset at {}", raw_path.display()))?;
    let parsed: SquadDataset = serde_json::from_str(&raw)
        .with_context(|| format!("parsing SQuAD dataset at {}", raw_path.display()))?;

    let mut paragraphs = Vec::new();
    for (article_idx, article) in parsed.data.into_iter().enumerate() {
        for (paragraph_idx, paragraph) in article.paragraphs.into_iter().enumerate() {
            let mut questions = Vec::new();
            for qa in paragraph.qas {
                let answers = dedupe_strings(qa.answers.into_iter().map(|answer| answer.text));
                questions.push(ConvertedQuestion {
                    id: qa.id,
                    question: qa.question.trim().to_string(),
                    answers,
                    is_impossible: qa.is_impossible,
                });
            }

            let paragraph_id =
                format!("{}-{}", slugify(&article.title, article_idx), paragraph_idx);

            paragraphs.push(ConvertedParagraph {
                id: paragraph_id,
                title: article.title.trim().to_string(),
                context: paragraph.context.trim().to_string(),
                questions,
            });
        }
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

fn slugify(input: &str, fallback_idx: usize) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in input.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }

    slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        slug = format!("article-{fallback_idx}");
    }
    slug
}
