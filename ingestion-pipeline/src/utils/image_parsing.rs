use async_openai::types::{
    ChatCompletionRequestMessageContentPartImageArgs,
    ChatCompletionRequestMessageContentPartTextArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs, ImageDetail, ImageUrlArgs,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
};

pub async fn extract_text_from_image(
    path: &str,
    db: &SurrealDbClient,
    client: &async_openai::Client<async_openai::config::OpenAIConfig>,
) -> Result<String, AppError> {
    let system_settings = SystemSettings::get_current(db).await?;
    let image_bytes = tokio::fs::read(&path).await?;

    let base64_image = STANDARD.encode(&image_bytes);

    let image_url = format!("data:image/png;base64,{}", base64_image);

    let request = CreateChatCompletionRequestArgs::default()
        .model(system_settings.image_processing_model)
        .messages([ChatCompletionRequestUserMessageArgs::default()
            .content(vec![
                ChatCompletionRequestMessageContentPartTextArgs::default()
                    .text(system_settings.image_processing_prompt)
                    .build()?
                    .into(),
                ChatCompletionRequestMessageContentPartImageArgs::default()
                    .image_url(
                        ImageUrlArgs::default()
                            .url(image_url)
                            .detail(ImageDetail::High)
                            .build()?,
                    )
                    .build()?
                    .into(),
            ])
            .build()?
            .into()])
        .build()?;

    let response = client.chat().create(request).await?;

    let description = response
        .choices
        .first()
        .and_then(|c| c.message.content.as_ref())
        .cloned()
        .unwrap_or_else(|| "No description found.".to_string());

    Ok(description)
}
