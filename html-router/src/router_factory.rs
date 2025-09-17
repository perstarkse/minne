use axum::{
    extract::FromRef,
    middleware::{from_fn_with_state, map_response_with_state},
    Router,
};
use axum_session::SessionLayer;
use axum_session_auth::{AuthConfig, AuthSessionLayer};
use axum_session_surreal::SessionSurrealPool;
use common::storage::types::user::User;
use surrealdb::{engine::any::Any, Surreal};

use crate::{
    html_state::HtmlState,
    middlewares::{
        analytics_middleware::analytics_middleware, auth_middleware::require_auth,
        compression::compression_layer, response_middleware::with_template_response,
    },
};

#[macro_export]
macro_rules! create_asset_service {
    // Takes the relative path to the asset directory
    ($relative_path:expr) => {{
        #[cfg(debug_assertions)]
        {
            let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let assets_path = crate_dir.join($relative_path);
            tracing::debug!("Assets: Serving from filesystem: {:?}", assets_path);
            tower_http::services::ServeDir::new(assets_path)
        }
        #[cfg(not(debug_assertions))]
        {
            tracing::debug!("Assets: Serving embedded directory");
            static ASSETS_DIR: include_dir::Dir<'static> =
                include_dir::include_dir!("$CARGO_MANIFEST_DIR/assets");
            tower_serve_static::ServeDir::new(&ASSETS_DIR)
        }
    }};
}

pub type MiddleWareVecType<S> = Vec<Box<dyn FnOnce(Router<S>) -> Router<S> + Send>>;

pub struct RouterFactory<S> {
    app_state: HtmlState,
    public_routers: Vec<Router<S>>,
    protected_routers: Vec<Router<S>>,
    nested_routes: Vec<(String, Router<S>)>,
    nested_protected_routes: Vec<(String, Router<S>)>,
    custom_middleware: MiddleWareVecType<S>,
    public_assets_config: Option<AssetsConfig>,
    compression_enabled: bool,
}

struct AssetsConfig {
    path: String,      // URL path for assets
    directory: String, // Directory on disk
}

impl<S> RouterFactory<S>
where
    S: Clone + Send + Sync + 'static,
    HtmlState: FromRef<S>,
{
    pub fn new(app_state: &HtmlState) -> Self {
        Self {
            app_state: app_state.to_owned(),
            public_routers: Vec::new(),
            protected_routers: Vec::new(),
            nested_routes: Vec::new(),
            nested_protected_routes: Vec::new(),
            custom_middleware: Vec::new(),
            public_assets_config: None,
            compression_enabled: false,
        }
    }

    // Add a serving of assets
    pub fn with_public_assets(mut self, path: &str, directory: &str) -> Self {
        self.public_assets_config = Some(AssetsConfig {
            path: path.to_string(),
            directory: directory.to_string(),
        });
        self
    }

    // Add a public router that will be merged at the root level
    pub fn add_public_routes(mut self, routes: Router<S>) -> Self {
        self.public_routers.push(routes);
        self
    }

    // Add a protected router that will be merged at the root level
    pub fn add_protected_routes(mut self, routes: Router<S>) -> Self {
        self.protected_routers.push(routes);
        self
    }

    // Nest a public router under a path prefix
    pub fn nest_public_routes(mut self, path: &str, routes: Router<S>) -> Self {
        self.nested_routes.push((path.to_string(), routes));
        self
    }

    // Nest a protected router under a path prefix
    pub fn nest_protected_routes(mut self, path: &str, routes: Router<S>) -> Self {
        self.nested_protected_routes
            .push((path.to_string(), routes));
        self
    }

    // Add custom middleware to be applied before the standard ones
    pub fn with_middleware<F>(mut self, middleware_fn: F) -> Self
    where
        F: FnOnce(Router<S>) -> Router<S> + Send + 'static,
    {
        self.custom_middleware.push(Box::new(middleware_fn));
        self
    }

    /// Enables response compression when building the router.
    pub fn with_compression(mut self) -> Self {
        self.compression_enabled = true;
        self
    }

    pub fn build(self) -> Router<S> {
        // Start with an empty router
        let mut public_router = Router::new();

        // Merge all public routers
        for router in self.public_routers {
            public_router = public_router.merge(router);
        }

        // Add nested public routes
        for (path, router) in self.nested_routes {
            public_router = public_router.nest(&path, router);
        }

        // Add public assets to public router
        if let Some(assets_config) = self.public_assets_config {
            // Call the macro using the stored relative directory path
            let asset_service = create_asset_service!(&assets_config.directory);
            // Nest the resulting service under the stored URL path
            public_router = public_router.nest_service(&assets_config.path, asset_service);
        }

        // Start with an empty protected router
        let mut protected_router = Router::new();

        // Check if there are any protected routers
        let has_protected_routes =
            !self.protected_routers.is_empty() || !self.nested_protected_routes.is_empty();

        // Merge root-level protected routers
        for router in self.protected_routers {
            protected_router = protected_router.merge(router);
        }

        // Nest protected routers
        for (path, router) in self.nested_protected_routes {
            protected_router = protected_router.nest(&path, router);
        }

        // Apply auth middleware
        if has_protected_routes {
            protected_router = protected_router
                .route_layer(from_fn_with_state(self.app_state.clone(), require_auth));
        }

        // Combine public and protected routes
        let mut router = Router::new().merge(public_router).merge(protected_router);

        // Apply custom middleware in order they were added
        for middleware_fn in self.custom_middleware {
            router = middleware_fn(router);
        }

        // Apply common middleware
        router = router.layer(from_fn_with_state(
            self.app_state.clone(),
            analytics_middleware::<HtmlState>,
        ));
        router = router.layer(map_response_with_state(
            self.app_state.clone(),
            with_template_response::<HtmlState>,
        ));
        router = router.layer(
            AuthSessionLayer::<User, String, SessionSurrealPool<Any>, Surreal<Any>>::new(Some(
                self.app_state.db.client.clone(),
            ))
            .with_config(AuthConfig::<String>::default()),
        );
        router = router.layer(SessionLayer::new((*self.app_state.session_store).clone()));

        if self.compression_enabled {
            router = router.layer(compression_layer());
        }

        router
    }
}
