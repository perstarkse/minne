fn main() {
    // Get the build profile ("debug" or "release")
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());

    // Embed templates only for release builds
    if profile == "release" {
        // Embed templates from the "templates" directory relative to CARGO_MANIFEST_DIR
        minijinja_embed::embed_templates!("templates");
    } else {
        println!("cargo:info=Build: Skipping template embedding for debug build.");
    }
}
