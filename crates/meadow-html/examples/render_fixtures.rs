//! Renders a documentation site from the repository's Ruby fixtures.
//!
//! Usage: `cargo run -p meadow-html --example render_fixtures -- [OUTPUT_DIR]`

use std::fs;
use std::path::{Path, PathBuf};

use meadow_handlers::build_registry;
use meadow_html::{SiteOptions, render_site};
use meadow_parser::{FileFacts, parse_source};

const FIXTURES: &[&str] = &[
    "core.rb",
    "tags.rb",
    "handlers.rb",
    "directives.rb",
    "links.rb",
    "docstrings.rb",
    "parameters.rb",
    "calls.rb",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::env::args_os()
        .nth(1)
        .map_or_else(|| root.join("target/meadow-example-site"), PathBuf::from);

    let facts: Vec<FileFacts> = FIXTURES
        .iter()
        .map(|name| {
            let relative = format!("tests/fixtures/{name}");
            let source = fs::read_to_string(root.join(&relative))?;
            Ok(parse_source(relative, &source))
        })
        .collect::<Result<_, std::io::Error>>()?;

    let registry = build_registry(&facts);
    let stats = render_site(
        &registry,
        &SiteOptions {
            output_dir: output.clone(),
            title: "Meadow Example".to_string(),
            readme: Some(PathBuf::from("README.md")),
            project_root: root,
            ..SiteOptions::default()
        },
    )?;

    println!(
        "rendered {} pages and {} search entries to {}",
        stats.pages,
        stats.search_entries,
        output.display()
    );
    Ok(())
}
