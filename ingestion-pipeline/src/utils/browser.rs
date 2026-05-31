use common::error::AppError;
use headless_chrome::Browser;

/// Launches a headless Chrome instance, honoring the `docker` feature flag
/// (which disables the Chrome sandbox for container environments).
///
/// This is the single place the crate spawns a browser. If the rendering backend
/// is ever swapped away from headless Chrome to something leaner, this function is
/// the seam to change; callers only depend on getting back a `Browser`.
pub(crate) fn launch_browser() -> Result<Browser, AppError> {
    #[cfg(feature = "docker")]
    {
        let options = headless_chrome::LaunchOptionsBuilder::default()
            .sandbox(false)
            .build()
            .map_err(|err| {
                AppError::Processing(format!("Failed to build headless browser options: {err}"))
            })?;
        Browser::new(options)
            .map_err(|err| AppError::Processing(format!("Failed to start headless browser: {err}")))
    }
    #[cfg(not(feature = "docker"))]
    {
        Browser::default()
            .map_err(|err| AppError::Processing(format!("Failed to start headless browser: {err}")))
    }
}
