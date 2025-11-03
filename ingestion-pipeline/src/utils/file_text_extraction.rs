use anyhow::anyhow;
use common::{
    error::AppError,
    storage::{db::SurrealDbClient, store::StorageManager, types::file_info::FileInfo},
    utils::config::AppConfig,
};
use std::{
    env,
    io::{Error as IoError, ErrorKind},
    path::{Path, PathBuf},
};
use uuid::Uuid;

use super::{
    audio_transcription::transcribe_audio_file, image_parsing::extract_text_from_image,
    pdf_ingestion::extract_pdf_content,
};

struct TempPathGuard {
    path: PathBuf,
}

impl TempPathGuard {
    fn as_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempPathGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn materialize_temp_file(
    bytes: &[u8],
    extension: Option<&str>,
) -> Result<TempPathGuard, AppError> {
    let mut path = env::temp_dir();
    let mut file_name = format!("minne-ingest-{}", Uuid::new_v4());

    if let Some(ext) = extension {
        if !ext.is_empty() {
            file_name.push('.');
            file_name.push_str(ext);
        }
    }

    path.push(file_name);

    tokio::fs::write(&path, bytes).await?;

    Ok(TempPathGuard { path })
}

async fn resolve_existing_local_path(storage: &StorageManager, location: &str) -> Option<PathBuf> {
    let path = storage.resolve_local_path(location)?;
    match tokio::fs::metadata(&path).await {
        Ok(_) => Some(path),
        Err(_) => None,
    }
}

fn infer_extension(file_info: &FileInfo) -> Option<String> {
    Path::new(&file_info.path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_string())
}

pub async fn extract_text_from_file(
    file_info: &FileInfo,
    db_client: &SurrealDbClient,
    openai_client: &async_openai::Client<async_openai::config::OpenAIConfig>,
    config: &AppConfig,
    storage: &StorageManager,
) -> Result<String, AppError> {
    let file_bytes = storage
        .get(&file_info.path)
        .await
        .map_err(|e| AppError::from(anyhow!(e)))?;
    let local_path = resolve_existing_local_path(storage, &file_info.path).await;

    match file_info.mime_type.as_str() {
        "text/plain" | "text/markdown" | "application/octet-stream" | "text/x-rust" => {
            let content = String::from_utf8(file_bytes.to_vec())
                .map_err(|err| AppError::Io(IoError::new(ErrorKind::InvalidData, err)))?;
            Ok(content)
        }
        "application/pdf" => {
            if let Some(path) = local_path.as_ref() {
                return extract_pdf_content(
                    path,
                    db_client,
                    openai_client,
                    &config.pdf_ingest_mode,
                )
                .await;
            }

            let temp_guard = materialize_temp_file(file_bytes.as_ref(), Some("pdf")).await?;
            let result = extract_pdf_content(
                temp_guard.as_path(),
                db_client,
                openai_client,
                &config.pdf_ingest_mode,
            )
            .await;
            drop(temp_guard);
            result
        }
        "image/png" | "image/jpeg" => {
            let content =
                extract_text_from_image(file_bytes.as_ref(), db_client, openai_client).await?;
            Ok(content)
        }
        "audio/mpeg" | "audio/mp3" | "audio/wav" | "audio/x-wav" | "audio/webm" | "audio/mp4"
        | "audio/ogg" | "audio/flac" => {
            if let Some(path) = local_path.as_ref() {
                let path_str = path.to_str().ok_or_else(|| {
                    AppError::Processing(format!(
                        "Encountered a non-UTF8 path while reading audio {}",
                        file_info.id
                    ))
                })?;
                return transcribe_audio_file(path_str, db_client, openai_client).await;
            }

            let extension = infer_extension(file_info);
            let temp_guard =
                materialize_temp_file(file_bytes.as_ref(), extension.as_deref()).await?;
            let path_str = temp_guard.as_path().to_str().ok_or_else(|| {
                AppError::Processing(format!(
                    "Encountered a non-UTF8 path while reading audio {}",
                    file_info.id
                ))
            })?;
            let result = transcribe_audio_file(path_str, db_client, openai_client).await;
            drop(temp_guard);
            result
        }
        _ => Err(AppError::NotFound(file_info.mime_type.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_openai::{config::OpenAIConfig, Client};
    use bytes::Bytes;
    use chrono::Utc;
    use common::{
        storage::{db::SurrealDbClient, store::StorageManager},
        utils::config::{AppConfig, StorageKind},
    };

    #[tokio::test]
    async fn extracts_text_using_memory_storage_backend() {
        let mut config = AppConfig::default();
        config.storage = StorageKind::Memory;

        let storage = StorageManager::new(&config)
            .await
            .expect("create storage manager");

        let location = "user/test/file.txt";
        let contents = b"hello from memory storage";

        storage
            .put(location, Bytes::from(contents.as_slice().to_vec()))
            .await
            .expect("write object");

        let now = Utc::now();
        let file_info = FileInfo {
            id: "file".into(),
            created_at: now,
            updated_at: now,
            sha256: "sha256".into(),
            path: location.to_string(),
            file_name: "file.txt".into(),
            mime_type: "text/plain".into(),
            user_id: "user".into(),
        };

        let namespace = "test_ns";
        let database = &Uuid::new_v4().to_string();
        let db = SurrealDbClient::memory(namespace, database)
            .await
            .expect("create surreal memory");

        let openai_client = Client::with_config(OpenAIConfig::default());

        let text = extract_text_from_file(&file_info, &db, &openai_client, &config, &storage)
            .await
            .expect("extract text");

        assert_eq!(text, String::from_utf8_lossy(contents));
    }
}
