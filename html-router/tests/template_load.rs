//! Compile-time smoke test for every file under `templates/`.
//!
//! Loads each `.html` through minijinja with the same `path_loader` setup as the
//! app. Catches syntax and extends/include errors without rendering or hitting routes.
//! Complements insta snapshots in `router_integration.rs`, which test rendered HTML.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use minijinja::{path_loader, Environment};

fn templates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates")
}

fn collect_html_templates(dir: &Path, root: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read template directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_html_templates(&path, root, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("html") {
            let rel = path.strip_prefix(root).expect("strip templates root");
            // minijinja template names use forward slashes regardless of OS.
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

#[test]
fn all_templates_compile() {
    let root = templates_dir();

    let mut env = Environment::new();
    env.set_loader(path_loader(&root));
    minijinja_contrib::add_to_environment(&mut env);

    let mut names = Vec::new();
    collect_html_templates(&root, &root, &mut names);
    assert!(
        !names.is_empty(),
        "expected to discover template files under {}",
        root.display()
    );

    let mut failures = Vec::new();
    for name in &names {
        if let Err(error) = env.get_template(name) {
            failures.push(format!("{name}: {error:#}"));
        }
    }

    assert!(
        failures.is_empty(),
        "{} template(s) failed to compile:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
