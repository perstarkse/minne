use tower_http::compression::CompressionLayer;

/// Provides a default compression layer that negotiates encoding based on the
/// `Accept-Encoding` header of the incoming request.
pub fn layer() -> CompressionLayer {
    CompressionLayer::new()
}
