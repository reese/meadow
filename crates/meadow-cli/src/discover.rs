//! Source file discovery.

use std::path::{Path, PathBuf};

use glob::{Pattern, glob};

use crate::config::RunOptions;

/// Discovers Ruby source files from the configured inputs and exclusions.
#[must_use]
pub fn discover(options: &RunOptions) -> Vec<PathBuf> {
    let patterns = if options.inputs.is_empty() {
        vec![
            "lib/**/*.rb".to_string(),
            "lib/**/*.rbs".to_string(),
            "app/**/*.rb".to_string(),
            "app/**/*.rbs".to_string(),
            "sig/**/*.rbs".to_string(),
            "ext/**/*.rb".to_string(),
        ]
    } else {
        options.inputs.clone()
    };
    let excludes: Vec<Pattern> = options
        .exclude
        .iter()
        .filter_map(|pattern| Pattern::new(pattern).ok())
        .collect();

    let mut files = Vec::new();
    for pattern in patterns {
        if pattern.contains(['*', '?', '[']) {
            let Ok(entries) = glob(&pattern) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.is_file() {
                    files.push(entry);
                }
            }
        } else {
            let path = Path::new(&pattern);
            if path.is_file() {
                files.push(path.to_path_buf());
            } else if path.is_dir() {
                for extension in ["rb", "rbs"] {
                    let nested = format!("{}/**/*.{extension}", path.display());
                    if let Ok(entries) = glob(&nested) {
                        for entry in entries.flatten() {
                            if entry.is_file() {
                                files.push(entry);
                            }
                        }
                    }
                }
            }
        }
    }

    files.retain(|path| {
        let display = path.to_string_lossy();
        !excludes.iter().any(|pattern| pattern.matches(&display))
            && !display.starts_with(options.db.to_string_lossy().as_ref())
            && !display.starts_with(options.output_dir.to_string_lossy().as_ref())
    });
    files.sort();
    files.dedup();
    files
}
