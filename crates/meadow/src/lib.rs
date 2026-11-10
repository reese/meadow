//! High-level build API for embedding Meadow.
//!
//! The CLI remains responsible for configuration files, discovery globs, and
//! caches. This crate exposes the pieces an application needs to turn a list of
//! source paths into a resolved [`Registry`] without spawning the `meadow`
//! binary or serializing through JSON.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use meadow_handlers::build_registry_full;
use meadow_model::Registry;
use meadow_parser::{
    DiagnosticLevel, FileFacts, ParseConfig, RubyVersion, SourceLocation, SourceText,
    parse_source_bytes_with,
};
use meadow_plugins::builtin_plugins_with;
use rayon::prelude::*;

pub use meadow_docstring::DocstringWarning;
pub use meadow_handlers::BuildOutput;
pub use meadow_model as model;
pub use meadow_parser as parser;
pub use meadow_plugins as plugins;
pub use meadow_plugins::PluginSettings;
pub use meadow_resolve::ResolutionWarning;

/// Options for [`build`].
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Override Prism's Ruby syntax version.
    pub ruby_version: Option<RubyVersion>,
    /// Built-in plugin names, such as `sorbet` or `activerecord`.
    pub plugins: Vec<String>,
    /// Settings passed to the built-in plugins.
    pub plugin_settings: PluginSettings,
    /// Project root used by plugins that resolve files relative to the source
    /// tree.
    pub project_root: PathBuf,
}

/// A parser diagnostic paired with its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiagnostic {
    /// Source file.
    pub file: PathBuf,
    /// Diagnostic severity.
    pub level: DiagnosticLevel,
    /// Diagnostic message.
    pub message: SourceText,
    /// Location in the source, when available.
    pub location: Option<SourceLocation>,
}

/// The result of building a registry from source paths.
#[derive(Debug)]
pub struct Build {
    /// Resolved registry.
    pub registry: Registry,
    /// Docstring warnings emitted while handling source files.
    pub docstring_warnings: Vec<DocstringWarning>,
    /// Parser syntax errors and warnings.
    pub parser_diagnostics: Vec<FileDiagnostic>,
    /// Unresolved-name warnings from resolution.
    pub resolution_warnings: Vec<ResolutionWarning>,
    /// Plugin warnings.
    pub plugin_warnings: Vec<String>,
    /// Number of source files parsed.
    pub files: usize,
}

/// Builds a registry from files and directories using default options.
///
/// Directories are searched recursively for `.rb` and `.rbs` files. Ruby files
/// are handled before RBS files so signatures can augment discovered code.
///
/// # Errors
///
/// Returns an error when a path cannot be read or a directory cannot be
/// traversed.
pub fn build(paths: &[PathBuf]) -> io::Result<Build> {
    build_with(paths, &BuildOptions::default())
}

/// Builds a registry from files and directories with explicit options.
///
/// # Errors
///
/// Returns an error when a path cannot be read or a directory cannot be
/// traversed.
pub fn build_with(paths: &[PathBuf], options: &BuildOptions) -> io::Result<Build> {
    let mut files = Vec::new();
    for path in paths {
        collect_files(&input_path(path, &options.project_root), &mut files)?;
    }
    files.sort_by(|left, right| {
        is_rbs(left)
            .cmp(&is_rbs(right))
            .then_with(|| left.cmp(right))
    });
    files.dedup();

    let sources: Vec<(PathBuf, Vec<u8>)> = files
        .iter()
        .map(|path| Ok((registry_path(path, &options.project_root), fs::read(path)?)))
        .collect::<io::Result<_>>()?;
    let config = ParseConfig {
        ruby_version: options.ruby_version,
    };
    let facts: Vec<FileFacts> = sources
        .par_iter()
        .map(|(path, bytes)| parse_source_bytes_with(path.clone(), bytes, &config))
        .collect();
    let parser_diagnostics = facts
        .iter()
        .flat_map(|facts| {
            let file = facts.path.clone();
            facts
                .diagnostics
                .iter()
                .map(move |diagnostic| FileDiagnostic {
                    file: file.clone(),
                    level: diagnostic.level,
                    message: diagnostic.message.clone(),
                    location: diagnostic.location,
                })
        })
        .collect();

    let plugins = builtin_plugins_with(
        &options.plugins,
        &options.project_root,
        &options.plugin_settings,
    );
    let BuildOutput {
        registry,
        docstring_warnings,
        resolution_warnings,
        plugin_warnings,
    } = build_registry_full(&facts, plugins);

    Ok(Build {
        registry,
        docstring_warnings,
        parser_diagnostics,
        resolution_warnings,
        plugin_warnings,
        files: files.len(),
    })
}

fn input_path(path: &Path, project_root: &Path) -> PathBuf {
    if path.is_absolute() || project_root.as_os_str().is_empty() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

fn registry_path(path: &Path, project_root: &Path) -> PathBuf {
    if project_root.as_os_str().is_empty() {
        return path.to_path_buf();
    }
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .to_path_buf()
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if path.is_dir() {
        let mut entries: Vec<PathBuf> = fs::read_dir(path)?
            .collect::<io::Result<Vec<_>>>()?
            .into_iter()
            .map(|entry| entry.path())
            .collect();
        entries.sort();
        for entry in entries {
            collect_files(&entry, files)?;
        }
    } else if path.is_file() && is_source(path) {
        files.push(path.to_path_buf());
    }
    Ok(())
}

fn is_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(std::ffi::OsStr::to_str),
        Some("rb" | "rbs")
    )
}

fn is_rbs(path: &Path) -> bool {
    path.extension().and_then(std::ffi::OsStr::to_str) == Some("rbs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_paths_are_relative_to_project_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let lib = root.path().join("lib");
        fs::create_dir(&lib).expect("create lib");
        fs::write(
            lib.join("example.rb"),
            "class Example\n  def hello; end\nend\n",
        )
        .expect("write source");

        let build = build_with(
            &[lib],
            &BuildOptions {
                project_root: root.path().to_path_buf(),
                ..BuildOptions::default()
            },
        )
        .expect("build registry");

        assert!(
            build
                .registry
                .files()
                .keys()
                .any(|path| path.to_string_lossy() == "lib/example.rb")
        );
    }

    #[test]
    fn builds_a_registry_from_a_source_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        let lib = root.path().join("lib");
        fs::create_dir(&lib).expect("create lib");
        fs::write(
            lib.join("example.rb"),
            "class Example\n  # Says hello.\n  def hello; end\nend\n",
        )
        .expect("write source");

        let build = build(&[lib]).expect("build registry");

        assert_eq!(build.files, 1);
        let hello = build.registry.find("Example#hello").expect("hello method");
        assert!(hello.docstring.text.contains("Says hello."));
    }
}
