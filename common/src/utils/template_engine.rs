pub use minijinja::{path_loader, Environment, Value};
pub use minijinja_autoreload::AutoReloader;
pub use minijinja_contrib;
pub use minijinja_embed;
use std::sync::Arc;

#[allow(clippy::module_name_repetitions)]
pub trait ProvidesTemplateEngine {
    fn template_engine(&self) -> &Arc<TemplateEngine>;
}

#[derive(Clone)]
pub enum TemplateEngine {
    // Use AutoReload for debug builds (debug_assertions is true)
    #[cfg(debug_assertions)]
    AutoReload(Arc<AutoReloader>),
    // Use Embedded for release builds (debug_assertions is false)
    #[cfg(not(debug_assertions))]
    Embedded(Arc<Environment<'static>>),
}

#[macro_export]
macro_rules! create_template_engine {
    // Single path argument
    ($relative_path:expr) => {
        $crate::create_template_engine!($relative_path, Option::<&str>::None)
    };

    // Path + Fallback argument
    ($relative_path:expr, $fallback_path:expr) => {{
        // Code for debug builds (AutoReload)
        #[cfg(debug_assertions)]
        {
            // These lines execute in the CALLING crate's context
            let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let template_path = crate_dir.join($relative_path);
            let fallback_path = $fallback_path.map(|p| crate_dir.join(p));

            let reloader = $crate::utils::template_engine::AutoReloader::new(move |notifier| {
                let mut env = $crate::utils::template_engine::Environment::new();

                let loader_primary = $crate::utils::template_engine::path_loader(&template_path);

                // Clone fallback_path for the closure
                let fallback = fallback_path.clone();

                env.set_loader(move |name| match loader_primary(name) {
                    Ok(Some(tmpl)) => Ok(Some(tmpl)),
                    Ok(None) => {
                        if let Some(ref fb_path) = fallback {
                            let loader_fallback =
                                $crate::utils::template_engine::path_loader(fb_path);
                            loader_fallback(name)
                        } else {
                            Ok(None)
                        }
                    }
                    Err(e) => Err(e),
                });

                notifier.set_fast_reload(true);
                notifier.watch_path(&template_path, true);
                if let Some(ref fb) = fallback_path {
                    notifier.watch_path(fb, true);
                }

                // Add contrib filters/functions
                $crate::utils::template_engine::minijinja_contrib::add_to_environment(&mut env);
                Ok(env)
            });
            $crate::utils::template_engine::TemplateEngine::AutoReload(std::sync::Arc::new(
                reloader,
            ))
        }
        // Code for release builds (Embedded)
        #[cfg(not(debug_assertions))]
        {
            // These lines also execute in the CALLING crate's context
            let mut env = $crate::utils::template_engine::Environment::new();
            $crate::utils::template_engine::minijinja_embed::load_templates!(&mut env);
            // Add contrib filters/functions
            $crate::utils::template_engine::minijinja_contrib::add_to_environment(&mut env);
            $crate::utils::template_engine::TemplateEngine::Embedded(std::sync::Arc::new(env))
        }
    }};
}

impl TemplateEngine {
    pub fn render(&self, name: &str, ctx: &Value) -> Result<String, minijinja::Error> {
        match self {
            // Only compile this arm for debug builds
            #[cfg(debug_assertions)]
            Self::AutoReload(reloader) => {
                let env = reloader.acquire_env()?;
                env.get_template(name)?.render(ctx)
            }
            // Only compile this arm for release builds
            #[cfg(not(debug_assertions))]
            Self::Embedded(env) => env.get_template(name)?.render(ctx),
        }
    }

    pub fn render_block(
        &self,
        template_name: &str,
        block_name: &str,
        context: &Value,
    ) -> Result<String, minijinja::Error> {
        match self {
            // Only compile this arm for debug builds
            #[cfg(debug_assertions)]
            Self::AutoReload(reloader) => reloader
                .acquire_env()?
                .get_template(template_name)?
                .eval_to_state(context)?
                .render_block(block_name),
            // Only compile this arm for release builds
            #[cfg(not(debug_assertions))]
            Self::Embedded(env) => env
                .get_template(template_name)?
                .eval_to_state(context)?
                .render_block(block_name),
        }
    }
}
