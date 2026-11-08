//! Tag and type parser tests.

use meadow_docstring::{DocstringOptions, parse_docstring, parse_type_list, parse_type_list_bytes};
use meadow_model::{HashStyle, SourceLocation, SourceText, TypeExpr};

fn location() -> SourceLocation {
    SourceLocation {
        start: 0,
        end: 100,
        start_line: 10,
        end_line: 20,
    }
}

fn text(value: &str) -> SourceText {
    SourceText::from_bytes(value.as_bytes())
}

#[test]
fn parses_type_forms() {
    let cases = [
        ("String", "String"),
        ("Foo::Bar", "Foo::Bar"),
        ("Array<String>", "Array<String>"),
        ("Hash<Symbol, Object>", "Hash<Symbol, Object>"),
        ("Hash{Symbol=>Object}", "Hash{Symbol=>Object}"),
        ("(String, Integer)", "(String, Integer)"),
        ("String, nil", "String, nil"),
        ("#each", "#each"),
        (
            "true, false, nil, void, self",
            "true, false, nil, void, self",
        ),
        (":foo", ":foo"),
        ("Class<Foo>", "Class<Foo>"),
    ];

    for (input, expected) in cases {
        let parsed =
            parse_type_list(&text(input)).unwrap_or_else(|error| panic!("{input}: {error}"));
        assert_eq!(parsed.to_string(), expected, "input {input}");
    }
}

#[test]
fn parses_nested_types() {
    let TypeExpr::Generic { base, params } =
        parse_type_list(&text("Array<Hash{Symbol=>Array<Integer>}>")).unwrap()
    else {
        panic!("not generic");
    };
    assert_eq!(base.to_string(), "Array");
    assert_eq!(params.len(), 1);
    let TypeExpr::Hash { key, value, style } = &params[0] else {
        panic!("not hash");
    };
    assert_eq!(*style, HashStyle::Braces);
    assert_eq!(key.to_string(), "Symbol");
    assert_eq!(value.to_string(), "Array<Integer>");
}

#[test]
fn rejects_malformed_types() {
    assert!(parse_type_list(&text("Array<")).is_err());
    assert!(parse_type_list(&text("Hash{Symbol=>}")).is_err());
    assert!(parse_type_list(&text("(String,)")).is_err());
    assert!(parse_type_list_bytes(b"Foo::Bar").is_ok());
}

#[test]
fn summarizes_the_first_paragraph() {
    let parsed = parse_docstring(
        &text("A summary that spans\ntwo lines.\n\nSecond paragraph."),
        location(),
        &DocstringOptions::default(),
    );
    assert_eq!(parsed.docstring.summary, "A summary that spans\ntwo lines.");
    assert_eq!(
        parsed.docstring.text,
        "A summary that spans\ntwo lines.\n\nSecond paragraph."
    );
}

#[test]
fn parses_docstring_body_summary_and_tags() {
    let source = text(
        "Does things.\n\n@param [String, nil] name the name\n@param count [Integer] the count\n@return [Array<String>] results\n@option options [String] :format (\"json\") output format",
    );
    let options = DocstringOptions {
        warn_unknown_tags: true,
        validate_params: true,
        parameter_names: vec![text("name"), text("count"), text("options")],
    };
    let parsed = parse_docstring(&source, location(), &options);

    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    assert_eq!(parsed.docstring.text, "Does things.");
    assert_eq!(parsed.docstring.summary, "Does things.");
    assert_eq!(parsed.docstring.tags.len(), 4);

    let name = &parsed.docstring.tags[0];
    assert_eq!(name.tag, "param");
    assert_eq!(name.name.as_ref().unwrap(), "name");
    assert_eq!(name.types.as_ref().unwrap().to_string(), "String, nil");
    assert_eq!(name.text, "the name");

    let count = &parsed.docstring.tags[1];
    assert_eq!(count.name.as_ref().unwrap(), "count");
    assert_eq!(count.types.as_ref().unwrap().to_string(), "Integer");

    let results = &parsed.docstring.tags[2];
    assert_eq!(results.tag, "return");
    assert_eq!(results.types.as_ref().unwrap().to_string(), "Array<String>");

    let option = &parsed.docstring.tags[3];
    assert_eq!(option.tag, "option");
    assert_eq!(option.name.as_ref().unwrap(), "options");
    assert_eq!(option.key.as_ref().unwrap(), ":format");
    assert_eq!(option.default.as_ref().unwrap(), "\"json\"");
    assert_eq!(option.text, "output format");
}

#[test]
fn warns_for_unknown_tags_and_params() {
    let source = text("@param [String] other nope\n@unknown mystery");
    let options = DocstringOptions {
        warn_unknown_tags: true,
        validate_params: true,
        parameter_names: vec![text("name")],
    };
    let parsed = parse_docstring(&source, location(), &options);

    assert_eq!(parsed.warnings.len(), 2);
    assert!(parsed.warnings[0].message.contains("does not match"));
    assert!(parsed.warnings[1].message.contains("unknown docstring tag"));
    assert_eq!(parsed.docstring.tags.len(), 2);
}

#[test]
fn falls_back_to_raw_types_on_error() {
    let source = text("@return [Array<] broken");
    let parsed = parse_docstring(&source, location(), &DocstringOptions::default());

    let tag = &parsed.docstring.tags[0];
    assert!(tag.types.is_none());
    assert_eq!(tag.types_source.as_ref().unwrap(), "Array<");
    assert_eq!(tag.text, "broken");
    assert_eq!(parsed.warnings.len(), 1);
    assert!(parsed.warnings[0].message.contains("invalid type list"));
}

#[test]
fn parses_overload_sub_docstrings() {
    let source = text(
        "@overload run(input)\n  @param [String] input the input\n  @return [Array<String>] parsed",
    );
    let options = DocstringOptions {
        validate_params: true,
        parameter_names: vec![text("input")],
        ..DocstringOptions::default()
    };
    let parsed = parse_docstring(&source, location(), &options);

    let overload = &parsed.docstring.tags[0];
    assert_eq!(overload.tag, "overload");
    assert_eq!(overload.text, "run(input)");
    let nested = overload.docstring.as_ref().expect("nested docstring");
    assert_eq!(nested.tags.len(), 2);
    assert_eq!(nested.tags[0].tag, "param");
    assert_eq!(nested.tags[1].tag, "return");
}

#[test]
fn unknown_tags_are_preserved() {
    let options = DocstringOptions {
        warn_unknown_tags: false,
        ..DocstringOptions::default()
    };
    let parsed = parse_docstring(&text("@custom value"), location(), &options);
    assert_eq!(parsed.docstring.tags.len(), 1);
    assert_eq!(parsed.docstring.tags[0].tag, "custom");
    assert_eq!(parsed.docstring.tags[0].text, "value");
    assert_eq!(parsed.warnings.len(), 0);
}
