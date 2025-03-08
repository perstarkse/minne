// use axum::{
//     http::StatusCode,
//     response::{Html, IntoResponse, Response},
// };
// use common::error::AppError;
// use minijinja::context;
// use minijinja_autoreload::AutoReloader;
// use std::sync::Arc;

// pub type TemplateResult<T> = Result<T, HtmlError>;

// // Helper trait for converting to HtmlError with templates
// pub trait IntoHtmlError {
//     fn with_template(self, templates: Arc<AutoReloader>) -> HtmlError;
// }
// // // Implement for AppError
// impl IntoHtmlError for AppError {
//     fn with_template(self, templates: Arc<AutoReloader>) -> HtmlError {
//         HtmlError::new(self, templates)
//     }
// }
// // // Implement for minijinja::Error directly
// impl IntoHtmlError for minijinja::Error {
//     fn with_template(self, templates: Arc<AutoReloader>) -> HtmlError {
//         HtmlError::from_template_error(self, templates)
//     }
// }

// pub enum HtmlError {
//     ServerError(Arc<AutoReloader>),
//     NotFound(Arc<AutoReloader>),
//     Unauthorized(Arc<AutoReloader>),
//     BadRequest(String, Arc<AutoReloader>),
//     Template(String, Arc<AutoReloader>),
// }

// impl HtmlError {
//     pub fn new(error: AppError, templates: Arc<AutoReloader>) -> Self {
//         match error {
//             AppError::NotFound(_msg) => HtmlError::NotFound(templates),
//             AppError::Auth(_msg) => HtmlError::Unauthorized(templates),
//             AppError::Validation(msg) => HtmlError::BadRequest(msg, templates),
//             _ => {
//                 tracing::error!("Internal error: {:?}", error);
//                 HtmlError::ServerError(templates)
//             }
//         }
//     }

//     pub fn from_template_error(error: minijinja::Error, templates: Arc<AutoReloader>) -> Self {
//         tracing::error!("Template error: {:?}", error);
//         HtmlError::Template(error.to_string(), templates)
//     }
// }

// impl IntoResponse for HtmlError {
//     fn into_response(self) -> Response {
//         let (status, context, templates) = match self {
//             HtmlError::ServerError(templates) | HtmlError::Template(_, templates) => (
//                 StatusCode::INTERNAL_SERVER_ERROR,
//                 context! {
//                     status_code => 500,
//                     title => "Internal Server Error",
//                     error => "Internal Server Error",
//                     description => "Something went wrong on our end."
//                 },
//                 templates,
//             ),
//             HtmlError::NotFound(templates) => (
//                 StatusCode::NOT_FOUND,
//                 context! {
//                     status_code => 404,
//                     title => "Page Not Found",
//                     error => "Not Found",
//                     description => "The page you're looking for doesn't exist or was removed."
//                 },
//                 templates,
//             ),
//             HtmlError::Unauthorized(templates) => (
//                 StatusCode::UNAUTHORIZED,
//                 context! {
//                     status_code => 401,
//                     title => "Unauthorized",
//                     error => "Access Denied",
//                     description => "You need to be logged in to access this page."
//                 },
//                 templates,
//             ),
//             HtmlError::BadRequest(msg, templates) => (
//                 StatusCode::BAD_REQUEST,
//                 context! {
//                     status_code => 400,
//                     title => "Bad Request",
//                     error => "Bad Request",
//                     description => msg
//                 },
//                 templates,
//             ),
//         };

//         let html = match templates.acquire_env() {
//             Ok(env) => match env.get_template("errors/error.html") {
//                 Ok(tmpl) => match tmpl.render(context) {
//                     Ok(output) => output,
//                     Err(e) => {
//                         tracing::error!("Template render error: {:?}", e);
//                         Self::fallback_html()
//                     }
//                 },
//                 Err(e) => {
//                     tracing::error!("Template get error: {:?}", e);
//                     Self::fallback_html()
//                 }
//             },
//             Err(e) => {
//                 tracing::error!("Environment acquire error: {:?}", e);
//                 Self::fallback_html()
//             }
//         };

//         (status, Html(html)).into_response()
//     }
// }

// impl HtmlError {
//     fn fallback_html() -> String {
//         r#"
//                      <html>
//                          <body>
//                              <div class="container mx-auto p-4">
//                                  <h1 class="text-4xl text-error">Error</h1>
//                                  <p class="mt-4">Sorry, something went wrong displaying this page.</p>
//                              </div>
//                          </body>
//                      </html>
//                      "#
//         .to_string()
//     }
// }
