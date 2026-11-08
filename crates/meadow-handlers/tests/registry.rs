//! Registry construction tests for the M1 handler set.

use std::fs;
use std::path::{Path, PathBuf};

use meadow_handlers::{build_registry, build_registry_with_warnings};
use meadow_model::{KindData, MixinKind, ObjectKind, Ref, Scope, Visibility};
use meadow_parser::{FileFacts, parse_source, parse_source_bytes};

const FIXTURES: &[&str] = &[
    "core.rb",
    "docstrings.rb",
    "parameters.rb",
    "calls.rb",
    "handlers.rb",
    "tags.rb",
    "directives.rb",
    "links.rb",
    "syntax_error.rb",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn parse_fixtures() -> Vec<FileFacts> {
    FIXTURES
        .iter()
        .map(|name| {
            let relative = format!("tests/fixtures/{name}");
            let source =
                fs::read_to_string(workspace_root().join(&relative)).expect("read fixture");
            parse_source(relative, &source)
        })
        .collect()
}

fn method<'a>(registry: &'a meadow_model::Registry, path: &str) -> &'a meadow_model::CodeObject {
    let object = registry
        .find(path)
        .unwrap_or_else(|| panic!("missing object {path}"));
    assert_eq!(object.kind, ObjectKind::Method, "not a method: {path}");
    object
}

fn method_data(object: &meadow_model::CodeObject) -> (&Scope, &meadow_model::KindData) {
    let KindData::Method { scope, .. } = &object.extra else {
        panic!("not method data");
    };
    (scope, &object.extra)
}

#[test]
fn expected_registry_output() {
    let registry = build_registry(&parse_fixtures());
    let actual = format!(
        "{}\n",
        serde_json::to_string_pretty(&registry).expect("serialize registry")
    );
    let expected_path = workspace_root().join("tests/expected/registry.json");

    if std::env::var_os("UPDATE_EXPECTED").is_some() {
        fs::write(&expected_path, &actual).expect("write expected registry");
    }

    let expected = fs::read_to_string(&expected_path).unwrap_or_else(|error| {
        panic!(
            "missing expected registry {}: {error}. Run UPDATE_EXPECTED=1 cargo test -p meadow-handlers",
            expected_path.display()
        )
    });

    assert_eq!(actual, expected);
}

#[test]
fn builds_classes_modules_and_compact_namespaces() {
    let registry = build_registry(&parse_fixtures());

    assert_eq!(
        registry.find("Person").expect("Person").kind,
        ObjectKind::Class
    );
    assert_eq!(
        registry.find("Greeter").expect("Greeter").kind,
        ObjectKind::Module
    );
    assert_eq!(
        registry.find("Outer::Inner").expect("Inner").kind,
        ObjectKind::Module
    );
    assert_eq!(
        registry.find("Outer::Nested::Thing").expect("Thing").kind,
        ObjectKind::Class
    );
    assert_eq!(
        registry.find("Widget::Part").expect("Part").kind,
        ObjectKind::Class
    );

    let nested = registry.find("Outer::Nested").expect("implicit Nested");
    assert_eq!(nested.kind, ObjectKind::Module);
    assert!(nested.dynamic);
}

#[test]
fn reopens_classes_and_accumulates_sources() {
    let registry = build_registry(&parse_fixtures());
    let widget = registry.find("Widget").expect("Widget");
    assert_eq!(widget.sources.len(), 2);
}

#[test]
fn records_methods_scopes_and_signatures() {
    let registry = build_registry(&parse_fixtures());

    let initialize = method(&registry, "Person#initialize");
    let (scope, data) = method_data(initialize);
    assert_eq!(*scope, Scope::Instance);
    let KindData::Method {
        signature, params, ..
    } = data
    else {
        panic!("method data");
    };
    assert_eq!(
        signature,
        "initialize(name, age = 0, *rest, active: true, **options, &block)"
    );
    assert_eq!(params.len(), 6);

    let create = method(&registry, "Person.create");
    assert_eq!(method_data(create).0, &Scope::Class);

    let configure = method(&registry, "Factory.configure");
    assert_eq!(method_data(configure).0, &Scope::Class);
}

#[test]
fn creates_attribute_methods() {
    let registry = build_registry(&parse_fixtures());

    let name_reader = method(&registry, "Widget#name");
    let KindData::Method {
        attr: Some(attr), ..
    } = &name_reader.extra
    else {
        panic!("missing attr metadata");
    };
    assert!(attr.reader);
    assert!(attr.writer);

    let name_writer = method(&registry, "Widget#name=");
    let KindData::Method {
        attr: Some(attr), ..
    } = &name_writer.extra
    else {
        panic!("missing writer attr metadata");
    };
    assert!(attr.writer);

    let id = method(&registry, "Widget#id");
    let KindData::Method {
        attr: Some(attr), ..
    } = &id.extra
    else {
        panic!("missing id attr metadata");
    };
    assert!(attr.reader);
    assert!(!attr.writer);

    let secret = method(&registry, "Widget#secret=");
    let KindData::Method {
        attr: Some(attr), ..
    } = &secret.extra
    else {
        panic!("missing secret attr metadata");
    };
    assert!(attr.writer);
}

#[test]
fn applies_visibility_in_both_forms() {
    let registry = build_registry(&parse_fixtures());

    assert_eq!(
        method(&registry, "Widget#private_method").visibility,
        Visibility::Private
    );
    assert_eq!(
        method(&registry, "Widget#protected_method").visibility,
        Visibility::Protected
    );
    assert_eq!(
        method(&registry, "Widget#public_method").visibility,
        Visibility::Private
    );
    assert_eq!(
        method(&registry, "Calls#hidden").visibility,
        Visibility::Private
    );
    assert_eq!(
        method(&registry, "Widget#inline_private").visibility,
        Visibility::Private
    );
    assert_eq!(
        method(&registry, "Widget#inline_protected").visibility,
        Visibility::Protected
    );
}

#[test]
fn records_aliases_and_back_references() {
    let registry = build_registry(&parse_fixtures());

    let alias = method(&registry, "Person#nickname");
    let target = registry.find("Person#name").expect("Person#name");
    let KindData::Method {
        alias_of: Some(alias_of),
        ..
    } = &alias.extra
    else {
        panic!("missing alias_of");
    };
    assert_eq!(*alias_of, target.id);

    let KindData::Method { aliases, .. } = &target.extra else {
        panic!("target method data");
    };
    assert_eq!(aliases, &vec![alias.id]);

    let widget_alias = method(&registry, "Widget#alias_method");
    assert_eq!(widget_alias.docstring.text, "A public method.");
    let KindData::Method {
        alias_of,
        signature,
        ..
    } = &widget_alias.extra
    else {
        panic!("widget alias data");
    };
    assert!(alias_of.is_some());
    assert_eq!(signature, "alias_method()");

    let call_alias = method(&registry, "Widget#alias_by_call");
    let KindData::Method {
        alias_of,
        signature,
        ..
    } = &call_alias.extra
    else {
        panic!("call alias data");
    };
    assert!(alias_of.is_some());
    assert_eq!(signature, "alias_by_call()");
}

#[test]
fn records_mixins_in_order() {
    let registry = build_registry(&parse_fixtures());

    let utilities = registry.find("Utilities").expect("Utilities");
    let KindData::Namespace {
        includes,
        extends,
        prepends,
        mixin_order,
        ..
    } = &utilities.extra
    else {
        panic!("Utilities namespace");
    };

    assert_eq!(includes.len(), 1);
    assert_eq!(extends.len(), 1);
    assert_eq!(prepends.len(), 1);
    assert_eq!(
        mixin_order
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![MixinKind::Include, MixinKind::Extend, MixinKind::Prepend]
    );
    assert!(matches!(includes[0], Ref::Resolved(_)));

    let person = registry.find("Person").expect("Person");
    let KindData::Namespace {
        includes,
        superclass,
        ..
    } = &person.extra
    else {
        panic!("Person namespace");
    };
    assert!(matches!(includes[0], Ref::Resolved(_)));
    assert!(matches!(superclass, Some(Ref::Proxy { name, .. }) if name == "Object"));
}

#[test]
fn records_constants_class_variables_and_module_functions() {
    let registry = build_registry(&parse_fixtures());

    assert_eq!(
        registry.find("Person::VERSION").expect("VERSION").kind,
        ObjectKind::Constant
    );
    assert_eq!(
        registry.find("Widget::COLOR").expect("COLOR").kind,
        ObjectKind::Constant
    );
    assert_eq!(
        registry.find("Widget#@@count").expect("@@count").kind,
        ObjectKind::ClassVariable
    );
    assert_eq!(
        registry
            .find("Utilities#@@registry")
            .expect("@@registry")
            .kind,
        ObjectKind::ClassVariable
    );

    let util = method(&registry, "Utilities#util");
    assert_eq!(util.visibility, Visibility::Private);
    let util_class = method(&registry, "Utilities.util");
    assert_eq!(method_data(util_class).0, &Scope::Class);

    let double = method(&registry, "MathHelpers#double");
    assert_eq!(double.visibility, Visibility::Private);
    assert!(registry.find("MathHelpers.double").is_some());
}

#[test]
fn handles_conditionals_and_recovered_trees() {
    let registry = build_registry(&parse_fixtures());

    assert!(registry.find("Calls#conditional_method").is_some());
    assert!(registry.find("Broken#oops").is_some());
}

#[test]
fn registry_round_trips_through_bincode_and_json() {
    let registry = build_registry(&parse_fixtures());

    let encoded = bincode::serde::encode_to_vec(&registry, bincode::config::standard())
        .expect("bincode serialize");
    let (decoded, _): (meadow_model::Registry, usize) =
        bincode::serde::decode_from_slice(&encoded, bincode::config::standard())
            .expect("bincode deserialize");
    assert_eq!(registry, decoded);

    let encoded = serde_json::to_string(&registry).expect("json serialize");
    let decoded: meadow_model::Registry = serde_json::from_str(&encoded).expect("json deserialize");
    assert_eq!(registry, decoded);
}

#[test]
fn non_utf8_registry_round_trips() {
    let source: &[u8] =
        b"# encoding: ISO-8859-1\n# Caf\xe9\nclass Caf\xe9\n  def r\xe9sum\xe9; end\nend\n";
    let facts = parse_source_bytes("tests/fixtures/non_utf8.rb", source);
    let registry = build_registry(&[facts]);

    let caf = registry
        .objects()
        .iter()
        .find(|object| object.kind == ObjectKind::Class && object.name.as_bytes() == b"Caf\xe9")
        .expect("non-utf8 class");
    assert_eq!(caf.path.as_bytes(), b"Caf\xe9");
    assert!(registry.find("Caf\u{e9}").is_none());

    let encoded = serde_json::to_string(&registry).expect("json serialize");
    let decoded: meadow_model::Registry = serde_json::from_str(&encoded).expect("json deserialize");
    assert_eq!(registry, decoded);

    let encoded = bincode::serde::encode_to_vec(&registry, bincode::config::standard())
        .expect("bincode serialize");
    let (decoded, _): (meadow_model::Registry, usize) =
        bincode::serde::decode_from_slice(&encoded, bincode::config::standard())
            .expect("bincode deserialize");
    assert_eq!(registry, decoded);
}

#[test]
fn parses_tags_and_type_expressions() {
    let registry = build_registry(&parse_fixtures());
    let parse = method(&registry, "Tagged#parse");

    assert_eq!(
        parse.docstring.text,
        "Parses things.\n\nLonger description."
    );
    assert_eq!(parse.docstring.summary, "Parses things.");

    let tag = |name: &str| {
        parse
            .docstring
            .tags
            .iter()
            .find(|tag| tag.tag == name)
            .unwrap_or_else(|| panic!("missing tag @{name}"))
    };

    let param = parse
        .docstring
        .tags
        .iter()
        .find(|tag| tag.tag == "param" && tag.name.as_ref().is_some_and(|name| name == "name"))
        .expect("name param");
    assert_eq!(param.types.as_ref().unwrap().to_string(), "String, nil");

    let count = parse
        .docstring
        .tags
        .iter()
        .find(|tag| tag.tag == "param" && tag.name.as_ref().is_some_and(|name| name == "count"))
        .expect("count param");
    assert_eq!(count.types.as_ref().unwrap().to_string(), "Integer");

    let option = tag("option");
    assert_eq!(option.name.as_ref().unwrap(), "options");
    assert_eq!(option.key.as_ref().unwrap(), ":format");
    assert_eq!(option.default.as_ref().unwrap(), "\"json\"");
    assert_eq!(option.types.as_ref().unwrap().to_string(), "String");

    assert_eq!(
        tag("return").types.as_ref().unwrap().to_string(),
        "Array<String>"
    );
    assert_eq!(
        tag("raise").types.as_ref().unwrap().to_string(),
        "ArgumentError"
    );
    assert_eq!(
        tag("yieldreturn").types.as_ref().unwrap().to_string(),
        "void"
    );

    let overload = tag("overload");
    assert_eq!(overload.text, "parse(input)");
    let nested = overload.docstring.as_ref().expect("overload docstring");
    assert_eq!(nested.tags.len(), 2);
    assert_eq!(nested.tags[0].name.as_ref().unwrap(), "input");

    let example = tag("example");
    assert_eq!(example.text, "Basic usage\n  Tagged.new.parse(\"x\")");
}

#[test]
fn reports_docstring_warnings() {
    let (_, warnings) = build_registry_with_warnings(&parse_fixtures());

    assert_eq!(warnings.len(), 3, "{warnings:#?}");
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("unknown docstring tag"))
    );
    assert!(warnings.iter().any(|warning| {
        warning
            .message
            .contains("does not match any method parameter")
    }));
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("invalid type list"))
    );
    assert!(warnings.iter().all(|warning| warning.location.start > 0));
}

#[test]
fn inherits_docstrings_from_superclasses() {
    let registry = build_registry(&parse_fixtures());
    let parent = method(&registry, "Tagged#parse");
    let child = method(&registry, "Child#parse");

    assert_eq!(child.docstring, parent.docstring);
}

#[test]
fn applies_docstring_directives() {
    let registry = build_registry(&parse_fixtures());

    let generated = method(&registry, "Directives#generated");
    assert_eq!(generated.sources.len(), 1);
    assert!(generated.dynamic);
    let KindData::Method {
        params, signature, ..
    } = &generated.extra
    else {
        panic!("generated method data");
    };
    assert_eq!(signature, "generated(a, b = 1)");
    assert_eq!(params.len(), 2);
    assert_eq!(generated.docstring.tags.len(), 2);

    let exposed = method(&registry, "Directives#exposed");
    let KindData::Method {
        attr: Some(attr), ..
    } = &exposed.extra
    else {
        panic!("exposed attr");
    };
    assert!(attr.reader && attr.writer);
    assert!(registry.find("Directives#exposed=").is_some());

    let class_generated = method(&registry, "Directives.class_generated");
    assert_eq!(method_data(class_generated).0, &Scope::Class);

    assert_eq!(
        method(&registry, "Directives#hidden_generated").visibility,
        Visibility::Private
    );

    let grouped = method(&registry, "Directives#grouped");
    assert_eq!(grouped.group.as_ref().unwrap(), "Utilities");

    assert!(registry.find("Directives#from_parse").is_some());
}

#[test]
fn expands_attached_macros() {
    let registry = build_registry(&parse_fixtures());

    let property = method(&registry, "Directives#property");
    let return_tag = property
        .docstring
        .tags
        .iter()
        .find(|tag| tag.tag == "return")
        .expect("macro return tag");
    assert_eq!(return_tag.text, "the :widget value");
    assert_eq!(return_tag.types.as_ref().unwrap().to_string(), ":widget");
}

#[test]
fn registry_output_is_deterministic() {
    let first = build_registry(&parse_fixtures());
    let second = build_registry(&parse_fixtures());
    assert_eq!(
        serde_json::to_vec(&first).expect("serialize first"),
        serde_json::to_vec(&second).expect("serialize second")
    );
}

#[test]
fn root_is_present_and_paths_are_unique() {
    let registry = build_registry(&parse_fixtures());
    assert_eq!(registry.root().kind, ObjectKind::Root);
    assert_eq!(registry.objects().len(), registry.path_index().len());
}
