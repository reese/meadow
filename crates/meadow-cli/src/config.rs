//! Configuration and command-line option parsing.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Parses the command and options from process arguments.
///
/// # Errors
///
/// Returns a message when the command line is invalid.
pub fn parse(args: &[String]) -> Result<RunOptions, String> {
    let Some(command_name) = args.first() else {
        return Err("usage: meadow <doc|stats|list|server> [options] [files...]".to_string());
    };
    let command = match command_name.as_str() {
        "doc" => Command::Doc,
        "stats" => Command::Stats,
        "list" => Command::List,
        "server" => Command::Server,
        "--help" | "-h" | "help" => {
            return Err(help_text());
        }
        "--version" | "-V" => return Err(version_text()),
        other => return Err(format!("unknown command '{other}'\n{}", help_text())),
    };

    let mut options = RunOptions {
        command,
        ..RunOptions::default()
    };

    let cli = &args[1..];
    let explicit_config = find_flag_value(cli, "--config").map(PathBuf::from);
    options.config.clone_from(&explicit_config);
    load_config_files(&mut options, explicit_config.as_deref());

    parse_tokens(cli, &mut options, true);
    Ok(options)
}

/// Returns command help.
#[must_use]
pub fn help_text() -> String {
    [
        "meadow <doc|stats|list|server> [options] [files...]",
        "",
        "Commands:",
        "  doc      Build HTML documentation",
        "  stats    Print documentation statistics",
        "  list     List objects, optionally filtered by --query",
        "  server   Serve docs and rebuild on changes",
        "",
        "Options:",
        "  --output-dir DIR   --title TITLE   --readme FILE   --main FILE",
        "  --files PATHS      --asset SRC:DEST   --template-path DIR",
        "  --exclude GLOBS",
        "  --private          --protected      --no-private",
        "  --api TAG          --hide-api       --plugin NAME",
        "  --markup NAME      --query EXPR     --format json",
        "  --db DIR           --use-cache      --no-cache       --no-save",
        "  --json-export FILE --fail-on-warning --config FILE",
        "  --sanitize --timestamps --ruby-version V --no-inherited",
        "  --embed-mixins --follow-requires --proxy-cache FILE",
        "  --list-undoc       --port PORT      -q / -v",
    ]
    .join("\n")
}

fn version_text() -> String {
    format!("meadow {}", env!("CARGO_PKG_VERSION"))
}

fn find_flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == flag {
            return args.get(index + 1).cloned();
        }
        if let Some(value) = args[index].strip_prefix(&format!("{flag}=")) {
            return Some(value.to_string());
        }
        index += 1;
    }
    None
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
        parse_tokens(&tokens, options, false);
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

fn parse_tokens(tokens: &[String], options: &mut RunOptions, cli: bool) {
    let mut index = 0;
    let mut positional = false;

    while index < tokens.len() {
        let token = &tokens[index];
        if positional {
            options.inputs.push(token.clone());
            index += 1;
            continue;
        }
        if token == "-" || token == "--" {
            positional = true;
            index += 1;
            continue;
        }
        if !token.starts_with('-') {
            options.inputs.push(token.clone());
            index += 1;
            continue;
        }

        let (flag, inline_value) = split_flag(token);
        let mut next_value = || {
            if let Some(value) = inline_value {
                return Some(value.to_string());
            }
            index += 1;
            tokens.get(index).cloned()
        };

        if !parse_flag(&flag, &mut next_value, options) {
            options.warnings.push(format!(
                "unknown option '{}' (ignored)",
                if flag.is_empty() { token } else { &flag }
            ));
        }
        index += 1;
    }

    let _ = cli;
}

fn parse_flag<F: FnMut() -> Option<String>>(
    flag: &str,
    next_value: &mut F,
    options: &mut RunOptions,
) -> bool {
    match flag {
        "--output-dir" => take_path(next_value, &mut options.output_dir),
        "--title" => take_string(next_value, &mut options.title),
        "--readme" => take_optional_path(next_value, &mut options.readme),
        "--main" => take_optional_path(next_value, &mut options.main),
        "--db" => take_path(next_value, &mut options.db),
        "--markup" => take_string(next_value, &mut options.markup),
        "--query" => {
            if let Some(value) = next_value() {
                options.query = Some(value);
            }
        }
        "--format" => take_string(next_value, &mut options.format),
        "--json-export" => take_optional_path(next_value, &mut options.json_export),
        "--exclude" => {
            if let Some(value) = next_value() {
                options.exclude.extend(split_list(&value));
            }
        }
        "--files" => {
            if let Some(value) = next_value() {
                options
                    .files
                    .extend(split_list(&value).into_iter().map(PathBuf::from));
            }
        }
        "--template-path" => take_optional_path(next_value, &mut options.template_path),
        "--asset" => {
            if let Some(value) = next_value() {
                if let Some((source, destination)) = value.split_once(':') {
                    options
                        .assets
                        .push((PathBuf::from(source), PathBuf::from(destination)));
                } else {
                    options
                        .warnings
                        .push(format!("invalid --asset value '{value}'"));
                }
            }
        }
        "--plugin" => {
            if let Some(value) = next_value() {
                options.plugins.extend(split_list(&value));
            }
        }
        "--api" => {
            if let Some(value) = next_value() {
                options.api.extend(split_list(&value));
            }
        }
        "--port" => {
            if let Some(value) = next_value() {
                options.port = value.parse().unwrap_or(options.port);
            }
        }
        "--private" => options.show_private = true,
        "--no-private" => options.show_private = false,
        "--protected" => options.show_protected = true,
        "--no-protected" => options.show_protected = false,
        "--hide-api" => options.hide_api = true,
        "--use-cache" => options.use_cache = true,
        "--no-cache" => options.use_cache = false,
        "--no-save" => options.save = false,
        "--fail-on-warning" => options.fail_on_warning = true,
        "--no-warn-unresolved" => options.no_warn_unresolved = true,
        "--sanitize" => options.sanitize = true,
        "--timestamps" => options.timestamps = true,
        "--no-inherited" => options.show_inherited = false,
        "--embed-mixins" => options.embed_mixins = true,
        "--no-embed-mixins" => options.embed_mixins = false,
        "--follow-requires" => options.follow_requires = true,
        "--ruby-version" => {
            if let Some(value) = next_value() {
                options.ruby_version = Some(value);
            }
        }
        "--proxy-cache" => take_optional_path(next_value, &mut options.proxy_cache),
        "--list-undoc" => options.list_undoc = true,
        "--config" => {
            let _ = next_value();
        }
        "--no-output" | "--no-stats" => {}
        "-q" | "--quiet" => options.quiet = true,
        "-v" | "--verbose" => options.verbose = options.verbose.saturating_add(1),
        "--help" | "-h" => {
            options.warnings.push(help_text());
        }
        "--version" | "-V" => options.warnings.push(version_text()),
        _ => return false,
    }
    true
}

fn split_flag(token: &str) -> (String, Option<&str>) {
    match token.split_once('=') {
        Some((flag, value)) => (flag.to_string(), Some(value)),
        None => (token.to_string(), None),
    }
}

fn take_path<F: FnMut() -> Option<String>>(next: &mut F, target: &mut PathBuf) {
    if let Some(value) = next() {
        *target = PathBuf::from(value);
    }
}

fn take_optional_path<F: FnMut() -> Option<String>>(next: &mut F, target: &mut Option<PathBuf>) {
    if let Some(value) = next() {
        *target = Some(PathBuf::from(value));
    }
}

fn take_string<F: FnMut() -> Option<String>>(next: &mut F, target: &mut String) {
    if let Some(value) = next() {
        *target = value;
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
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
