//! Full-site rendering tests.

use std::fs;
use std::path::{Path, PathBuf};

use meadow_handlers::build_registry;
use meadow_html::{SiteOptions, render_site};
use meadow_parser::{FileFacts, parse_source};

const FIXTURES: &[&str] = &[
    "core.rb",
    "tags.rb",
    "handlers.rb",
    "directives.rb",
    "links.rb",
    "rdoc.rb",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn registry() -> meadow_model::Registry {
    let facts: Vec<FileFacts> = FIXTURES
        .iter()
        .map(|name| {
            let relative = format!("tests/fixtures/{name}");
            let source =
                fs::read_to_string(workspace_root().join(&relative)).expect("read fixture");
            parse_source(relative, &source)
        })
        .collect();
    build_registry(&facts)
}

fn output_dir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("meadow-html-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    directory
}

fn options(output_dir: PathBuf) -> SiteOptions {
    SiteOptions {
        output_dir,
        title: "Meadow Test".to_string(),
        readme: Some(PathBuf::from("README.md")),
        main: None,
        files: Vec::new(),
        template_path: None,
        project_root: workspace_root(),
        sanitize: false,
        ..SiteOptions::default()
    }
}

#[test]
fn renders_a_complete_site() {
    let registry = registry();
    let directory = output_dir("complete");
    let stats = render_site(&registry, &options(directory.clone())).expect("render site");

    assert!(stats.pages > 10);
    assert!(stats.search_entries > 20);

    for file in [
        "index.html",
        "class_list.html",
        "method_list.html",
        "file_list.html",
        "undocumented.html",
        "top_level.html",
        "class-person.html",
        "class-tagged.html",
        "class-derived.html",
        "file-readme-md.html",
        "search.json",
        "style.css",
        "search.js",
    ] {
        assert!(directory.join(file).is_file(), "missing {file}");
    }

    let person = fs::read_to_string(directory.join("class-person.html")).expect("read Person page");
    assert!(person.contains("Person"));
    assert!(person.contains("initialize"));
    assert!(person.contains("Builds a person."));
    assert!(person.contains("id=\"method-instance-initialize\""));

    let derived =
        fs::read_to_string(directory.join("class-derived.html")).expect("read Derived page");
    assert!(derived.contains("class-base.html"));
    assert!(derived.contains("inherited_docs"));

    let methods = fs::read_to_string(directory.join("method_list.html")).expect("read method list");
    assert!(methods.contains("Person#initialize"));

    let index = fs::read_to_string(directory.join("index.html")).expect("read index");
    assert!(index.contains("Meadow Test"));

    let search = fs::read_to_string(directory.join("search.json")).expect("read search index");
    let search: serde_json::Value = serde_json::from_str(&search).expect("parse search index");
    let entries = search.as_array().expect("search array");
    assert!(entries.iter().any(|entry| entry["path"] == "Person"));
    assert!(entries.iter().any(|entry| entry["kind"] == "file"));
}

fn tree(directory: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    for entry in fs::read_dir(directory).expect("read output dir") {
        let entry = entry.expect("entry");
        if entry.path().is_file() {
            files.insert(entry.path(), fs::read(entry.path()).expect("read file"));
        }
    }
    files
}

#[test]
fn renders_deterministically() {
    let registry = registry();
    let first = output_dir("deterministic-a");
    let second = output_dir("deterministic-b");
    render_site(&registry, &options(first.clone())).expect("render first");
    render_site(&registry, &options(second.clone())).expect("render second");

    let first_tree = tree(&first);
    let second_tree = tree(&second);
    assert_eq!(first_tree.keys().count(), second_tree.keys().count());
    for (path, contents) in &first_tree {
        let name = path.file_name().expect("file name");
        let other = second.join(name);
        assert_eq!(
            contents,
            &fs::read(other).expect("read second"),
            "{}",
            path.display()
        );
    }
}

#[test]
fn matches_the_expected_person_page() {
    let registry = registry();
    let directory = output_dir("golden-page");
    render_site(&registry, &options(directory.clone())).expect("render site");
    let page = fs::read_to_string(directory.join("class-person.html")).expect("read page");
    let normalized = page.split_whitespace().collect::<Vec<_>>().join(" ");
    let path = workspace_root().join("tests/expected/class-person.html");
    if std::env::var_os("UPDATE_EXPECTED").is_some() {
        fs::write(&path, format!("{normalized}\n")).expect("write page golden");
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing page golden {}: {error}. Run UPDATE_EXPECTED=1 cargo test -p meadow-html matches_the_expected_person_page",
            path.display()
        )
    });
    assert_eq!(format!("{normalized}\n"), expected);
}

#[test]
fn renders_rdoc_docstrings_when_selected() {
    let registry = registry();
    let directory = output_dir("rdoc");
    let mut options = options(directory.clone());
    options.markup = "rdoc".to_string();
    render_site(&registry, &options).expect("render rdoc site");
    let page = fs::read_to_string(directory.join("class-rdocexample.html")).expect("read page");
    assert!(page.contains("<h1>RDoc Class</h1>"), "{page}");
    assert!(page.contains("<strong>bold</strong>"));
    assert!(page.contains("<code>code</code>"));
    assert!(page.contains("<dl><dt>name</dt><dd>The name.</dd>"));
    assert!(page.contains("<pre><code>verbatim line</code></pre>"));
}

#[test]
fn honors_per_docstring_markup() {
    let registry = registry();
    let directory = output_dir("magic-markup");
    render_site(&registry, &options(directory.clone())).expect("render site");
    let page = fs::read_to_string(directory.join("class-rdocexample.html")).expect("read page");
    assert!(page.contains("<h1>Magic Heading</h1>"), "{page}");
    assert!(page.contains("<strong>bold</strong>"));
}

#[test]
fn renders_only_changed_pages_incrementally() {
    let registry = registry();
    let directory = output_dir("page-cache");
    let cache = output_dir("page-cache-db");
    let mut options = options(directory.clone());
    options.cache_dir = Some(cache);
    let first = render_site(&registry, &options).expect("first render");
    assert!(first.pages_written > 0);
    let second = render_site(&registry, &options).expect("second render");
    assert_eq!(second.pages_written, 0);
    assert_eq!(second.pages_unchanged, first.pages);
}

#[test]
fn rewrites_pages_whose_contents_were_lost() {
    let registry = registry();
    let directory = output_dir("page-cache-stale");
    let cache = output_dir("page-cache-stale-db");
    let mut options = options(directory.clone());
    options.cache_dir = Some(cache);
    render_site(&registry, &options).expect("first render");

    fs::write(directory.join("class_list.html"), "").expect("truncate page");

    let second = render_site(&registry, &options).expect("second render");
    assert!(second.pages_written > 0);
    let page = fs::read_to_string(directory.join("class_list.html")).expect("read page");
    assert!(page.contains("class-person.html"), "{page}");
}

#[test]
fn supports_inheritance_and_mixin_display_options() {
    let registry = registry();
    let directory = output_dir("inheritance-options");
    let mut inherited_options = options(directory.clone());
    inherited_options.show_inherited = false;
    render_site(&registry, &inherited_options).expect("render without inherited");
    let page = fs::read_to_string(directory.join("class-person.html")).expect("read page");
    assert!(!page.contains("inherited from"));

    let mixin_directory = output_dir("mixins-embedded");
    let mut mixin_options = options(mixin_directory.clone());
    mixin_options.embed_mixins = true;
    render_site(&registry, &mixin_options).expect("render with mixins");
    let page = fs::read_to_string(mixin_directory.join("class-person.html")).expect("read page");
    assert!(page.contains("hello"), "{page}");
}

#[test]
fn emits_timestamps_when_requested() {
    let registry = registry();
    let directory = output_dir("timestamps");
    let mut options = options(directory.clone());
    options.timestamps = true;
    render_site(&registry, &options).expect("render site");
    let page = fs::read_to_string(directory.join("index.html")).expect("read page");
    assert!(page.contains("Unix timestamp"));
}

#[test]
fn supports_template_and_asset_overrides() {
    let registry = registry();
    let directory = output_dir("override");
    let template_dir = output_dir("templates");
    fs::create_dir_all(&template_dir).expect("create template dir");
    fs::write(
        template_dir.join("class.html"),
        "<!doctype html><title>OVERRIDE</title><h1>{{ object.path }}</h1>",
    )
    .expect("write template");
    fs::write(template_dir.join("style.css"), "/* OVERRIDE CSS */").expect("write css");

    let mut options = options(directory.clone());
    options.template_path = Some(template_dir);

    render_site(&registry, &options).expect("render overridden site");
    let person = fs::read_to_string(directory.join("class-person.html")).expect("read Person page");
    assert!(person.contains("OVERRIDE"));
    assert!(person.contains("Person"));

    let css = fs::read_to_string(directory.join("style.css")).expect("read css");
    assert!(css.contains("OVERRIDE CSS"));
}

#[test]
fn unresolved_constants_render_without_broken_links() {
    let registry = registry();
    let directory = output_dir("unresolved-constants");
    render_site(&registry, &options(directory.clone())).expect("render site");
    let page = fs::read_to_string(directory.join("class-person.html")).expect("read Person page");
    assert!(page.contains("<p>Object</p>"), "{page}");
    assert!(!page.contains("href=\"None\""), "{page}");
}

#[test]
fn unresolved_constants_use_proxy_urls_when_available() {
    let registry = registry();
    let directory = output_dir("proxy-constants");
    let mut options = options(directory.clone());
    options.proxy_urls.insert(
        "Object".to_string(),
        "https://example.com/object".to_string(),
    );
    render_site(&registry, &options).expect("render site");
    let page = fs::read_to_string(directory.join("class-person.html")).expect("read Person page");
    assert!(page.contains("example.com"), "{page}");
    assert!(page.contains(">Object</a>"), "{page}");
}

#[test]
fn top_level_constants_link_to_the_top_level_page() {
    let facts = vec![parse_source(
        "toplevel.rb",
        "TOP = 1\nclass Rooted < TOP\nend\n",
    )];
    let registry = build_registry(&facts);
    let directory = output_dir("top-level-constants");
    render_site(&registry, &options(directory.clone())).expect("render site");

    let page = fs::read_to_string(directory.join("class-rooted.html")).expect("read Rooted page");
    assert!(
        page.contains("<a href=\"top_level.html#constant-top\">TOP</a>"),
        "{page}"
    );

    let top_level = fs::read_to_string(directory.join("top_level.html")).expect("read top level");
    assert!(top_level.contains("id=\"constant-top\""), "{top_level}");
}
