use common::{
    error::AppError,
    storage::{
        db::SurrealDbClient,
        types::{
            ingestion_payload::IngestionPayload,
            text_content::{TextContent, UrlInfo},
        },
    },
    utils::config::AppConfig,
};

use crate::utils::{
    file_text_extraction::extract_text_from_file, url_text_retrieval::extract_text_from_url,
};

pub(crate) async fn to_text_content(
    ingestion_payload: IngestionPayload,
    db: &SurrealDbClient,
    config: &AppConfig,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
) -> Result<TextContent, AppError> {
    match ingestion_payload {
        IngestionPayload::Url {
            url,
            context,
            category,
            user_id,
        } => {
            let (article, file_info) = extract_text_from_url(&url, db, &user_id, config).await?;
            Ok(TextContent::new(
                article.text_content.into(),
                Some(context),
                category,
                None,
                Some(UrlInfo {
                    url,
                    title: article.title,
                    image_id: file_info.id,
                }),
                user_id,
            ))
        }
        IngestionPayload::Text {
            text,
            context,
            category,
            user_id,
        } => Ok(TextContent::new(
            text,
            Some(context),
            category,
            None,
            None,
            user_id,
        )),
        IngestionPayload::File {
            file_info,
            context,
            category,
            user_id,
        } => {
            let text = extract_text_from_file(&file_info, db, openai_client, config).await?;
            Ok(TextContent::new(
                text,
                Some(context),
                category,
                Some(file_info),
                None,
                user_id,
            ))
        }
    }
}
