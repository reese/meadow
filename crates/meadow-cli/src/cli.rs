//! Command-line argument definitions.

use std::path::PathBuf;

use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{ArgAction, Args, Parser, Subcommand};

use crate::config::{Command, RunOptions};

/// The meadow command line interface.
#[derive(Debug, Parser)]
#[command(
    name = "meadow",
    version,
    about = "A fast, YARD-compatible documentation generator for Ruby"
)]
pub struct Cli {
    /// Subcommand to run.
    #[command(subcommand)]
    pub command: CliCommand,
}

/// A meadow subcommand.
#[derive(Debug, Subcommand)]
pub enum CliCommand {
    /// Build HTML documentation.
    Doc(RunArgs),
    /// Print documentation statistics.
    Stats(RunArgs),
    /// List objects, optionally filtered by --query.
    List(RunArgs),
    /// Serve docs and rebuild on changes.
    Server(RunArgs),
}

impl Cli {
    /// Splits the parsed arguments into a command and its options.
    #[must_use]
    pub fn into_parts(self) -> (Command, RunArgs) {
        match self.command {
            CliCommand::Doc(args) => (Command::Doc, args),
            CliCommand::Stats(args) => (Command::Stats, args),
            CliCommand::List(args) => (Command::List, args),
            CliCommand::Server(args) => (Command::Server, args),
        }
    }
}

/// Options shared by every subcommand.
#[derive(Debug, Clone, Default, Args)]
#[allow(clippy::struct_excessive_bools)]
pub struct RunArgs {
    /// Input globs or file paths.
    #[arg(value_name = "FILES")]
    pub inputs: Vec<String>,
    /// Exclusion globs.
    #[arg(long, value_name = "GLOBS", value_delimiter = ',')]
    pub exclude: Vec<String>,
    /// HTML output directory.
    #[arg(long, value_name = "DIR")]
    pub output_dir: Option<PathBuf>,
    /// Site title.
    #[arg(long, value_name = "TITLE")]
    pub title: Option<String>,
    /// README path.
    #[arg(long, value_name = "FILE")]
    pub readme: Option<PathBuf>,
    /// Main page path.
    #[arg(long, value_name = "FILE")]
    pub main: Option<PathBuf>,
    /// Extra documentation files.
    #[arg(long, value_name = "PATHS", value_delimiter = ',')]
    pub files: Vec<PathBuf>,
    /// Asset mapping from source to destination.
    #[arg(long, value_name = "SRC:DEST")]
    pub asset: Vec<String>,
    /// Optional HTML template override directory.
    #[arg(long, value_name = "DIR")]
    pub template_path: Option<PathBuf>,
    /// Markup provider name.
    #[arg(long, value_name = "NAME")]
    pub markup: Option<String>,
    /// Include private objects.
    #[arg(
        long,
        action = ArgAction::Set,
        num_args = 0,
        default_missing_value = "true",
        overrides_with = "no_private"
    )]
    pub private: Option<bool>,
    /// Exclude private objects.
    #[arg(
        long = "no-private",
        action = ArgAction::Set,
        num_args = 0,
        default_missing_value = "false",
        overrides_with = "private"
    )]
    pub no_private: Option<bool>,
    /// Include protected objects.
    #[arg(
        long,
        action = ArgAction::Set,
        num_args = 0,
        default_missing_value = "true",
        overrides_with = "no_protected"
    )]
    pub protected: Option<bool>,
    /// Exclude protected objects.
    #[arg(
        long = "no-protected",
        action = ArgAction::Set,
        num_args = 0,
        default_missing_value = "false",
        overrides_with = "protected"
    )]
    pub no_protected: Option<bool>,
    /// API tags to include.
    #[arg(long, value_name = "TAG", value_delimiter = ',')]
    pub api: Vec<String>,
    /// Hide objects matching --api instead of showing them.
    #[arg(long)]
    pub hide_api: bool,
    /// Built-in plugin names.
    #[arg(long, value_name = "NAME", value_delimiter = ',')]
    pub plugin: Vec<String>,
    /// Cache/registry directory.
    #[arg(long, value_name = "DIR")]
    pub db: Option<PathBuf>,
    /// Reuse cached parse facts.
    #[arg(
        long,
        action = ArgAction::Set,
        num_args = 0,
        default_missing_value = "true",
        overrides_with = "no_cache"
    )]
    pub use_cache: Option<bool>,
    /// Rebuild fully instead of reusing cached facts.
    #[arg(
        long = "no-cache",
        action = ArgAction::Set,
        num_args = 0,
        default_missing_value = "false",
        overrides_with = "use_cache"
    )]
    pub no_cache: Option<bool>,
    /// Avoid writing cache and registry files.
    #[arg(long = "no-save", action = ArgAction::Set, num_args = 0, default_missing_value = "true")]
    pub no_save: Option<bool>,
    /// Exit nonzero when warnings are emitted.
    #[arg(long)]
    pub fail_on_warning: bool,
    /// Silence unresolved-name warnings.
    #[arg(long)]
    pub no_warn_unresolved: bool,
    /// Escape raw HTML in markup.
    #[arg(long)]
    pub sanitize: bool,
    /// Include generation timestamps.
    #[arg(long)]
    pub timestamps: bool,
    /// Prism Ruby syntax version override.
    #[arg(long, value_name = "VERSION")]
    pub ruby_version: Option<String>,
    /// Exclude inherited methods.
    #[arg(long = "no-inherited", action = ArgAction::Set, num_args = 0, default_missing_value = "true")]
    pub no_inherited: Option<bool>,
    /// Embed mixin methods in namespace method lists.
    #[arg(
        long,
        action = ArgAction::Set,
        num_args = 0,
        default_missing_value = "true",
        overrides_with = "no_embed_mixins"
    )]
    pub embed_mixins: Option<bool>,
    /// Do not embed mixin methods.
    #[arg(
        long = "no-embed-mixins",
        action = ArgAction::Set,
        num_args = 0,
        default_missing_value = "false",
        overrides_with = "embed_mixins"
    )]
    pub no_embed_mixins: Option<bool>,
    /// Follow local `require_relative` calls.
    #[arg(long)]
    pub follow_requires: bool,
    /// External proxy resolution cache.
    #[arg(long, value_name = "FILE")]
    pub proxy_cache: Option<PathBuf>,
    /// Registry JSON export path.
    #[arg(long, value_name = "FILE")]
    pub json_export: Option<PathBuf>,
    /// Verbosity level.
    #[arg(short, long, action = ArgAction::Count)]
    pub verbose: u8,
    /// Suppress normal output.
    #[arg(short, long)]
    pub quiet: bool,
    /// Query expression for `list`.
    #[arg(long, value_name = "EXPR")]
    pub query: Option<String>,
    /// Output format for `list`.
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<String>,
    /// List undocumented objects.
    #[arg(long)]
    pub list_undoc: bool,
    /// Server port.
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,
    /// Explicit config file path.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
    /// Ignored for YARD compatibility.
    #[arg(long = "no-output", hide = true)]
    pub no_output: bool,
    /// Ignored for YARD compatibility.
    #[arg(long = "no-stats", hide = true)]
    pub no_stats: bool,
}

impl RunArgs {
    /// Applies the parsed arguments on top of existing options.
    #[allow(clippy::too_many_lines)]
    pub fn apply(self, options: &mut RunOptions) {
        options.inputs.extend(self.inputs);
        options.exclude.extend(self.exclude);
        options.files.extend(self.files);
        options.plugins.extend(self.plugin);
        options.api.extend(self.api);

        if let Some(value) = self.output_dir {
            options.output_dir = value;
        }
        if let Some(value) = self.title {
            options.title = value;
        }
        if let Some(value) = self.readme {
            options.readme = Some(value);
        }
        if let Some(value) = self.main {
            options.main = Some(value);
        }
        if let Some(value) = self.template_path {
            options.template_path = Some(value);
        }
        if let Some(value) = self.markup {
            options.markup = value;
        }
        if let Some(value) = self.db {
            options.db = value;
        }
        if let Some(value) = self.proxy_cache {
            options.proxy_cache = Some(value);
        }
        if let Some(value) = self.json_export {
            options.json_export = Some(value);
        }
        if let Some(value) = self.query {
            options.query = Some(value);
        }
        if let Some(value) = self.format {
            options.format = value;
        }
        if let Some(value) = self.ruby_version {
            options.ruby_version = Some(value);
        }
        if let Some(value) = self.port {
            options.port = value;
        }
        if let Some(value) = self.config {
            options.config = Some(value);
        }

        if let Some(value) = self.private {
            options.show_private = value;
        }
        if let Some(value) = self.no_private {
            options.show_private = value;
        }
        if let Some(value) = self.protected {
            options.show_protected = value;
        }
        if let Some(value) = self.no_protected {
            options.show_protected = value;
        }
        if let Some(value) = self.use_cache {
            options.use_cache = value;
        }
        if let Some(value) = self.no_cache {
            options.use_cache = value;
        }
        if let Some(value) = self.no_save {
            options.save = !value;
        }
        if let Some(value) = self.no_inherited {
            options.show_inherited = !value;
        }
        if let Some(value) = self.embed_mixins {
            options.embed_mixins = value;
        }
        if let Some(value) = self.no_embed_mixins {
            options.embed_mixins = value;
        }

        if self.hide_api {
            options.hide_api = true;
        }
        if self.fail_on_warning {
            options.fail_on_warning = true;
        }
        if self.no_warn_unresolved {
            options.no_warn_unresolved = true;
        }
        if self.sanitize {
            options.sanitize = true;
        }
        if self.timestamps {
            options.timestamps = true;
        }
        if self.follow_requires {
            options.follow_requires = true;
        }
        if self.quiet {
            options.quiet = true;
        }
        if self.list_undoc {
            options.list_undoc = true;
        }
        options.verbose = options.verbose.saturating_add(self.verbose);

        for asset in self.asset {
            if let Some((source, destination)) = asset.split_once(':') {
                options
                    .assets
                    .push((PathBuf::from(source), PathBuf::from(destination)));
            } else {
                options
                    .warnings
                    .push(format!("invalid --asset value '{asset}'"));
            }
        }
    }

    /// Parses config-file tokens (such as `.yardopts`) leniently.
    ///
    /// Unknown options are reported as warnings and ignored so that YARD
    /// configuration files continue to work.
    pub fn apply_tokens(tokens: &[String], options: &mut RunOptions) {
        let mut remaining = tokens.to_vec();
        loop {
            let mut argv = Vec::with_capacity(remaining.len() + 1);
            argv.push("meadow".to_string());
            argv.extend(remaining.iter().cloned());

            let mut command = clap::Command::new("meadow");
            command = <Self as Args>::augment_args(command);

            match command.try_get_matches_from(&argv) {
                Ok(matches) => {
                    if let Ok(args) = <Self as clap::FromArgMatches>::from_arg_matches(&matches) {
                        args.apply(options);
                    }
                    return;
                }
                Err(error) => {
                    if matches!(
                        error.kind(),
                        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                    ) {
                        return;
                    }
                    if error.kind() == ErrorKind::UnknownArgument
                        && let Some(ContextValue::String(invalid)) =
                            error.get(ContextKind::InvalidArg)
                    {
                        let invalid = invalid.clone();
                        let prefix = format!("{invalid}=");
                        let removed = remaining
                            .iter()
                            .position(|token| token == &invalid || token.starts_with(&prefix))
                            .map(|index| remaining.remove(index));
                        if removed.is_some() {
                            options
                                .warnings
                                .push(format!("unknown option '{invalid}' (ignored)"));
                            continue;
                        }
                    }
                    options
                        .warnings
                        .push(format!("cannot parse config: {error}"));
                    return;
                }
            }
        }
    }
}
