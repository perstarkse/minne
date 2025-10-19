use common::{
    error::AppError,
    storage::{db::SurrealDbClient, store, types::file_info::FileInfo},
    utils::config::AppConfig,
};

use super::{
    audio_transcription::transcribe_audio_file, image_parsing::extract_text_from_image,
    pdf_ingestion::extract_pdf_content,
};

pub async fn extract_text_from_file(
    file_info: &FileInfo,
    db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    config: &AppConfig,
) -> Result<String, AppError> {
    let base_path = store::resolve_base_dir(config);
    let absolute_path = base_path.join(&file_info.path);

    match file_info.mime_type.as_str() {
        "text/plain" | "text/markdown" | "application/octet-stream" | "text/x-rust" => {
            let content = tokio::fs::read_to_string(&absolute_path).await?;
            Ok(content)
        }
        "application/pdf" => {
            extract_pdf_content(
                &absolute_path,
                db_client,
                openai_client,
                &config.pdf_ingest_mode,
            )
            .await
        }
        "image/png" | "image/jpeg" => {
            let path_str = absolute_path
                .to_str()
                .ok_or_else(|| {
                    AppError::Processing(format!(
                        "Encountered a non-UTF8 path while reading image {}",
                        file_info.id
                    ))
                })?
                .to_string();
            let content = extract_text_from_image(&path_str, db_client, openai_client).await?;
            Ok(content)
        }
        "audio/mpeg" | "audio/mp3" | "audio/wav" | "audio/x-wav" | "audio/webm" | "audio/mp4"
        | "audio/ogg" | "audio/flac" => {
            let path_str = absolute_path
                .to_str()
                .ok_or_else(|| {
                    AppError::Processing(format!(
                        "Encountered a non-UTF8 path while reading audio {}",
                        file_info.id
                    ))
                })?
                .to_string();
            transcribe_audio_file(&path_str, db_client, openai_client).await
        }
        _ => Err(AppError::NotFound(file_info.mime_type.clone())),
    }
}
