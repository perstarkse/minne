use axum::response::{IntoResponse, Response};
use axum_typed_multipart::TypedMultipart;

use crate::models::files::FileUploadRequest;

// async fn upload_asset(
//     TypedMultipart(UploadAssetRequest { image, author }): TypedMultipart<UploadAssetRequest>,
// ) -> StatusCode {
//     let file_name = image.metadata.file_name.unwrap_or(String::from("data.bin"));
//     let path = Path::new("/tmp").join(author).join(file_name);

//     match image.contents.persist(path) {
//         Ok(_) => StatusCode::CREATED,
//         Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
//     }
// }
pub async fn upload_handler(TypedMultipart(input): TypedMultipart<FileUploadRequest>) -> Response {
    // let file_name = input.file.metadata.file_name.unwrap_or("newstring".to_string());
    // 

    "Successfully processed".to_string().into_response()
} 
