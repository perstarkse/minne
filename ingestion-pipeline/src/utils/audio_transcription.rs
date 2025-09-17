use async_openai::types::{AudioResponseFormat, CreateTranscriptionRequestArgs};
use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
};

/// Transcribes an audio file using the configured OpenAI Whisper model.
pub async fn transcribe_audio_file(
    file_path: &str,
    db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
) -> Result<String, AppError> {
    let system_settings = SystemSettings::get_current(db_client).await?;
    let model = system_settings.voice_processing_model;

    let request = CreateTranscriptionRequestArgs::default()
        .file(file_path)
        .model(model)
        .response_format(AudioResponseFormat::Json)
        .build()?;

    let response = openai_client
        .audio()
        .transcribe(request)
        .await
        .map_err(|e| AppError::Processing(format!("Audio transcription failed: {}", e)))?;
    Ok(response.text)
}
