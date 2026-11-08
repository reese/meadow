//! The meadow command line interface.

mod cache;
mod config;
mod discover;
mod query;
mod server;

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;

use meadow_html::{SiteOptions, render_site};
use meadow_model::{CodeObject, KindData, ObjectKind};
use meadow_parser::DiagnosticLevel;

use crate::config::{Command, RunOptions};
use crate::query::Query;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .first()
        .is_some_and(|argument| matches!(argument.as_str(), "--help" | "-h" | "help"))
    {
        println!("{}", config::help_text());
        return ExitCode::SUCCESS;
    }
    if args
        .first()
        .is_some_and(|argument| matches!(argument.as_str(), "--version" | "-V"))
    {
        println!("meadow {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let options = match config::parse(&args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    if !options.quiet {
        for warning in &options.warnings {
            eprintln!("warning: {warning}");
        }
    }

    let result = match options.command {
        Command::Doc => doc(&options),
        Command::Stats => stats(&options),
        Command::List => list(&options),
        Command::Server => server::serve(options.clone())
            .map(|()| ExitCode::SUCCESS)
            .map_err(|error| Box::new(error) as Box<dyn std::error::Error>),
    };

    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn doc(options: &RunOptions) -> Result<ExitCode, Box<dyn std::error::Error>> {
    match options.markup.as_str() {
        "markdown" | "rdoc" => {}
        "textile" => return Err("Textile markup is not supported".into()),
        other => return Err(format!("unknown markup provider '{other}'").into()),
    }

    let build = cache::load_or_build(options)?;
    report_build_warnings(options, &build);

    let stats = render_site(&build.registry, &site_options(options)?)?;

    if !options.quiet {
        for warning in &stats.warnings {
            if !options.no_warn_unresolved {
                eprintln!("warning: {warning}");
            }
        }
        println!(
            "wrote {} pages ({} changed, {} unchanged) and {} search entries to {}",
            stats.pages,
            stats.pages_written,
            stats.pages_unchanged,
            stats.search_entries,
            options.output_dir.display()
        );
        if options.verbose > 0 {
            println!(
                "sources: {} files ({} parsed, {} reused, {} cache hit)",
                build.stats.files,
                build.stats.parsed,
                build.stats.reused,
                if build.stats.cache_hit { "yes" } else { "no" }
            );
        }
    }

    if options.fail_on_warning
        && (!build.warnings.is_empty()
            || !build.parser_diagnostics.is_empty()
            || !build.plugin_warnings.is_empty()
            || (!options.no_warn_unresolved
                && (!build.resolution_warnings.is_empty() || !stats.warnings.is_empty())))
    {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

fn stats(options: &RunOptions) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let build = cache::load_or_build(options)?;
    report_build_warnings(options, &build);
    let registry = &build.registry;

    let mut modules = 0;
    let mut classes = 0;
    let mut constants = 0;
    let mut methods = 0;
    let mut class_variables = 0;
    let mut attributes = BTreeSet::new();
    let mut total = 0;
    let mut documented = 0;
    let mut undocumented = Vec::new();

    for object in registry.objects() {
        match object.kind {
            ObjectKind::Root | ObjectKind::Macro => continue,
            ObjectKind::Module => modules += 1,
            ObjectKind::Class => classes += 1,
            ObjectKind::Constant => constants += 1,
            ObjectKind::Method => {
                if let KindData::Method { attr, alias_of, .. } = &object.extra {
                    if let Some(attr) = attr {
                        attributes.insert(attr.name.clone());
                    } else if alias_of.is_none() {
                        methods += 1;
                    }
                }
            }
            ObjectKind::ClassVariable => class_variables += 1,
            ObjectKind::InstanceVariable => {}
        }
        total += 1;
        if object.docstring.text.is_empty() && object.docstring.tags.is_empty() {
            undocumented.push(object);
        } else {
            documented += 1;
        }
    }

    let percent = if total == 0 {
        100.0
    } else {
        f64::from(documented) / f64::from(total) * 100.0
    };

    if !options.quiet {
        println!("Files:          {}", build.stats.files);
        println!("Modules:        {modules}");
        println!("Classes:        {classes}");
        println!("Constants:      {constants}");
        println!("Class variables:{class_variables}");
        println!("Methods:        {methods}");
        println!("Attributes:     {}", attributes.len());
        println!("Documented:     {documented}/{total} ({percent:.1}%)");
    }

    if options.list_undoc {
        for object in undocumented {
            println!("{}", object.path);
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn list(options: &RunOptions) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let build = cache::load_or_build(options)?;
    report_build_warnings(options, &build);
    let query = options.query.as_deref().map(Query::parse).transpose()?;

    let mut selected: Vec<&CodeObject> = build
        .registry
        .objects()
        .iter()
        .filter(|object| {
            object.kind != ObjectKind::Root
                && visible(object, options)
                && query.as_ref().is_none_or(|query| query.matches(object))
        })
        .collect();
    selected.sort_by(|left, right| left.path.cmp(&right.path));

    if options.format == "json" {
        let values: Vec<serde_json::Value> = selected
            .iter()
            .map(|object| {
                serde_json::json!({
                    "path": object.path,
                    "name": object.name,
                    "kind": kind_name(object.kind),
                    "visibility": visibility_name(object.visibility),
                    "namespace": object.namespace.map(|id| build.registry.get(id).path.to_string_lossy().into_owned()),
                    "tags": object.docstring.tags.iter().map(|tag| tag.tag.to_string_lossy().into_owned()).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        for object in selected {
            println!("{}", object.path);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn site_options(options: &RunOptions) -> Result<SiteOptions, Box<dyn std::error::Error>> {
    Ok(SiteOptions {
        output_dir: options.output_dir.clone(),
        title: options.title.clone(),
        readme: options.readme.clone(),
        main: options.main.clone(),
        files: options.files.clone(),
        template_path: options.template_path.clone(),
        project_root: std::env::current_dir()?,
        sanitize: options.sanitize,
        markup: options.markup.clone(),
        show_private: options.show_private,
        show_protected: options.show_protected,
        assets: options.assets.clone(),
        api: options.api.clone(),
        hide_api: options.hide_api,
        show_inherited: options.show_inherited,
        embed_mixins: options.embed_mixins,
        timestamps: options.timestamps,
        proxy_urls: load_proxy_urls(options),
        cache_dir: options.use_cache.then(|| options.db.clone()),
    })
}

fn report_build_warnings(options: &RunOptions, build: &cache::BuildResult) {
    if options.quiet {
        return;
    }
    for warning in &build.warnings {
        eprintln!(
            "warning: {}:{}: {}",
            warning.location.start_line, warning.location.start_line, warning.message
        );
    }
    for diagnostic in &build.parser_diagnostics {
        let level = match diagnostic.level {
            DiagnosticLevel::Error => "error",
            DiagnosticLevel::Warning => "warning",
        };
        eprintln!(
            "{}: {}: {}",
            diagnostic.file.to_string_lossy(),
            level,
            diagnostic.message.to_string_lossy()
        );
    }
    for warning in &build.plugin_warnings {
        eprintln!("warning: {warning}");
    }
    if !options.no_warn_unresolved {
        for warning in &build.resolution_warnings {
            let target = warning
                .object
                .map(|id| build.registry.get(id).path.to_string_lossy().into_owned())
                .unwrap_or_default();
            eprintln!("warning: {} ({target})", warning.message);
        }
    }
}

fn load_proxy_urls(options: &RunOptions) -> BTreeMap<String, String> {
    let Some(path) = &options.proxy_cache else {
        return BTreeMap::new();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn visible(object: &CodeObject, options: &RunOptions) -> bool {
    match object.visibility {
        meadow_model::Visibility::Private => options.show_private,
        meadow_model::Visibility::Protected => options.show_protected,
        meadow_model::Visibility::Public => true,
    }
}

fn kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Root => "root",
        ObjectKind::Module => "module",
        ObjectKind::Class => "class",
        ObjectKind::Method => "method",
        ObjectKind::Constant => "constant",
        ObjectKind::ClassVariable => "class_variable",
        ObjectKind::InstanceVariable => "instance_variable",
        ObjectKind::Macro => "macro",
    }
}

fn visibility_name(visibility: meadow_model::Visibility) -> &'static str {
    match visibility {
        meadow_model::Visibility::Public => "public",
        meadow_model::Visibility::Protected => "protected",
        meadow_model::Visibility::Private => "private",
    }
}
