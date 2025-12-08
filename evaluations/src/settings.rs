//! System settings enforcement for evaluations.

use anyhow::{Context, Result};
use common::{
    error::AppError,
    storage::{db::SurrealDbClient, types::system_settings::SystemSettings},
};
use tracing::info;

use crate::args::Config;

/// Enforce evaluation-specific system settings overrides.
pub(crate) async fn enforce_system_settings(
    db: &SurrealDbClient,
    mut settings: SystemSettings,
    provider_dimension: usize,
    config: &Config,
) -> Result<SystemSettings> {
    let mut updated_settings = settings.clone();
    let mut needs_settings_update = false;

    if provider_dimension != settings.embedding_dimensions as usize {
        updated_settings.embedding_dimensions = provider_dimension as u32;
        needs_settings_update = true;
    }
    if let Some(query_override) = config.query_model.as_deref() {
        if settings.query_model != query_override {
            info!(
                model = query_override,
                "Overriding system query model for this run"
            );
            updated_settings.query_model = query_override.to_string();
            needs_settings_update = true;
        }
    }
    if needs_settings_update {
        settings = SystemSettings::update(db, updated_settings)
            .await
            .context("updating system settings overrides")?;
    }
    Ok(settings)
}

/// Load existing system settings or initialize them via migrations.
pub(crate) async fn load_or_init_system_settings(
    db: &SurrealDbClient,
    _dimension: usize,
) -> Result<(SystemSettings, bool)> {
    match SystemSettings::get_current(db).await {
        Ok(settings) => Ok((settings, false)),
        Err(AppError::NotFound(_)) => {
            info!("System settings missing; applying database migrations for namespace");
            db.apply_migrations()
                .await
                .context("applying database migrations after missing system settings")?;
            let settings = SystemSettings::get_current(db)
                .await
                .context("loading system settings after migrations")?;
            Ok((settings, true))
        }
        Err(err) => Err(err).context("loading system settings"),
    }
}
