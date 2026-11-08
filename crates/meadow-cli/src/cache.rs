//! Registry and parse-fact caching for incremental runs.

use std::collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use meadow_handlers::{Handler, handle_into_registry};
use meadow_model::Registry;
use meadow_parser::{
    DiagnosticLevel, FileFacts, ParseConfig, RubyVersion, SourceText, parse_source_bytes_with,
};
use meadow_plugins::{PluginSettings, builtin_plugins_with};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::RunOptions;
use crate::discover::discover;

const FORMAT_VERSION: u32 = 3;
const BUILD_BATCH: usize = 128;

/// A cached file state used to detect changes between runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    size: u64,
    mtime_ns: u64,
    hash: u64,
}

/// The cache manifest saved under `.meadow/manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    format: u32,
    version: String,
    options_hash: u64,
    files: BTreeMap<String, FileState>,
}

/// Statistics about how a registry was produced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuildStats {
    /// Number of source files.
    pub files: usize,
    /// Files parsed in this run.
    pub parsed: usize,
    /// Files reused from cache.
    pub reused: usize,
    /// Whether the registry itself was loaded from cache.
    pub cache_hit: bool,
}

/// A parser diagnostic paired with its file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiagnostic {
    /// Source file.
    pub file: SourceText,
    /// Diagnostic level.
    pub level: DiagnosticLevel,
    /// Diagnostic message.
    pub message: SourceText,
}

/// A built registry and its build metadata.
pub struct BuildResult {
    /// The resolved registry.
    pub registry: Registry,
    /// Docstring warnings emitted while handling changed files.
    pub warnings: Vec<meadow_docstring::DocstringWarning>,
    /// Parser syntax errors and warnings.
    pub parser_diagnostics: Vec<FileDiagnostic>,
    /// Unresolved-name warnings from resolution.
    pub resolution_warnings: Vec<meadow_resolve::ResolutionWarning>,
    /// Plugin warnings.
    pub plugin_warnings: Vec<String>,
    /// Cache/build statistics.
    pub stats: BuildStats,
}

/// Loads a cached registry or builds one from source files.
///
/// # Errors
///
/// Returns an error when required cache or source files cannot be read or
/// written.
pub fn load_or_build(options: &RunOptions) -> io::Result<BuildResult> {
    let mut paths = discover(options);
    if options.follow_requires {
        paths = expand_relative_requires(paths, options)?;
    }
    let options_hash = options_hash(options);
    let db = &options.db;

    if options.use_cache
        && let Some(result) = try_registry_cache(options, &paths, options_hash, db)?
    {
        return Ok(result);
    }

    let mut old_facts: BTreeMap<String, FileFacts> = BTreeMap::new();
    let mut old_manifest: Option<Manifest> = None;
    if options.use_cache {
        old_manifest = load_manifest(db).ok().filter(|manifest| {
            manifest.format == FORMAT_VERSION
                && manifest.options_hash == options_hash
                && manifest.version == env!("CARGO_PKG_VERSION")
        });
        if old_manifest.is_some() {
            old_facts = load_facts(db).unwrap_or_default();
        }
    }

    if options.use_cache
        && options.save
        && let Some(result) = try_surgical_update(
            options,
            &paths,
            options_hash,
            db,
            old_manifest.as_ref(),
            &old_facts,
        )?
    {
        return Ok(result);
    }

    let Built {
        registry,
        warnings,
        parser_diagnostics,
        resolution_warnings,
        plugin_warnings,
        stats,
        manifest_files,
    } = build_streaming(options, &paths, db, old_manifest.as_ref(), &mut old_facts)?;

    if options.save {
        save_build(
            db,
            options_hash,
            &registry,
            manifest_files,
            &parser_diagnostics,
        )?;
    }

    if let Some(path) = &options.json_export {
        write_json(&registry, path)?;
    }

    Ok(BuildResult {
        registry,
        warnings,
        parser_diagnostics,
        resolution_warnings,
        plugin_warnings,
        stats,
    })
}

struct Built {
    registry: Registry,
    warnings: Vec<meadow_docstring::DocstringWarning>,
    parser_diagnostics: Vec<FileDiagnostic>,
    resolution_warnings: Vec<meadow_resolve::ResolutionWarning>,
    plugin_warnings: Vec<String>,
    stats: BuildStats,
    manifest_files: BTreeMap<String, FileState>,
}

struct ParsedFile {
    key: String,
    state: FileState,
    facts: Option<FileFacts>,
    can_reuse: bool,
}

fn build_streaming(
    options: &RunOptions,
    paths: &[PathBuf],
    db: &Path,
    manifest: Option<&Manifest>,
    old_facts: &mut BTreeMap<String, FileFacts>,
) -> io::Result<Built> {
    let mut ordered: Vec<&PathBuf> = paths.iter().collect();
    ordered.sort_by(|left, right| {
        is_rbs_path(left)
            .cmp(&is_rbs_path(right))
            .then_with(|| left.cmp(right))
    });

    let mut handler = Handler::with_plugins(build_plugins(options)?);
    let mut parser_diagnostics: Vec<FileDiagnostic> = Vec::new();
    let mut manifest_files = BTreeMap::new();
    let mut facts_writer = if options.save {
        fs::create_dir_all(db)?;
        Some(FactsWriter::create(&db.join("facts.bin"), paths.len())?)
    } else {
        None
    };
    let mut parsed_count = 0;
    let mut reused = 0;

    for chunk in ordered.chunks(BUILD_BATCH) {
        let results = parse_chunk(chunk, options, manifest, old_facts)?;
        for (path, parsed) in chunk.iter().zip(results) {
            manifest_files.insert(parsed.key.clone(), parsed.state);
            let facts = if parsed.can_reuse {
                if let Some(facts) = old_facts.remove(&parsed.key) {
                    reused += 1;
                    facts
                } else {
                    parsed_count += 1;
                    parse_path(path, options)?
                }
            } else {
                parsed_count += 1;
                parsed.facts.expect("parsed facts for changed file")
            };
            parser_diagnostics.extend(diagnostics_for(&facts));
            if let Some(writer) = &mut facts_writer {
                writer.write(&facts)?;
            }
            handler.handle_file(&facts);
        }
    }

    if let Some(writer) = facts_writer {
        writer.finish()?;
    }

    let warnings = handler.warnings().to_vec();
    let (registry, resolution_warnings, plugin_warnings) = handler.finish_reporting();
    let stats = BuildStats {
        files: paths.len(),
        parsed: parsed_count,
        reused,
        cache_hit: false,
    };
    Ok(Built {
        registry,
        warnings,
        parser_diagnostics,
        resolution_warnings,
        plugin_warnings,
        stats,
        manifest_files,
    })
}

fn parse_chunk(
    chunk: &[&PathBuf],
    options: &RunOptions,
    manifest: Option<&Manifest>,
    old_facts: &BTreeMap<String, FileFacts>,
) -> io::Result<Vec<ParsedFile>> {
    chunk
        .par_iter()
        .map(|path| {
            let path = *path;
            let key = path.to_string_lossy().into_owned();
            let previous = manifest
                .and_then(|manifest| manifest.files.get(&key))
                .copied();
            let metadata_unchanged = previous
                .as_ref()
                .is_some_and(|state| metadata_matches(state, path).unwrap_or(false));
            if metadata_unchanged && old_facts.contains_key(&key) {
                return Ok(ParsedFile {
                    key,
                    state: previous.expect("previous state"),
                    facts: None,
                    can_reuse: true,
                });
            }
            let bytes = fs::read(path)?;
            let state = file_state_from_bytes(path, &bytes)?;
            let content_unchanged = previous
                .as_ref()
                .is_some_and(|previous| previous.hash == state.hash)
                && old_facts.contains_key(&key);
            if content_unchanged {
                return Ok(ParsedFile {
                    key,
                    state,
                    facts: None,
                    can_reuse: true,
                });
            }
            Ok(ParsedFile {
                key,
                state,
                facts: Some(parse_bytes(path, &bytes, options)),
                can_reuse: false,
            })
        })
        .collect()
}

fn save_build(
    db: &Path,
    options_hash: u64,
    registry: &Registry,
    manifest_files: BTreeMap<String, FileState>,
    parser_diagnostics: &[FileDiagnostic],
) -> io::Result<()> {
    fs::create_dir_all(db)?;
    write_bincode(&db.join("registry.bin"), registry)?;
    write_bincode(&db.join("diagnostics.bin"), parser_diagnostics)?;
    let manifest = Manifest {
        format: FORMAT_VERSION,
        version: env!("CARGO_PKG_VERSION").to_string(),
        options_hash,
        files: manifest_files,
    };
    fs::write(
        db.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?,
    )
}

fn write_bincode<T>(path: &Path, value: &T) -> io::Result<()>
where
    T: ?Sized + Serialize,
{
    let file = fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    bincode::serialize_into(&mut writer, value).map_err(io::Error::other)?;
    writer.flush()
}

fn read_bincode<T>(path: &Path) -> io::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let file = fs::File::open(path)?;
    bincode::deserialize_from(BufReader::new(file)).map_err(io::Error::other)
}

struct FactsWriter {
    writer: BufWriter<fs::File>,
    temp: PathBuf,
    path: PathBuf,
}

impl FactsWriter {
    fn create(path: &Path, count: usize) -> io::Result<Self> {
        let temp = path.with_extension("bin.tmp");
        let file = fs::File::create(&temp)?;
        let mut writer = BufWriter::new(file);
        bincode::serialize_into(&mut writer, &u64::try_from(count).unwrap_or(u64::MAX))
            .map_err(io::Error::other)?;
        Ok(Self {
            writer,
            temp,
            path: path.to_path_buf(),
        })
    }

    fn write(&mut self, facts: &FileFacts) -> io::Result<()> {
        bincode::serialize_into(&mut self.writer, facts).map_err(io::Error::other)
    }

    fn finish(self) -> io::Result<()> {
        let Self { writer, temp, path } = self;
        writer
            .into_inner()
            .map_err(std::io::IntoInnerError::into_error)?;
        fs::rename(temp, path)
    }
}

fn is_rbs_path(path: &Path) -> bool {
    path.extension().and_then(std::ffi::OsStr::to_str) == Some("rbs")
}

fn try_surgical_update(
    options: &RunOptions,
    paths: &[PathBuf],
    options_hash: u64,
    db: &Path,
    manifest: Option<&Manifest>,
    old_facts: &BTreeMap<String, FileFacts>,
) -> io::Result<Option<BuildResult>> {
    let Some(manifest) = manifest else {
        return Ok(None);
    };
    if paths
        .iter()
        .any(|path| path.extension().and_then(std::ffi::OsStr::to_str) == Some("rbs"))
    {
        return Ok(None);
    }
    let Ok(mut registry) = read_bincode::<Registry>(&db.join("registry.bin")) else {
        return Ok(None);
    };

    let mut changed = Vec::new();
    let mut manifest_files = BTreeMap::new();
    let mut changed_keys = BTreeSet::new();
    for path in paths {
        let key = path.to_string_lossy().into_owned();
        let previous = manifest.files.get(&key).copied();
        if previous
            .as_ref()
            .is_some_and(|state| metadata_matches(state, path).unwrap_or(false))
        {
            manifest_files.insert(key, previous.expect("previous state"));
            continue;
        }
        let bytes = fs::read(path)?;
        let state = file_state_from_bytes(path, &bytes)?;
        let content_unchanged = previous
            .as_ref()
            .is_some_and(|previous| previous.hash == state.hash);
        manifest_files.insert(key.clone(), state);
        if !content_unchanged {
            changed.push((path.clone(), bytes));
            changed_keys.insert(SourceText::from(key.as_str()));
        }
    }
    if changed.is_empty() || changed.len() == paths.len() {
        return Ok(None);
    }

    registry.remove_files(&changed_keys);
    let changed_facts: Vec<FileFacts> = changed
        .par_iter()
        .map(|(path, bytes)| parse_bytes(path, bytes, options))
        .collect();
    let plugins = build_plugins(options)?;
    let output = handle_into_registry(registry, &changed_facts, plugins);
    let registry = output.registry;

    let mut facts_map = old_facts.clone();
    for facts in &changed_facts {
        facts_map.insert(facts.path.to_string_lossy().into_owned(), facts.clone());
    }
    let parser_diagnostics = diagnostics(&facts_map);
    write_bincode(&db.join("registry.bin"), &registry)?;
    let mut facts_writer = FactsWriter::create(&db.join("facts.bin"), facts_map.len())?;
    for facts in facts_map.values() {
        facts_writer.write(facts)?;
    }
    facts_writer.finish()?;
    write_bincode(&db.join("diagnostics.bin"), &parser_diagnostics)?;
    let manifest = Manifest {
        format: FORMAT_VERSION,
        version: env!("CARGO_PKG_VERSION").to_string(),
        options_hash,
        files: manifest_files,
    };
    fs::write(
        db.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?,
    )?;
    if let Some(path) = &options.json_export {
        write_json(&registry, path)?;
    }

    Ok(Some(BuildResult {
        registry,
        warnings: output.docstring_warnings,
        parser_diagnostics,
        resolution_warnings: output.resolution_warnings,
        plugin_warnings: output.plugin_warnings,
        stats: BuildStats {
            files: paths.len(),
            parsed: changed.len(),
            reused: paths.len() - changed.len(),
            cache_hit: false,
        },
    }))
}

fn build_plugins(options: &RunOptions) -> io::Result<Vec<Box<dyn meadow_plugins::Plugin>>> {
    Ok(builtin_plugins_with(
        &options.plugins,
        &std::env::current_dir()?,
        &PluginSettings {
            activerecord_schema: options.ar_schema.clone(),
            activerecord_models: options.ar_models.clone(),
            irregulars: options.irregulars.clone(),
        },
    ))
}

fn diagnostics(facts: &BTreeMap<String, FileFacts>) -> Vec<FileDiagnostic> {
    facts.values().flat_map(diagnostics_for).collect()
}

fn diagnostics_for(facts: &FileFacts) -> impl Iterator<Item = FileDiagnostic> + '_ {
    let file = SourceText::from_bytes(facts.path.to_string_lossy().as_bytes());
    facts
        .diagnostics
        .iter()
        .map(move |diagnostic| FileDiagnostic {
            file: file.clone(),
            level: diagnostic.level,
            message: diagnostic.message.clone(),
        })
}

fn try_registry_cache(
    options: &RunOptions,
    paths: &[PathBuf],
    options_hash: u64,
    db: &Path,
) -> io::Result<Option<BuildResult>> {
    let Some(manifest) = load_manifest(db).ok() else {
        return Ok(None);
    };
    if manifest.format != FORMAT_VERSION
        || manifest.options_hash != options_hash
        || manifest.version != env!("CARGO_PKG_VERSION")
        || manifest.files.len() != paths.len()
    {
        return Ok(None);
    }
    for path in paths {
        let key = path.to_string_lossy().into_owned();
        let Some(state) = manifest.files.get(&key) else {
            return Ok(None);
        };
        if !metadata_matches(state, path)? {
            return Ok(None);
        }
    }

    let Ok(registry) = read_bincode::<Registry>(&db.join("registry.bin")) else {
        return Ok(None);
    };

    if let Some(path) = &options.json_export {
        write_json(&registry, path)?;
    }

    let parser_diagnostics = load_diagnostics(db).unwrap_or_default();

    Ok(Some(BuildResult {
        registry,
        warnings: Vec::new(),
        parser_diagnostics,
        resolution_warnings: Vec::new(),
        plugin_warnings: Vec::new(),
        stats: BuildStats {
            files: paths.len(),
            parsed: 0,
            reused: paths.len(),
            cache_hit: true,
        },
    }))
}

fn write_json(registry: &Registry, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, registry).map_err(io::Error::other)?;
    writer.flush()
}

fn load_manifest(db: &Path) -> io::Result<Manifest> {
    let bytes = fs::read(db.join("manifest.json"))?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn load_facts(db: &Path) -> io::Result<BTreeMap<String, FileFacts>> {
    let file = fs::File::open(db.join("facts.bin"))?;
    let mut reader = BufReader::new(file);
    let count: u64 = bincode::deserialize_from(&mut reader).map_err(io::Error::other)?;
    let mut facts = BTreeMap::new();
    for _ in 0..count {
        let value: FileFacts = bincode::deserialize_from(&mut reader).map_err(io::Error::other)?;
        facts.insert(value.path.to_string_lossy().into_owned(), value);
    }
    Ok(facts)
}

fn load_diagnostics(db: &Path) -> io::Result<Vec<FileDiagnostic>> {
    read_bincode(&db.join("diagnostics.bin"))
}

fn metadata_stamp(path: &Path) -> io::Result<(u64, u64)> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();
    let mtime_ns = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
        });
    Ok((size, mtime_ns))
}

fn metadata_matches(state: &FileState, path: &Path) -> io::Result<bool> {
    let (size, mtime_ns) = metadata_stamp(path)?;
    Ok(state.size == size && state.mtime_ns == mtime_ns)
}

fn file_state_from_bytes(path: &Path, bytes: &[u8]) -> io::Result<FileState> {
    let metadata = fs::metadata(path)?;
    let mtime_ns = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
        });
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(FileState {
        size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        mtime_ns,
        hash: hasher.finish(),
    })
}

fn options_hash(options: &RunOptions) -> u64 {
    let mut hasher = DefaultHasher::new();
    options.markup.hash(&mut hasher);
    options.plugins.hash(&mut hasher);
    options.inputs.hash(&mut hasher);
    options.exclude.hash(&mut hasher);
    options.ruby_version.hash(&mut hasher);
    options.follow_requires.hash(&mut hasher);
    options.ar_schema.hash(&mut hasher);
    options.ar_models.hash(&mut hasher);
    options.irregulars.hash(&mut hasher);
    options.sanitize.hash(&mut hasher);
    hasher.finish()
}

fn parse_path(path: &Path, options: &RunOptions) -> io::Result<FileFacts> {
    let bytes = fs::read(path)?;
    Ok(parse_bytes(path, &bytes, options))
}

fn parse_bytes(path: &Path, bytes: &[u8], options: &RunOptions) -> FileFacts {
    let config = ParseConfig {
        ruby_version: options.ruby_version.as_deref().and_then(ruby_version),
    };
    parse_source_bytes_with(path.to_path_buf(), bytes, &config)
}

fn ruby_version(value: &str) -> Option<RubyVersion> {
    match value {
        "3.3" => Some(RubyVersion::CRuby3_3),
        "3.4" => Some(RubyVersion::CRuby3_4),
        "4.0" => Some(RubyVersion::CRuby4_0),
        "4.1" => Some(RubyVersion::CRuby4_1),
        _ => None,
    }
}

fn expand_relative_requires(paths: Vec<PathBuf>, options: &RunOptions) -> io::Result<Vec<PathBuf>> {
    let mut result = paths;
    let mut seen: std::collections::BTreeSet<PathBuf> = result.iter().cloned().collect();
    let mut index = 0;
    while index < result.len() {
        let facts = parse_path(&result[index], options)?;
        let directory = result[index]
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        for event in &facts.events {
            let meadow_parser::Event::Call(call) = event else {
                continue;
            };
            if call.name.as_bytes() != b"require_relative" {
                continue;
            }
            let Some(argument) = call.arguments.first() else {
                continue;
            };
            let relative = argument
                .literal
                .as_ref()
                .map_or_else(
                    || argument.source.to_string_lossy(),
                    |literal| literal.to_string_lossy(),
                )
                .into_owned();
            let mut candidate = directory.join(&relative);
            if candidate.extension().is_none() {
                candidate.set_extension("rb");
            }
            if candidate.is_file() && seen.insert(candidate.clone()) {
                result.push(candidate);
            }
        }
        index += 1;
    }
    result.sort();
    Ok(result)
}
