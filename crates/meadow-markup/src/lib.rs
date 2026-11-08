//! Markup providers for meadow.
//!
//! Markdown is the default provider. The trait keeps `RDoc` and other providers
//! pluggable at a later milestone.

#![deny(missing_docs)]

use std::sync::OnceLock;

use comrak::options::Plugins;
use comrak::plugins::syntect::SyntectAdapter;
use comrak::{Options, markdown_to_html_with_plugins};

static SYNTECT: OnceLock<SyntectAdapter> = OnceLock::new();

pub use rdoc::RDoc;

mod rdoc;

/// A provider that turns docstring text into HTML.
pub trait Markup {
    /// Renders `text` to HTML.
    fn render(&self, text: &str) -> String;
}

/// GitHub-flavored Markdown rendering via `comrak`.
#[derive(Debug, Clone, Copy)]
pub struct Markdown {
    /// Whether raw HTML in the source should be escaped.
    pub sanitize: bool,
}

impl Markdown {
    /// Creates a Markdown provider.
    #[must_use]
    pub const fn new(sanitize: bool) -> Self {
        Self { sanitize }
    }
}

impl Default for Markdown {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Markup for Markdown {
    fn render(&self, text: &str) -> String {
        let mut options = Options::default();
        options.extension.table = true;
        options.extension.strikethrough = true;
        options.extension.autolink = true;
        options.extension.tasklist = true;
        options.extension.footnotes = true;
        options.render.r#unsafe = !self.sanitize;
        let adapter = SYNTECT.get_or_init(|| SyntectAdapter::new(None));
        let mut plugins = Plugins::default();
        plugins.render.codefence_syntax_highlighter = Some(adapter);
        markdown_to_html_with_plugins(text, &options, &plugins)
    }
}

/// Renders Markdown with the default options.
#[must_use]
pub fn render_markdown(text: &str) -> String {
    Markdown::default().render(text)
}
