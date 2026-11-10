//! Built-in plugin tests.

use std::fs;
use std::path::{Path, PathBuf};

use meadow_handlers::build_registry_with_plugins;
use meadow_model::{KindData, ObjectKind, Registry};
use meadow_parser::parse_source;
use meadow_plugins::{ActiveRecord, Plugin, builtin_plugins};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn registry(name: &str, plugins: &[&str]) -> Registry {
    registry_files(&[name], plugins)
}

fn registry_files(names: &[&str], plugins: &[&str]) -> Registry {
    let facts = names
        .iter()
        .map(|name| {
            let relative = format!("tests/fixtures/{name}");
            let source =
                fs::read_to_string(workspace_root().join(&relative)).expect("read fixture");
            parse_source(relative, &source)
        })
        .collect::<Vec<_>>();
    let plugins = builtin_plugins(
        &plugins
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>(),
        &workspace_root().join("tests/fixtures"),
    );
    build_registry_with_plugins(&facts, plugins)
}

fn tag_types(registry: &Registry, path: &str, tag: &str, name: Option<&str>) -> Vec<String> {
    let object = registry
        .find(path)
        .unwrap_or_else(|| panic!("missing {path}"));
    object
        .docstring
        .tags
        .iter()
        .filter(|candidate| {
            candidate.tag == tag
                && name.is_none_or(|name| {
                    candidate
                        .name
                        .as_ref()
                        .is_some_and(|candidate| candidate == name)
                })
        })
        .filter_map(|candidate| candidate.types.as_ref().map(ToString::to_string))
        .collect()
}

#[test]
fn sorbet_synthesizes_tags_and_types() {
    let registry = registry("sorbet.rb", &["sorbet"]);

    assert_eq!(
        tag_types(&registry, "SorbetExample#tagged", "param", Some("name")),
        vec!["String"]
    );
    assert_eq!(
        tag_types(&registry, "SorbetExample#tagged", "param", Some("age")),
        vec!["Integer, nil"]
    );
    assert_eq!(
        tag_types(&registry, "SorbetExample#tagged", "return", None),
        vec!["Array<String>"]
    );

    let booleans = registry.find("SorbetExample#booleans").expect("booleans");
    assert!(booleans.docstring.tags.iter().any(|tag| {
        tag.tag == "return"
            && tag
                .types
                .as_ref()
                .is_some_and(|types| types.to_string() == "String, Symbol")
    }));
    assert!(booleans.docstring.tags.iter().any(|tag| {
        tag.tag == "param"
            && tag
                .types
                .as_ref()
                .is_some_and(|types| types.to_string() == "true, false")
    }));

    let abstract_method = registry
        .find("SorbetExample#abstract_method")
        .expect("abstract");
    assert!(
        abstract_method
            .docstring
            .tags
            .iter()
            .any(|tag| tag.tag == "abstract")
    );
    let final_method = registry.find("SorbetExample#final_method").expect("final");
    assert!(
        final_method
            .docstring
            .tags
            .iter()
            .any(|tag| tag.tag == "final")
    );
    assert!(
        final_method
            .docstring
            .tags
            .iter()
            .any(|tag| tag.tag == "override")
    );

    assert_eq!(
        tag_types(
            &registry,
            "SorbetExample#with_yield",
            "yieldparam",
            Some("item")
        ),
        vec!["String"]
    );
    assert_eq!(
        tag_types(&registry, "SorbetExample#with_yield", "yieldreturn", None),
        vec!["Integer"]
    );

    assert_eq!(
        tag_types(&registry, "SorbetExample::CONST", "return", None),
        vec!["String"]
    );

    assert_eq!(
        tag_types(&registry, "SorbetExample#explicit", "param", Some("name")),
        vec!["Symbol"]
    );
    assert_eq!(
        tag_types(&registry, "SorbetExample#explicit", "return", None),
        vec!["String"]
    );

    let value = registry
        .find("SorbetExample#@value")
        .expect("typed instance variable");
    assert_eq!(value.kind, meadow_model::ObjectKind::InstanceVariable);
    assert_eq!(
        tag_types(&registry, "SorbetExample#@value", "return", None),
        vec!["Integer"]
    );
}

#[test]
fn sorbet_rbi_files_augment_source_types() {
    let registry = registry_files(&["sorbet_rbi.rb", "sorbet_rbi.rbi"], &["sorbet"]);

    let greet = registry.find("RbiExample#greet").expect("greet");
    assert_eq!(greet.docstring.text, "Original source docs.");
    assert_eq!(
        tag_types(&registry, "RbiExample#greet", "param", Some("name")),
        vec!["Symbol"]
    );
    assert_eq!(
        tag_types(&registry, "RbiExample#greet", "return", None),
        vec!["Symbol"]
    );
    assert!(
        greet.docstring.tags.iter().any(|tag| tag.tag == "version"),
        "{:?}",
        greet.docstring.tags
    );

    assert_eq!(
        tag_types(&registry, "RbiExample#undocumented", "param", Some("value")),
        vec!["Integer"]
    );
    assert_eq!(
        tag_types(&registry, "RbiExample#undocumented", "return", None),
        vec!["Integer"]
    );

    assert_eq!(
        tag_types(&registry, "RbiExample.toggle", "param", Some("flag")),
        vec!["true, false"]
    );
    assert_eq!(
        tag_types(&registry, "RbiExample.toggle", "return", None),
        vec!["true, false"]
    );

    assert_eq!(
        tag_types(&registry, "RbiExample::RBI_CONST", "return", None),
        vec!["Integer"]
    );

    let shimmed = registry.find("RbiOnly#shimmed").expect("shimmed");
    assert_eq!(
        tag_types(&registry, "RbiOnly#shimmed", "return", None),
        vec!["String"]
    );
    assert!(
        !shimmed.docstring.text.contains("pkg:gem/"),
        "{}",
        shimmed.docstring.text
    );
}

#[test]
fn expected_plugin_registries() {
    for (fixture, plugin, golden) in [
        ("sorbet.rb", "sorbet", "sorbet.json"),
        ("activerecord.rb", "activerecord", "activerecord.json"),
    ] {
        let registry = registry(fixture, &[plugin]);
        let actual = format!(
            "{}\n",
            serde_json::to_string_pretty(&registry).expect("serialize registry")
        );
        let path = workspace_root().join("tests/expected").join(golden);
        if std::env::var_os("UPDATE_EXPECTED").is_some() {
            fs::write(&path, &actual).expect("write expected registry");
        }
        let expected = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "missing expected registry {}: {error}. Run UPDATE_EXPECTED=1 cargo test -p meadow-handlers expected_plugin_registries",
                path.display()
            )
        });
        assert_eq!(actual, expected, "registry mismatch for {fixture}");
    }
}

#[test]
fn sorbet_struct_fields_become_attributes() {
    let registry = registry("sorbet.rb", &["sorbet"]);

    let x = registry.find("Point#x").expect("x reader");
    assert_eq!(x.kind, ObjectKind::Method);
    assert_eq!(
        tag_types(&registry, "Point#x", "return", None),
        vec!["Integer"]
    );
    assert!(registry.find("Point#x=").is_none());

    let label = registry.find("Point#label").expect("label reader");
    assert_eq!(
        tag_types(&registry, "Point#label", "return", None),
        vec!["String, nil"]
    );
    assert!(registry.find("Point#label=").is_some());
    let _ = label;
}

#[test]
fn activerecord_supports_custom_schema_paths_and_structure_warnings() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("meadow-ar-{}-{stamp}", std::process::id()));
    fs::create_dir_all(root.join("db")).expect("create db dir");
    let schema = root.join("db/custom_schema.rb");
    fs::write(
        &schema,
        "ActiveRecord::Schema.define do\n  create_table \"widgets\" do |t|\n    t.string \"name\", null: false\n  end\nend\n",
    )
    .expect("write schema");

    let source = "class Widget < ApplicationRecord; end\n";
    let facts = parse_source("widget.rb", source);
    let plugins: Vec<Box<dyn Plugin>> = vec![Box::new(ActiveRecord::with_options(
        &root,
        Some(schema),
        Vec::new(),
        std::collections::BTreeMap::new(),
    ))];
    let registry = meadow_handlers::build_registry_with_plugins(&[facts], plugins);
    assert_eq!(
        tag_types(&registry, "Widget#name", "return", None),
        vec!["String"]
    );

    fs::write(root.join("db/structure.sql"), "CREATE TABLE widgets ();\n")
        .expect("write structure");
    let plugin = ActiveRecord::new(&root);
    let mut empty = meadow_model::Registry::new();
    plugin.after_handle(&mut empty);
    assert!(
        plugin
            .warnings()
            .iter()
            .any(|warning| warning.contains("structure.sql"))
    );
}

#[test]
fn activerecord_generates_associations_scopes_enums_and_columns() {
    let registry = registry("activerecord.rb", &["activerecord"]);

    for path in [
        "User#posts",
        "User#posts=",
        "User#posts_ids",
        "User#posts_ids=",
        "User#account",
        "User#account=",
        "User#profile",
        "User#profile=",
        "User.active",
        "User#active?",
        "User#active!",
        "User.active",
        "User#role_admin?",
        "User#role_admin!",
        "User.role_admin",
        "User#slow?",
        "User.fast",
        "User#status",
        "User.statuses",
        "User#name",
        "User#name=",
        "User#age",
        "User#settings",
    ] {
        assert!(registry.find(path).is_some(), "missing {path}");
    }

    assert_eq!(
        tag_types(&registry, "User#account", "return", None),
        vec!["Account"]
    );
    let active = registry.find("User.active").expect("scope");
    let KindData::Method { scope, params, .. } = &active.extra else {
        panic!("scope method");
    };
    assert_eq!(*scope, meadow_model::Scope::Class);
    assert_eq!(params.len(), 1);

    assert_eq!(
        tag_types(&registry, "User#age", "return", None),
        vec!["Integer"]
    );
    assert_eq!(
        tag_types(&registry, "User#name", "return", None),
        vec!["String, nil"]
    );
    assert_eq!(
        tag_types(&registry, "User#admin", "return", None),
        vec!["true, false, nil"]
    );
}
