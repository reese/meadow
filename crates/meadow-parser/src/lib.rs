//! The meadow parser stage.
//!
//! [`parse_source`] walks a Prism AST and returns an owned [`FileFacts`] value:
//! an ordered event stream, attached docstrings, and parser diagnostics. Prism
//! types are deliberately kept private to this crate; the result is `Send`,
//! `Sync`, and serializable so callers can parse files in parallel and drop the
//! native Prism arena inside each worker.

mod comments;
mod event;
mod rbs;
mod walker;

use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub use meadow_model::{SourceLocation, SourceText};

pub use event::{
    AliasEvent, Argument, BlockEvent, CallEvent, ClassCloseEvent, ClassOpenEvent,
    ClassVariableWriteEvent, CommentEvent, ConstantWriteEvent, Diagnostic, DiagnosticLevel,
    Docstring, Event, InstanceVariableWriteEvent, MethodDefEvent, MethodScope, ModuleCloseEvent,
    ModuleOpenEvent, Param, ParamKind, SingletonClassCloseEvent, SingletonClassOpenEvent,
};
pub use rbs::parse_rbs_source;

use comments::CommentIndex;
use walker::Walker;

/// The Ruby syntax version Prism should target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RubyVersion {
    /// Ruby 3.3 syntax.
    CRuby3_3,
    /// Ruby 3.4 syntax.
    CRuby3_4,
    /// Ruby 4.0 syntax.
    CRuby4_0,
    /// Ruby 4.1 syntax.
    CRuby4_1,
}

impl From<RubyVersion> for ruby_prism::Version {
    fn from(version: RubyVersion) -> Self {
        match version {
            RubyVersion::CRuby3_3 => Self::CRuby3_3,
            RubyVersion::CRuby3_4 => Self::CRuby3_4,
            RubyVersion::CRuby4_0 => Self::CRuby4_0,
            RubyVersion::CRuby4_1 => Self::CRuby4_1,
        }
    }
}

/// Options that affect how a file is parsed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseConfig {
    /// Override Prism's default Ruby syntax version.
    pub ruby_version: Option<RubyVersion>,
}

/// Everything the parser learned about one source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFacts {
    /// Path of the source file.
    pub path: PathBuf,
    /// Events in document order.
    pub events: Vec<Event>,
    /// Syntax errors and warnings reported by Prism.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses a UTF-8 Ruby source string using the default configuration.
#[must_use]
pub fn parse_source(path: impl Into<PathBuf>, source: &str) -> FileFacts {
    parse_source_bytes_with(path, source.as_bytes(), &ParseConfig::default())
}

/// Parses a UTF-8 Ruby source string with explicit options.
#[must_use]
pub fn parse_source_with(
    path: impl Into<PathBuf>,
    source: &str,
    config: &ParseConfig,
) -> FileFacts {
    parse_source_bytes_with(path, source.as_bytes(), config)
}

/// Parses raw Ruby source bytes using the default configuration.
///
/// The bytes are passed to Prism unchanged, so source files in non-UTF-8
/// encodings are parsed according to their magic comment or Prism's defaults.
#[must_use]
pub fn parse_source_bytes(path: impl Into<PathBuf>, source: &[u8]) -> FileFacts {
    parse_source_bytes_with(path, source, &ParseConfig::default())
}

/// Parses raw Ruby source bytes with explicit options.
///
/// All text in the returned events is stored as [`SourceText`], which preserves
/// the original bytes instead of assuming UTF-8.
#[must_use]
pub fn parse_source_bytes_with(
    path: impl Into<PathBuf>,
    source: &[u8],
    config: &ParseConfig,
) -> FileFacts {
    let path = path.into();
    if path.extension().and_then(std::ffi::OsStr::to_str) == Some("rbs") {
        return rbs::parse_rbs_bytes_with(path, source, *config);
    }
    let mut options = ruby_prism::Options::default();

    if let Some(filepath) = path.to_str() {
        options = options.filepath(filepath);
    }
    if let Some(version) = config.ruby_version {
        options = options.version(version.into());
    }

    let options = options.build();
    let result = ruby_prism::parse_with_options(source, &options);

    let mut diagnostics = Vec::new();
    for error in result.errors() {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Error,
            message: SourceText::from_bytes(error.message()),
            location: Some(walker::source_location(&error.location())),
        });
    }
    for warning in result.warnings() {
        diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            message: SourceText::from_bytes(warning.message()),
            location: Some(walker::source_location(&warning.location())),
        });
    }

    let comments = CommentIndex::from_parse(&result);
    let mut walker = Walker::new(comments);
    walker.walk_node(&result.node());

    FileFacts {
        path,
        events: walker.finish(),
        diagnostics,
    }
}

/// Reads and parses one file from disk.
///
/// Bytes are passed to Prism unchanged so that arbitrary source encodings and
/// byte sequences never crash the process.
///
/// # Errors
///
/// Returns an I/O error when the file cannot be read.
pub fn parse_file(path: impl AsRef<Path>) -> io::Result<FileFacts> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    Ok(parse_source_bytes(path.to_path_buf(), &bytes))
}

/// Parses a batch of in-memory sources in parallel.
///
/// Results preserve the input order, which keeps downstream handling
/// deterministic.
#[must_use]
pub fn parse_sources(sources: &[(PathBuf, String)]) -> Vec<FileFacts> {
    sources
        .par_iter()
        .map(|(path, source)| parse_source(path.clone(), source))
        .collect()
}

/// Parses a batch of raw in-memory sources in parallel.
///
/// Results preserve the input order, which keeps downstream handling
/// deterministic.
#[must_use]
pub fn parse_sources_bytes(sources: &[(PathBuf, Vec<u8>)]) -> Vec<FileFacts> {
    sources
        .par_iter()
        .map(|(path, source)| parse_source_bytes(path.clone(), source))
        .collect()
}
