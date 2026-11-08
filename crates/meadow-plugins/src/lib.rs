//! Plugin trait and built-in plugins for meadow.

#![deny(missing_docs)]

mod activerecord;
mod sorbet;
mod util;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use meadow_model::{CodeObject, ObjectId, Registry, SourceText};
use meadow_parser::Event;

pub use activerecord::ActiveRecord;
pub use sorbet::Sorbet;

/// Whether a plugin fully handled an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    /// Stop default handling.
    Yes,
    /// Let the default handler process the event.
    No,
}

/// Context available while a plugin observes an event.
#[derive(Debug, Clone, Copy)]
pub struct PluginContext<'a> {
    /// File currently being handled.
    pub file: &'a SourceText,
    /// Active namespace id.
    pub namespace: ObjectId,
}

/// A compiled-in documentation plugin.
pub trait Plugin: Send + Sync {
    /// Stable plugin name.
    fn name(&self) -> &'static str;

    /// Observes an event before default handling.
    fn on_event(&self, _event: &Event, _context: &PluginContext<'_>) -> Handled {
        Handled::No
    }

    /// Runs after all files have been handled.
    fn after_handle(&self, _registry: &mut Registry) {}

    /// Transforms one object after resolution.
    fn transform_tags(&self, _object: &mut CodeObject) {}

    /// Returns warnings emitted by the plugin.
    fn warnings(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Configuration for built-in plugins.
#[derive(Debug, Clone, Default)]
pub struct PluginSettings {
    /// Override path for `db/schema.rb`.
    pub activerecord_schema: Option<PathBuf>,
    /// `ActiveRecord` model superclass names.
    pub activerecord_models: Vec<String>,
    /// Inflector irregular overrides.
    pub irregulars: BTreeMap<String, String>,
}

/// Creates built-in plugins selected by name.
#[must_use]
pub fn builtin_plugins(names: &[String], project_root: &Path) -> Vec<Box<dyn Plugin>> {
    builtin_plugins_with(names, project_root, &PluginSettings::default())
}

/// Creates built-in plugins with explicit settings.
#[must_use]
pub fn builtin_plugins_with(
    names: &[String],
    project_root: &Path,
    settings: &PluginSettings,
) -> Vec<Box<dyn Plugin>> {
    let mut plugins: Vec<Box<dyn Plugin>> = Vec::new();
    for name in names {
        match name.as_str() {
            "sorbet" => plugins.push(Box::new(Sorbet::default())),
            "activerecord" => plugins.push(Box::new(ActiveRecord::with_options(
                project_root,
                settings.activerecord_schema.clone(),
                settings.activerecord_models.clone(),
                settings.irregulars.clone(),
            ))),
            _ => {}
        }
    }
    plugins
}
