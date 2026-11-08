//! Resolution and link expansion tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use meadow_handlers::build_registry;
use meadow_model::{KindData, ObjectId, Ref, Registry, SourceText};
use meadow_parser::{FileFacts, parse_source};
use meadow_resolve::{LinkTargetResolver, expand_links, resolve_registry};

const FIXTURES: &[&str] = &["links.rb", "core.rb"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn registry() -> Registry {
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

struct FakeResolver {
    files: BTreeMap<String, Vec<u8>>,
    proxies: BTreeMap<String, String>,
}

impl LinkTargetResolver for FakeResolver {
    fn object_url(&self, _registry: &Registry, id: ObjectId) -> String {
        format!("/objects/{}", id.0)
    }

    fn file_url(&self, path: &str) -> String {
        format!("/files/{path}")
    }

    fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }

    fn proxy_url(&self, name: &str) -> Option<String> {
        self.proxies.get(name).cloned()
    }
}

fn resolver() -> FakeResolver {
    let mut files = BTreeMap::new();
    files.insert("README.md".to_string(), b"Read me intro.".to_vec());
    FakeResolver {
        files,
        proxies: BTreeMap::new(),
    }
}

#[test]
fn resolves_superclasses_and_ancestors() {
    let mut registry = registry();
    let _ = resolve_registry(&mut registry);

    let derived = registry.find("Derived").expect("Derived");
    let KindData::Namespace {
        superclass,
        ancestors,
        ..
    } = &derived.extra
    else {
        panic!("Derived namespace");
    };
    assert!(matches!(superclass, Some(Ref::Resolved(_))));
    assert_ne!(ancestors.len(), 0);
    assert_eq!(registry.get(ancestors[0]).path, "Base");
}

#[test]
fn copies_see_references() {
    let registry = registry();
    let inherited = registry
        .find("Derived#inherited_docs")
        .expect("inherited_docs");
    assert_eq!(inherited.docstring.text, "Base method docs.");
    assert_eq!(inherited.docstring.tags[0].tag, "return");
}

#[test]
fn expands_object_url_and_file_links() {
    let registry = registry();
    let linked = registry.find("Derived#linked").expect("linked");
    let base = registry.find("Base#base_method").expect("base method");
    let expanded = expand_links(
        &linked.docstring.text,
        linked.namespace.expect("namespace"),
        &registry,
        &resolver(),
    );

    let text = expanded.text.to_string_lossy();
    assert!(text.contains(&format!("[links](/objects/{})", base.id.0)));
    assert!(text.contains(&format!("[Base#base_method](/objects/{})", base.id.0)));
    assert!(text.contains("[Example](https://example.com)"));
    assert!(text.contains("[File](/files/README.md#intro)"));
    assert!(text.contains("`{Base#base_method}` stays"));
    assert!(text.contains("Base method docs."));
    assert!(expanded.warnings.is_empty(), "{:?}", expanded.warnings);
}

#[test]
fn resolves_external_proxy_urls_from_cache() {
    let registry = registry();
    let broken = registry.find("Missing#broken").expect("broken");
    let mut resolver = resolver();
    resolver.proxies.insert(
        "NoSuchThing#nope".to_string(),
        "https://example.com/nope".to_string(),
    );
    let expanded = expand_links(
        &broken.docstring.text,
        broken.namespace.expect("namespace"),
        &registry,
        &resolver,
    );
    assert_eq!(
        expanded.text.to_string_lossy(),
        "See [here](https://example.com/nope)."
    );
    assert_eq!(expanded.warnings.len(), 0);
}

#[test]
fn expands_file_includes_and_warns_on_unresolved_links() {
    let registry = registry();
    let broken = registry.find("Missing#broken").expect("broken");
    let expanded = expand_links(
        &SourceText::from_bytes(b"Before {include:file:README.md} after {file:doc.txt}"),
        broken.namespace.expect("namespace"),
        &registry,
        &resolver(),
    );
    assert_eq!(
        expanded.text.to_string_lossy(),
        "Before Read me intro. after [doc.txt](/files/doc.txt)"
    );

    let missing = expand_links(
        &broken.docstring.text,
        broken.namespace.expect("namespace"),
        &registry,
        &resolver(),
    );
    assert_eq!(missing.text, "See here.");
    assert_eq!(missing.warnings.len(), 1);
    assert!(missing.warnings[0].contains("Cannot resolve link"));
}
