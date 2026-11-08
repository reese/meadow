//! RBS-to-Ruby registry merging tests.

use std::path::{Path, PathBuf};

use meadow_handlers::build_registry;
use meadow_model::{KindData, ObjectKind, Scope, Visibility};
use meadow_parser::{FileFacts, parse_file};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/rbs")
        .join(name)
}

fn facts() -> Vec<FileFacts> {
    vec![
        parse_file(fixture("core.rb")).expect("parse Ruby fixture"),
        parse_file(fixture("core.rbs")).expect("parse RBS fixture"),
    ]
}

fn method<'a>(registry: &'a meadow_model::Registry, path: &str) -> &'a meadow_model::CodeObject {
    let object = registry
        .find(path)
        .unwrap_or_else(|| panic!("missing object {path}"));
    assert_eq!(object.kind, ObjectKind::Method, "not a method: {path}");
    object
}

fn tags(object: &meadow_model::CodeObject) -> Vec<&str> {
    object
        .docstring
        .tags
        .iter()
        .map(|tag| tag.tag.as_str().unwrap_or_default())
        .collect()
}

#[test]
fn merges_rbs_types_into_existing_methods() {
    let registry = build_registry(&facts());

    let name = method(&registry, "Person#name");
    assert_eq!(name.docstring.text, "The person's name.");
    assert!(tags(name).contains(&"return"));
    let return_tag = name
        .docstring
        .tags
        .iter()
        .find(|tag| tag.tag == "return")
        .expect("return tag");
    assert_eq!(
        return_tag
            .types_source
            .as_ref()
            .expect("return type")
            .to_string_lossy(),
        "String"
    );

    let initialize = method(&registry, "Person#initialize");
    let text = &initialize.docstring.text;
    assert_eq!(text, "Create a person.");
    assert!(initialize.docstring.tags.iter().any(|tag| {
        tag.tag == "param"
            && tag
                .name
                .as_ref()
                .is_some_and(|name| name.as_bytes() == b"age")
    }));
    assert!(
        initialize
            .docstring
            .tags
            .iter()
            .any(|tag| tag.tag == "return")
    );
    let KindData::Method { signature, .. } = &initialize.extra else {
        panic!("method data");
    };
    assert_eq!(signature, "initialize(name, age = 0)");
}

#[test]
fn adds_rbs_only_objects() {
    let registry = build_registry(&facts());

    let age = method(&registry, "Person#age");
    assert!(
        age.docstring
            .tags
            .iter()
            .any(|tag| tag.tag == "return" && tag.types_source.is_some())
    );

    let create = method(&registry, "Person.create");
    let KindData::Method { scope, .. } = &create.extra else {
        panic!("method data");
    };
    assert_eq!(*scope, Scope::Class);

    assert!(registry.find("Person::VERSION").is_some());
    assert!(registry.find("Person#@cache").is_some());
    assert!(registry.find("Person#@@count").is_some());
    assert!(registry.find("Person#nickname").is_some());
}

#[test]
fn merges_mixins_and_visibility() {
    let registry = build_registry(&facts());
    let person = registry.find("Person").expect("Person");
    let KindData::Namespace {
        includes,
        extends,
        prepends,
        ..
    } = &person.extra
    else {
        panic!("namespace data");
    };
    assert_eq!(includes.len(), 1);
    assert_eq!(extends.len(), 1);
    assert_eq!(prepends.len(), 1);

    let secret = method(&registry, "Person#secret");
    assert_eq!(secret.visibility, Visibility::Private);
}

#[test]
fn keeps_ruby_docstring_tags_when_rbs_adds_overloads() {
    let registry = build_registry(&facts());
    let greet = method(&registry, "Person#greet");
    assert_eq!(greet.docstring.text, "Greet someone.");
    let tags = &greet.docstring.tags;
    assert_eq!(tags.iter().filter(|tag| tag.tag == "overload").count(), 2);
    assert!(
        tags.iter()
            .any(|tag| tag.tag == "return" && tag.text == "existing return description")
    );
}
