//! RBS parsing tests.

use std::path::{Path, PathBuf};

use meadow_parser::{
    DiagnosticLevel, Event, MethodScope, ParamKind, parse_file, parse_rbs_source,
    parse_source_bytes,
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/rbs")
        .join(name)
}

#[test]
fn parses_rbs_declarations_and_docstrings() {
    let facts = parse_file(fixture("core.rbs")).expect("parse RBS fixture");
    assert!(facts.diagnostics.is_empty(), "{:?}", facts.diagnostics);

    let class = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::ClassOpen(open) if open.name == "Person" => Some(open),
            _ => None,
        })
        .expect("Person class");
    assert_eq!(class.superclass.as_ref().expect("superclass"), "Object");
    assert_eq!(
        class.docstring.as_ref().expect("class docstring").text,
        "A person."
    );

    let module = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::ModuleOpen(open) if open.name == "Greeter" => Some(open),
            _ => None,
        })
        .expect("Greeter module");
    assert!(module.docstring.is_none());
}

#[test]
fn parses_method_signatures_and_overloads() {
    let facts = parse_file(fixture("core.rbs")).expect("parse RBS fixture");

    let initialize = method(&facts, "initialize");
    assert_eq!(initialize.scope, MethodScope::Instance);
    assert_eq!(initialize.signature, "initialize(name, age = ?)");
    assert_eq!(initialize.parameters.len(), 2);
    assert_eq!(initialize.parameters[0].name, "name");
    assert_eq!(initialize.parameters[0].kind, ParamKind::Required);
    assert_eq!(initialize.parameters[1].kind, ParamKind::Optional);
    let docstring = initialize.docstring.as_ref().expect("method docstring");
    assert!(docstring.text.contains("@param [String] name"));
    assert!(docstring.text.contains("@param [Integer] age"));
    assert!(docstring.text.contains("@return [void]"));

    let greet = method(&facts, "greet");
    assert_eq!(greet.parameters.len(), 2);
    assert_eq!(greet.parameters[0].name, "name");
    assert_eq!(greet.parameters[1].kind, ParamKind::KeywordOptional);
    let text = greet
        .docstring
        .as_ref()
        .expect("overload docstring")
        .text
        .to_string_lossy();
    assert_eq!(text.matches("@overload greet(").count(), 2, "{text}");
    assert!(text.contains("  @param [String] name"), "{text}");
    assert!(text.contains("  @return [Array<String>]"), "{text}");

    let create = method(&facts, "create");
    assert_eq!(create.scope, MethodScope::Class);
    assert_eq!(create.receiver.as_ref().expect("receiver"), "self");
}

#[test]
fn parses_attributes_mixins_aliases_and_variables() {
    let facts = parse_file(fixture("core.rbs")).expect("parse RBS fixture");

    let name = method(&facts, "name");
    assert_eq!(name.parameters.len(), 0);
    assert!(
        name.docstring
            .as_ref()
            .expect("attribute docstring")
            .text
            .contains("@return [String]")
    );

    let age_writer = method(&facts, "age=");
    assert_eq!(age_writer.parameters[0].name, "value");
    assert!(
        age_writer
            .docstring
            .as_ref()
            .expect("writer docstring")
            .text
            .contains("@param [Integer] value")
    );

    let calls: Vec<_> = facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Call(call) => Some(call),
            _ => None,
        })
        .collect();
    assert!(
        calls
            .iter()
            .any(|call| call.name == "include" && call.arguments[0].source == "Comparable")
    );
    assert!(
        calls
            .iter()
            .any(|call| call.name == "extend" && call.arguments[0].source == "Forwardable")
    );
    assert!(
        calls
            .iter()
            .any(|call| call.name == "prepend" && call.arguments[0].source == "Enumerable")
    );
    assert!(calls.iter().any(|call| call.name == "private"));

    let alias = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::Alias(alias) => Some(alias),
            _ => None,
        })
        .expect("alias");
    assert_eq!(alias.new_name, "nickname");
    assert_eq!(alias.old_name, "name");

    assert!(facts.events.iter().any(|event| matches!(
        event,
        Event::ConstantWrite(constant) if constant.path == "VERSION" && constant.value_source == "String"
    )));
    assert!(facts.events.iter().any(|event| matches!(
        event,
        Event::InstanceVariableWrite(variable) if variable.name == "@cache"
    )));
    assert!(facts.events.iter().any(|event| matches!(
        event,
        Event::ClassVariableWrite(variable) if variable.name == "@@count"
    )));
}

#[test]
fn reports_rbs_parse_errors() {
    let facts = parse_rbs_source("broken.rbs", "class { end");
    assert_eq!(facts.events.len(), 0);
    assert_eq!(facts.diagnostics.len(), 1);
    assert_eq!(facts.diagnostics[0].level, DiagnosticLevel::Error);
    assert_ne!(facts.diagnostics[0].message.as_bytes(), b"");

    let facts = parse_source_bytes("broken.rbs", &[0xff, 0xfe]);
    assert_eq!(facts.events.len(), 0);
    assert_eq!(facts.diagnostics.len(), 1);
}

#[test]
fn ignores_interfaces() {
    let facts = parse_file(fixture("core.rbs")).expect("parse RBS fixture");
    assert!(!facts.events.iter().any(|event| match event {
        Event::ClassOpen(open) => open.name.as_bytes() == b"_Greetable",
        Event::ModuleOpen(open) => open.name.as_bytes() == b"_Greetable",
        _ => false,
    }));
}

fn method<'a>(
    facts: &'a meadow_parser::FileFacts,
    name: &str,
) -> &'a meadow_parser::MethodDefEvent {
    facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::MethodDef(method) if method.name == name => Some(method),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing method {name}"))
}
