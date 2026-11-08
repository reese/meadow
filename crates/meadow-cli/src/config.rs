//! Configuration file loading and option merging.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::RunArgs;

/// A meadow subcommand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Build documentation.
    Doc,
    /// Print statistics.
    Stats,
    /// List objects.
    List,
    /// Serve and rebuild documentation.
    Server,
}

/// All options that drive a run.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct RunOptions {
    /// Command to execute.
    pub command: Command,
    /// Input globs or file paths.
    pub inputs: Vec<String>,
    /// Exclusion globs.
    pub exclude: Vec<String>,
    /// HTML output directory.
    pub output_dir: PathBuf,
    /// Site title.
    pub title: String,
    /// README path.
    pub readme: Option<PathBuf>,
    /// Main page path.
    pub main: Option<PathBuf>,
    /// Extra documentation files.
    pub files: Vec<PathBuf>,
    /// Asset mappings from source to destination.
    pub assets: Vec<(PathBuf, PathBuf)>,
    /// Optional HTML template override directory.
    pub template_path: Option<PathBuf>,
    /// Markup provider name.
    pub markup: String,
    /// Include private objects.
    pub show_private: bool,
    /// Include protected objects.
    pub show_protected: bool,
    /// API tags to include or hide.
    pub api: Vec<String>,
    /// Whether `--api` should hide matching objects instead of showing them.
    pub hide_api: bool,
    /// Plugin names.
    pub plugins: Vec<String>,
    /// Cache/registry directory.
    pub db: PathBuf,
    /// Whether cached parse facts may be reused.
    pub use_cache: bool,
    /// Whether caches and the registry should be written.
    pub save: bool,
    /// Whether warnings should cause a nonzero exit.
    pub fail_on_warning: bool,
    /// Silence unresolved-name warnings.
    pub no_warn_unresolved: bool,
    /// Escape raw HTML in markup.
    pub sanitize: bool,
    /// Include generation timestamps.
    pub timestamps: bool,
    /// Prism Ruby syntax version override.
    pub ruby_version: Option<String>,
    /// Show inherited methods.
    pub show_inherited: bool,
    /// Embed mixin methods in namespace method lists.
    pub embed_mixins: bool,
    /// Follow local `require_relative` calls.
    pub follow_requires: bool,
    /// Optional external proxy resolution cache.
    pub proxy_cache: Option<PathBuf>,
    /// `ActiveRecord` schema path override.
    pub ar_schema: Option<PathBuf>,
    /// `ActiveRecord` model superclass names.
    pub ar_models: Vec<String>,
    /// Inflector irregular overrides.
    pub irregulars: BTreeMap<String, String>,
    /// Optional registry JSON export path.
    pub json_export: Option<PathBuf>,
    /// Verbosity level.
    pub verbose: u8,
    /// Whether normal output should be suppressed.
    pub quiet: bool,
    /// Query expression for `list`.
    pub query: Option<String>,
    /// Output format for `list`.
    pub format: String,
    /// Whether `stats` should list undocumented objects.
    pub list_undoc: bool,
    /// Server port.
    pub port: u16,
    /// Explicit config file path.
    pub config: Option<PathBuf>,
    /// Warnings emitted while parsing configuration.
    pub warnings: Vec<String>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            command: Command::Doc,
            inputs: Vec::new(),
            exclude: Vec::new(),
            output_dir: PathBuf::from("doc"),
            title: "Documentation".to_string(),
            readme: None,
            main: None,
            files: Vec::new(),
            assets: Vec::new(),
            template_path: None,
            markup: "markdown".to_string(),
            show_private: false,
            show_protected: true,
            api: Vec::new(),
            hide_api: false,
            plugins: Vec::new(),
            db: PathBuf::from(".meadow"),
            use_cache: true,
            save: true,
            fail_on_warning: false,
            no_warn_unresolved: false,
            sanitize: false,
            timestamps: false,
            ruby_version: None,
            show_inherited: true,
            embed_mixins: false,
            follow_requires: false,
            proxy_cache: None,
            ar_schema: None,
            ar_models: Vec::new(),
            irregulars: BTreeMap::new(),
            json_export: None,
            verbose: 0,
            quiet: false,
            query: None,
            format: "text".to_string(),
            list_undoc: false,
            port: 8808,
            config: None,
            warnings: Vec::new(),
        }
    }
}

/// Builds run options by layering config files underneath command-line flags.
#[must_use]
pub fn resolve(command: Command, args: RunArgs) -> RunOptions {
    let mut options = RunOptions {
        command,
        ..RunOptions::default()
    };

    options.config.clone_from(&args.config);
    load_config_files(&mut options, args.config.as_deref());
    args.apply(&mut options);
    options
}

fn load_config_files(options: &mut RunOptions, explicit: Option<&Path>) {
    if let Some(path) = explicit {
        load_config_file(options, path);
        return;
    }

    let toml = Path::new(".meadow.toml");
    if toml.is_file() {
        load_config_file(options, toml);
    }
    let yardopts = Path::new(".yardopts");
    if yardopts.is_file() {
        load_config_file(options, yardopts);
    }
}

fn load_config_file(options: &mut RunOptions, path: &Path) {
    let Ok(contents) = fs::read_to_string(path) else {
        options
            .warnings
            .push(format!("cannot read config {}", path.display()));
        return;
    };

    if path.extension().and_then(|extension| extension.to_str()) == Some("toml") {
        apply_toml(options, &contents);
    } else {
        let tokens = tokenize(&contents);
        RunArgs::apply_tokens(&tokens, options);
    }
}

fn apply_toml(options: &mut RunOptions, contents: &str) {
    let Ok(value) = contents.parse::<toml::Value>() else {
        options.warnings.push("invalid .meadow.toml".to_string());
        return;
    };
    let Some(table) = value.as_table() else {
        return;
    };

    for (key, value) in table {
        match key.as_str() {
            "output-dir" | "output_dir" => set_string(value, &mut options.output_dir),
            "title" => set_string_value(value, &mut options.title),
            "readme" => set_optional_path(value, &mut options.readme),
            "main" => set_optional_path(value, &mut options.main),
            "markup" => set_string_value(value, &mut options.markup),
            "db" => set_string(value, &mut options.db),
            "files" => set_path_list(value, &mut options.files),
            "template-path" | "template_path" => {
                set_optional_path(value, &mut options.template_path);
            }
            "exclude" => set_string_list(value, &mut options.exclude),
            "plugins" => set_string_list(value, &mut options.plugins),
            "api" => set_string_list(value, &mut options.api),
            "private" => options.show_private = value.as_bool().unwrap_or(options.show_private),
            "protected" => {
                options.show_protected = value.as_bool().unwrap_or(options.show_protected);
            }
            "hide-api" | "hide_api" => {
                options.hide_api = value.as_bool().unwrap_or(options.hide_api);
            }
            "use-cache" | "use_cache" => {
                options.use_cache = value.as_bool().unwrap_or(options.use_cache);
            }
            "no-save" | "no_save" => options.save = !value.as_bool().unwrap_or(false),
            "fail-on-warning" | "fail_on_warning" => {
                options.fail_on_warning = value.as_bool().unwrap_or(options.fail_on_warning);
            }
            "no-warn-unresolved" | "no_warn_unresolved" => {
                options.no_warn_unresolved = value.as_bool().unwrap_or(options.no_warn_unresolved);
            }
            "sanitize" => options.sanitize = value.as_bool().unwrap_or(options.sanitize),
            "timestamps" => options.timestamps = value.as_bool().unwrap_or(options.timestamps),
            "ruby-version" | "ruby_version" => {
                set_optional_string_value(value, &mut options.ruby_version);
            }
            "no-inherited" | "no_inherited" => {
                options.show_inherited = !value.as_bool().unwrap_or(false);
            }
            "embed-mixins" | "embed_mixins" => {
                options.embed_mixins = value.as_bool().unwrap_or(options.embed_mixins);
            }
            "follow-requires" | "follow_requires" => {
                options.follow_requires = value.as_bool().unwrap_or(options.follow_requires);
            }
            "proxy-cache" | "proxy_cache" => set_optional_path(value, &mut options.proxy_cache),
            "activerecord-schema" | "activerecord_schema" => {
                set_optional_path(value, &mut options.ar_schema);
            }
            "activerecord-models" | "activerecord_models" => {
                set_string_list(value, &mut options.ar_models);
            }
            "irregulars" => set_irregulars(value, &mut options.irregulars),
            unknown => options
                .warnings
                .push(format!("unknown config key '{unknown}' (ignored)")),
        }
    }
}

fn set_string(value: &toml::Value, target: &mut PathBuf) {
    if let Some(value) = value.as_str() {
        *target = PathBuf::from(value);
    }
}

fn set_string_value(value: &toml::Value, target: &mut String) {
    if let Some(value) = value.as_str() {
        *target = value.to_string();
    }
}

fn set_optional_string_value(value: &toml::Value, target: &mut Option<String>) {
    if let Some(value) = value.as_str() {
        *target = Some(value.to_string());
    }
}

fn set_irregulars(value: &toml::Value, target: &mut BTreeMap<String, String>) {
    if let Some(table) = value.as_table() {
        for (key, value) in table {
            if let Some(value) = value.as_str() {
                target.insert(key.clone(), value.to_string());
            }
        }
    }
}

fn set_optional_path(value: &toml::Value, target: &mut Option<PathBuf>) {
    if let Some(value) = value.as_str() {
        *target = Some(PathBuf::from(value));
    }
}

fn set_path_list(value: &toml::Value, target: &mut Vec<PathBuf>) {
    if let Some(values) = value.as_array() {
        *target = values
            .iter()
            .filter_map(|value| value.as_str().map(PathBuf::from))
            .collect();
    }
}

fn set_string_list(value: &toml::Value, target: &mut Vec<String>) {
    if let Some(values) = value.as_array() {
        *target = values
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect();
    }
}

/// Tokenizes `.yardopts` contents using shell-like quoting and comments.
#[must_use]
pub fn tokenize(contents: &str) -> Vec<String> {
    let characters: Vec<char> = contents.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut has_token = false;
    let mut index = 0;

    while index < characters.len() {
        let character = characters[index];
        index += 1;

        if escaped {
            current.push(character);
            escaped = false;
            has_token = true;
            continue;
        }

        if let Some(active) = quote {
            if character == active {
                quote = None;
            } else if character == '\\' && active == '"' {
                escaped = true;
            } else {
                current.push(character);
            }
            continue;
        }

        match character {
            '#' => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
                while index < characters.len() && characters[index] != '\n' {
                    index += 1;
                }
            }
            '\'' | '"' => {
                quote = Some(character);
                has_token = true;
            }
            '\\' => escaped = true,
            character if character.is_whitespace() => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            _ => {
                current.push(character);
                has_token = true;
            }
        }
    }

    if has_token {
        tokens.push(current);
    }
    tokens
}
