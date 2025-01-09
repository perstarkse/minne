use config::{Config, ConfigError, File};

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_relayer: String,
    pub surrealdb_address: String,
    pub surrealdb_username: String,
    pub surrealdb_password: String,
    pub surrealdb_namespace: String,
    pub surrealdb_database: String,
}

pub fn get_config() -> Result<AppConfig, ConfigError> {
    let config = Config::builder()
        .add_source(File::with_name("config"))
        .build()?;

    Ok(AppConfig {
        smtp_username: config.get_string("SMTP_USERNAME")?,
        smtp_password: config.get_string("SMTP_PASSWORD")?,
        smtp_relayer: config.get_string("SMTP_RELAYER")?,
        surrealdb_address: config.get_string("SURREALDB_ADDRESS")?,
        surrealdb_username: config.get_string("SURREALDB_USERNAME")?,
        surrealdb_password: config.get_string("SURREALDB_PASSWORD")?,
        surrealdb_namespace: config.get_string("SURREALDB_NAMESPACE")?,
        surrealdb_database: config.get_string("SURREALDB_DATABASE")?,
    })
}
