//! `RDoc` markup subset tests.

use std::fs;
use std::path::Path;

use meadow_markup::{Markup, RDoc};

#[test]
fn renders_the_rdoc_subset() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/rdoc/example.rdoc");
    let source = fs::read_to_string(fixture).expect("read rdoc fixture");
    let html = RDoc::default().render(&source);

    assert!(html.contains("<h1>RDoc Example</h1>"));
    assert!(html.contains("<h2>Lists</h2>"));
    assert!(html.contains("<strong>bold</strong>"));
    assert!(html.contains("<em>emphasis</em>"));
    assert!(html.contains("<code>code</code>"));
    assert!(html.contains("<code>tt code</code>"));
    assert!(html.contains("<ul><li>first</li><li>second</li></ul>"));
    assert!(html.contains("<ol><li>one</li><li>two</li></ol>"));
    assert!(html.contains("<pre><code>verbatim line 1"));
    assert!(html.contains("<dl><dt>term</dt><dd>definition text</dd>"));
    assert!(html.contains("<dt>other</dt><dd>second definition</dd>"));
    assert!(html.contains("<a href=\"https://example.com\">Example</a>"));
}

#[test]
fn markdown_code_fences_are_syntax_highlighted() {
    let html = meadow_markup::render_markdown("```ruby\nputs 1\n```");
    assert!(html.contains("language-ruby"), "{html}");
    assert!(html.contains("puts"), "{html}");
}

#[test]
fn markdown_can_escape_raw_html() {
    let safe = meadow_markup::Markdown::new(true).render("<script>x</script>");
    assert!(!safe.contains("<script>"), "{safe}");
    let unsafe_html = meadow_markup::Markdown::new(false).render("<b>x</b>");
    assert!(unsafe_html.contains("<b>x</b>"), "{unsafe_html}");
}

#[test]
fn unsupported_rdoc_degrades_without_panicking() {
    let html = RDoc::default().render("{{{ weird }}} { :not_a_list }\n= =\n");
    assert!(html.contains("weird"));
}
