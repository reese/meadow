//! Expected-output tests for the owned parser event stream.

use std::fs;
use std::path::{Path, PathBuf};

use meadow_parser::{
    DiagnosticLevel, Event, FileFacts, MethodScope, ParamKind, parse_source, parse_source_bytes,
    parse_sources,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> (String, String) {
    let relative = format!("tests/fixtures/{name}");
    let source = fs::read_to_string(workspace_root().join(&relative)).expect("read fixture");
    (relative, source)
}

fn parse_fixture(name: &str) -> meadow_parser::FileFacts {
    let (relative, source) = fixture(name);
    parse_source(relative, &source)
}

#[test]
fn expected_event_streams() {
    for name in [
        "core.rb",
        "docstrings.rb",
        "parameters.rb",
        "calls.rb",
        "syntax_error.rb",
    ] {
        let facts = parse_fixture(name);
        let actual = format!(
            "{}\n",
            serde_json::to_string_pretty(&facts).expect("serialize fixture facts")
        );
        let expected_path = workspace_root()
            .join("tests/expected")
            .join(format!("{name}.json"));

        if std::env::var_os("UPDATE_EXPECTED").is_some() {
            fs::write(&expected_path, &actual).expect("write expected output");
        }

        let expected = fs::read_to_string(&expected_path).unwrap_or_else(|error| {
            panic!(
                "missing expected output {}: {error}. Run UPDATE_EXPECTED=1 cargo test -p meadow-parser",
                expected_path.display()
            )
        });

        assert_eq!(actual, expected, "expected output mismatch for {name}");
    }
}

#[test]
fn events_are_in_source_order() {
    for name in ["core.rb", "docstrings.rb", "parameters.rb", "calls.rb"] {
        let facts = parse_fixture(name);
        let offsets: Vec<u32> = facts
            .events
            .iter()
            .map(|event| event.location().start)
            .collect();
        assert!(
            offsets.windows(2).all(|pair| pair[0] <= pair[1]),
            "events out of order in {name}: {offsets:?}"
        );
    }
}

#[test]
fn attaches_preceding_comment_blocks() {
    let facts = parse_fixture("core.rb");
    let class = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::ClassOpen(open) if open.name == "Person" => Some(open),
            _ => None,
        })
        .expect("Person class event");

    let docstring = class.docstring.as_ref().expect("class docstring");
    assert_eq!(docstring.text, "A class with a superclass.");

    let module = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::ModuleOpen(open) if open.name == "Greeter" => Some(open),
            _ => None,
        })
        .expect("Greeter module event");
    assert_eq!(
        module.docstring.as_ref().expect("module docstring").text,
        "A top-level module."
    );
}

#[test]
fn keeps_relative_indentation_in_docstrings() {
    let facts = parse_fixture("docstrings.rb");
    let method = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::MethodDef(method) if method.name == "indented" => Some(method),
            _ => None,
        })
        .expect("indented method");

    assert_eq!(
        method.docstring.as_ref().expect("docstring").text,
        "  indented code example\n    puts \"hi\""
    );
}

#[test]
fn supports_embdoc_comments() {
    let facts = parse_fixture("docstrings.rb");
    let method = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::MethodDef(method) if method.name == "from_embdoc" => Some(method),
            _ => None,
        })
        .expect("embdoc method");

    assert_eq!(
        method.docstring.as_ref().expect("docstring").text,
        "A method documented with an embdoc block."
    );
}

#[test]
fn attaches_trailing_attribute_comments() {
    let facts = parse_fixture("docstrings.rb");
    let call = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::Call(call) if call.name == "attr_accessor" => Some(call),
            _ => None,
        })
        .expect("attr_accessor call");

    assert_eq!(
        call.docstring.as_ref().expect("trailing docstring").text,
        "Trailing comment for an attribute."
    );
}

#[test]
fn ignores_magic_comments_and_shebangs() {
    let facts = parse_fixture("docstrings.rb");
    let class = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::ClassOpen(open) if open.name == "Documented" => Some(open),
            _ => None,
        })
        .expect("Documented class");

    let text = &class.docstring.as_ref().expect("docstring").text;
    assert!(!text.contains("frozen_string_literal"));
    assert!(!text.contains("usr/bin/env"));
}

#[test]
fn collects_free_standing_directive_comments() {
    let facts = parse_fixture("docstrings.rb");
    let comments: Vec<_> = facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Comment(comment) => Some(comment),
            _ => None,
        })
        .collect();

    assert!(
        comments
            .iter()
            .any(|comment| comment.text.contains("@!macro [new] my_macro")),
        "missing free-standing directive comment: {comments:#?}"
    );
}

#[test]
fn records_parameter_kinds_and_signatures() {
    let facts = parse_fixture("parameters.rb");
    let methods: Vec<_> = facts
        .events
        .iter()
        .filter_map(|event| match event {
            Event::MethodDef(method) => Some(method),
            _ => None,
        })
        .collect();

    let with_rest = methods
        .iter()
        .find(|method| method.name == "with_rest")
        .expect("with_rest");
    assert_eq!(with_rest.signature, "with_rest(a, *rest, z)");
    assert_eq!(
        with_rest
            .parameters
            .iter()
            .map(|param| param.kind)
            .collect::<Vec<_>>(),
        vec![ParamKind::Required, ParamKind::Rest, ParamKind::Post,]
    );

    let keywords = methods
        .iter()
        .find(|method| method.name == "with_keywords")
        .expect("with_keywords");
    assert_eq!(keywords.signature, "with_keywords(a:, b: 2, **opts)");

    let anonymous = methods
        .iter()
        .find(|method| method.name == "anonymous")
        .expect("anonymous");
    assert_eq!(anonymous.signature, "anonymous(*, **, &)");

    let forwarding = methods
        .iter()
        .find(|method| method.name == "forwarding")
        .expect("forwarding");
    assert_eq!(forwarding.signature, "forwarding(...)");
    assert_eq!(forwarding.parameters[0].kind, ParamKind::Forwarding);

    let endless = methods
        .iter()
        .find(|method| method.name == "endless")
        .expect("endless");
    assert_eq!(endless.signature, "endless(x)");

    let object_method = methods
        .iter()
        .find(|method| method.name == "method")
        .expect("object method");
    assert_eq!(object_method.scope, MethodScope::Class);
    assert_eq!(
        object_method
            .receiver
            .as_ref()
            .expect("receiver")
            .as_bytes(),
        b"object"
    );
    assert_eq!(object_method.signature, "object.method(value)");
}

#[test]
fn recovers_from_syntax_errors() {
    let facts = parse_fixture("syntax_error.rb");
    assert!(
        facts
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error),
        "expected a syntax error diagnostic"
    );
}

#[test]
fn parallel_parsing_preserves_input_order() {
    let fixtures = ["core.rb", "docstrings.rb", "parameters.rb", "calls.rb"]
        .into_iter()
        .map(|name| {
            let (relative, source) = fixture(name);
            (PathBuf::from(relative), source)
        })
        .collect::<Vec<_>>();

    let results = parse_sources(&fixtures);
    assert_eq!(results.len(), fixtures.len());
    for (result, (path, _)) in results.iter().zip(&fixtures) {
        assert_eq!(&result.path, path);
    }
}

#[test]
fn preserves_non_utf8_source_bytes() {
    let source: &[u8] = b"# encoding: ISO-8859-1\n# caf\xe9\nclass NonUtf8; end\n";
    let facts = parse_source_bytes("tests/fixtures/non_utf8.rb", source);

    assert!(
        facts
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.level != DiagnosticLevel::Error),
        "unexpected errors: {:?}",
        facts.diagnostics
    );

    let class = facts
        .events
        .iter()
        .find_map(|event| match event {
            Event::ClassOpen(open) if open.name.as_bytes() == b"NonUtf8" => Some(open),
            _ => None,
        })
        .expect("NonUtf8 class");
    assert_eq!(
        class.docstring.as_ref().expect("docstring").text.as_bytes(),
        b"caf\xe9"
    );

    let json = serde_json::to_string(&facts).expect("serialize");
    let round_tripped: FileFacts = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(facts, round_tripped);
}

#[test]
fn file_facts_are_thread_safe() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<meadow_parser::FileFacts>();
    assert_send_sync::<meadow_parser::Event>();
}
