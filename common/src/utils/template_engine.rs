pub use minijinja::{path_loader, Environment, Value};
pub use minijinja_autoreload::AutoReloader;
pub use minijinja_contrib;
pub use minijinja_embed;
use std::sync::Arc;

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
    // Macro takes the relative path to the templates dir as input
    ($relative_path:expr) => {{
        // Code for debug builds (AutoReload)
        #[cfg(debug_assertions)]
        {
            // These lines execute in the CALLING crate's context
            let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let template_path = crate_dir.join($relative_path);
            let reloader = $crate::utils::template_engine::AutoReloader::new(move |notifier| {
                let mut env = $crate::utils::template_engine::Environment::new();
                env.set_loader($crate::utils::template_engine::path_loader(&template_path));
                notifier.set_fast_reload(true);
                notifier.watch_path(&template_path, true);
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
                .render_block(block.name),
        }
    }
}
