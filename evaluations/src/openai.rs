use std::sync::Arc;

use anyhow::{Context, Result};
use async_openai::{Client, config::OpenAIConfig};

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

pub fn ingestion_openai_client(
    include_entities: bool,
) -> Result<(Arc<Client<OpenAIConfig>>, Option<String>)> {
    if include_entities {
        let (client, base_url) = build_client_from_env().context(
            "OPENAI_API_KEY must be set when --include-entities is enabled (entity extraction uses OpenAI)",
        )?;
        Ok((Arc::new(client), Some(base_url)))
    } else {
        Ok((Arc::new(Client::with_config(OpenAIConfig::default())), None))
    }
}

fn build_client_from_env() -> Result<(Client<OpenAIConfig>, String)> {
    let api_key = std::env::var("OPENAI_API_KEY")
        .context("OPENAI_API_KEY must be set to run retrieval evaluations")?;
    let base_url =
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());

    let config = OpenAIConfig::new()
        .with_api_key(api_key)
        .with_api_base(&base_url);
    Ok((Client::with_config(config), base_url))
}
