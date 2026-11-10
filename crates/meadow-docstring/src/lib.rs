//! Docstring, tag, and type parsing for meadow.
//!
//! This crate turns the raw comment text attached by the parser into a
//! structured [`Docstring`] with an ordered list of tags and structured type
//! expressions. Malformed input never panics: type errors fall back to the raw
//! type text and are reported as warnings.

#![deny(missing_docs)]

mod types;

use meadow_model::{Docstring, SourceLocation, SourceText, Tag, TypeExpr};

pub use types::{TypeParseError, parse_type_list, parse_type_list_bytes};

/// Options controlling docstring parsing.
#[derive(Debug, Clone, Default)]
pub struct DocstringOptions {
    /// Warn about tags outside the standard set.
    pub warn_unknown_tags: bool,
    /// Validate `@param` and `@option` names against a method signature.
    pub validate_params: bool,
    /// Parameter names for validation, without sigils.
    pub parameter_names: Vec<SourceText>,
}

/// A non-fatal warning produced while parsing a docstring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocstringWarning {
    /// Human-readable message.
    pub message: String,
    /// Location of the docstring in the source.
    pub location: SourceLocation,
}

/// A parsed docstring plus any warnings generated while parsing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDocstring {
    /// The structured docstring.
    pub docstring: Docstring,
    /// Warnings emitted during parsing.
    pub warnings: Vec<DocstringWarning>,
}

/// Parses docstring text and tags.
#[must_use]
pub fn parse_docstring(
    text: &SourceText,
    location: SourceLocation,
    options: &DocstringOptions,
) -> ParsedDocstring {
    let mut warnings = Vec::new();
    let docstring = parse_into(text.as_bytes(), location, options, &mut warnings);
    ParsedDocstring {
        docstring,
        warnings,
    }
}

fn parse_into(
    text: &[u8],
    location: SourceLocation,
    options: &DocstringOptions,
    warnings: &mut Vec<DocstringWarning>,
) -> Docstring {
    let lines = split_lines(text);
    let mut lines = lines;
    let markup_tag =
        if let Some(index) = lines.iter().position(|line| !line.trim_ascii().is_empty()) {
            let line = trim_start(lines[index]);
            if let Some(value) = line.strip_prefix(b"@markup ") {
                let value = trim_bytes(first_line(line.strip_prefix(b"@markup ").unwrap_or(value)));
                lines.remove(index);
                Some(Tag {
                    tag: SourceText::from("markup"),
                    name: None,
                    types: None,
                    types_source: None,
                    text: SourceText::from_bytes(value),
                    docstring: None,
                    key: None,
                    default: None,
                })
            } else {
                None
            }
        } else {
            None
        };
    let mut body_lines: Vec<&[u8]> = Vec::new();
    let mut tag_groups: Vec<Vec<&[u8]>> = Vec::new();
    let mut tag_indent: Option<usize> = None;

    for line in lines {
        if line.starts_with(b"@") {
            tag_groups.push(vec![line]);
            tag_indent = Some(0);
        } else if let Some(group) = tag_groups.last_mut() {
            let base = tag_indent.unwrap_or(0);
            if line.trim_ascii().is_empty() || indent(line) > base {
                group.push(line);
            } else {
                // A non-empty line at the tag's indentation resumes the
                // docstring body. This matters for `@!macro` references that
                // are followed by more prose at the same level.
                body_lines.push(line);
                tag_indent = None;
            }
        } else {
            body_lines.push(line);
        }
    }

    while body_lines
        .last()
        .is_some_and(|line| line.trim_ascii().is_empty())
    {
        body_lines.pop();
    }

    let body = join_lines(&body_lines);
    let summary = summarize(&body);
    let mut tags = Vec::with_capacity(tag_groups.len());
    if let Some(markup) = markup_tag {
        tags.push(markup);
    }

    for group in &tag_groups {
        tags.push(parse_tag(group, location, options, warnings));
    }

    Docstring {
        text: SourceText::from_vec(body),
        summary,
        tags,
    }
}

fn parse_tag(
    lines: &[&[u8]],
    location: SourceLocation,
    options: &DocstringOptions,
    warnings: &mut Vec<DocstringWarning>,
) -> Tag {
    let first = lines.first().copied().unwrap_or_default();
    let (tag_name, first_rest) = split_tag_name(first);
    let rest = join_rest(first_rest, &lines[1..]);
    let tag = SourceText::from_bytes(tag_name);

    if tag_name == b"!macro" {
        return parse_macro_tag(&tag, first_rest, &lines[1..]);
    }

    match tag_name {
        b"overload" => parse_overload(&rest, location, options, warnings, tag),
        b"option" => parse_option(&rest, location, options, warnings, tag),
        b"param" | b"yieldparam" | b"attr" | b"attr_reader" | b"attr_writer" => {
            parse_named_typed(&rest, location, options, warnings, tag)
        }
        b"return" | b"raise" | b"yield" | b"yieldreturn" => {
            parse_typed(&rest, location, warnings, tag)
        }
        b"example" => Tag {
            tag,
            name: None,
            types: None,
            types_source: None,
            // Keep the newline that starts a code-only example. Trimming all
            // whitespace would remove the first code line's indentation while
            // leaving the rest, producing a misaligned code block.
            text: SourceText::from_bytes(trim_horizontal_start(&rest)),
            docstring: None,
            key: None,
            default: None,
        },
        b"abstract" | b"api" | b"author" | b"deprecated" | b"note" | b"private" | b"see"
        | b"since" | b"todo" | b"version" => Tag {
            tag,
            name: None,
            types: None,
            types_source: None,
            text: SourceText::from_bytes(trim_start(&rest)),
            docstring: None,
            key: None,
            default: None,
        },
        _ => {
            if options.warn_unknown_tags && !tag_name.starts_with(b"!") {
                warnings.push(DocstringWarning {
                    message: format!(
                        "unknown docstring tag @{}",
                        String::from_utf8_lossy(tag_name)
                    ),
                    location,
                });
            }
            Tag {
                tag,
                name: None,
                types: None,
                types_source: None,
                text: SourceText::from_bytes(trim_start(&rest)),
                docstring: None,
                key: None,
                default: None,
            }
        }
    }
}

fn parse_macro_tag(tag: &SourceText, first_rest: &[u8], continuation: &[&[u8]]) -> Tag {
    let mut first = trim_start(first_rest);
    // A macro declaration may carry modifiers before its name, such as
    // `[attach]` or `[new]`. They are not part of the macro name.
    while first.starts_with(b"[") {
        let Some(end) = first.iter().position(|byte| *byte == b']') else {
            break;
        };
        first = trim_start(&first[end + 1..]);
    }
    let (name, first_body) = if first.is_empty() {
        (None, &[][..])
    } else {
        let name_end = first
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(first.len());
        (Some(&first[..name_end]), &first[name_end..])
    };
    let body = join_rest(trim_start(first_body), continuation);
    Tag {
        tag: tag.clone(),
        name: name.map(SourceText::from_bytes),
        types: None,
        types_source: None,
        text: SourceText::from_bytes(trim_end(&body)),
        docstring: None,
        key: None,
        default: None,
    }
}

fn parse_named_typed(
    rest: &[u8],
    location: SourceLocation,
    options: &DocstringOptions,
    warnings: &mut Vec<DocstringWarning>,
    tag: SourceText,
) -> Tag {
    let mut cursor = 0;
    let mut types_source = parse_optional_types(rest, &mut cursor);
    let name = read_token(rest, &mut cursor).filter(|name| !name.is_empty());

    if types_source.is_none() {
        types_source = parse_optional_types(rest, &mut cursor);
    }

    if options.validate_params
        && tag.as_bytes() == b"param"
        && let Some(name) = &name
        && !options
            .parameter_names
            .iter()
            .any(|parameter| parameter.as_bytes() == strip_sigil(name.as_bytes()))
    {
        warnings.push(DocstringWarning {
            message: format!(
                "@{} name '{}' does not match any method parameter",
                tag,
                name.to_string_lossy()
            ),
            location,
        });
    }

    let (types, types_source) = parse_types_or_warn(types_source, &tag, location, warnings);
    Tag {
        tag,
        name: Some(name.unwrap_or_default()),
        types,
        types_source,
        text: SourceText::from_bytes(trim_start(&rest[cursor..])),
        docstring: None,
        key: None,
        default: None,
    }
}

fn parse_typed(
    rest: &[u8],
    location: SourceLocation,
    warnings: &mut Vec<DocstringWarning>,
    tag: SourceText,
) -> Tag {
    let mut cursor = 0;
    let types_source = parse_optional_types(rest, &mut cursor);
    let (types, types_source) = parse_types_or_warn(types_source, &tag, location, warnings);

    Tag {
        tag,
        name: None,
        types,
        types_source,
        text: SourceText::from_bytes(trim_start(&rest[cursor..])),
        docstring: None,
        key: None,
        default: None,
    }
}

fn parse_option(
    rest: &[u8],
    location: SourceLocation,
    options: &DocstringOptions,
    warnings: &mut Vec<DocstringWarning>,
    tag: SourceText,
) -> Tag {
    let mut cursor = 0;
    let name = read_token(rest, &mut cursor).filter(|name| !name.is_empty());
    let types_source = parse_optional_types(rest, &mut cursor);
    let key = read_token(rest, &mut cursor).filter(|key| !key.is_empty());
    let default = parse_optional_parenthesized(rest, &mut cursor);

    if options.validate_params
        && tag.as_bytes() == b"param"
        && let Some(name) = &name
        && !options
            .parameter_names
            .iter()
            .any(|parameter| parameter.as_bytes() == strip_sigil(name.as_bytes()))
    {
        warnings.push(DocstringWarning {
            message: format!(
                "@option name '{}' does not match any method parameter",
                name.to_string_lossy()
            ),
            location,
        });
    }

    if key.is_none() {
        warnings.push(DocstringWarning {
            message: "@option is missing its key".to_string(),
            location,
        });
    }

    let (types, types_source) = parse_types_or_warn(types_source, &tag, location, warnings);
    Tag {
        tag,
        name,
        types,
        types_source,
        text: SourceText::from_bytes(trim_start(&rest[cursor..])),
        docstring: None,
        key,
        default,
    }
}

fn parse_overload(
    rest: &[u8],
    location: SourceLocation,
    options: &DocstringOptions,
    warnings: &mut Vec<DocstringWarning>,
    tag: SourceText,
) -> Tag {
    let (signature, nested_text) = match rest.iter().position(|byte| *byte == b'\n') {
        Some(position) => (&rest[..position], &rest[position + 1..]),
        None => (rest, &[][..]),
    };
    let nested_options = DocstringOptions {
        warn_unknown_tags: options.warn_unknown_tags,
        validate_params: false,
        parameter_names: Vec::new(),
    };
    let nested = parse_into(&dedent(nested_text), location, &nested_options, warnings);

    Tag {
        tag,
        name: None,
        types: None,
        types_source: None,
        text: SourceText::from_bytes(trim_start(signature)),
        docstring: Some(Box::new(nested)),
        key: None,
        default: None,
    }
}

fn parse_types_or_warn(
    types_source: Option<SourceText>,
    tag: &SourceText,
    location: SourceLocation,
    warnings: &mut Vec<DocstringWarning>,
) -> (Option<TypeExpr>, Option<SourceText>) {
    let Some(raw) = types_source else {
        return (None, None);
    };

    match parse_type_list(&raw) {
        Ok(types) => (Some(types), Some(raw)),
        Err(error) => {
            warnings.push(DocstringWarning {
                message: format!("invalid type list for @{tag}: {error}; falling back to raw text"),
                location,
            });
            (None, Some(raw))
        }
    }
}

fn split_tag_name(line: &[u8]) -> (&[u8], &[u8]) {
    let line = line.strip_prefix(b"@").unwrap_or(line);
    let end = line
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(line.len());
    (&line[..end], &line[end..])
}

fn join_rest(first: &[u8], continuation: &[&[u8]]) -> Vec<u8> {
    let mut result = trim_start(first).to_vec();
    for line in continuation {
        result.push(b'\n');
        result.extend_from_slice(line);
    }
    result
}

fn parse_optional_types(text: &[u8], cursor: &mut usize) -> Option<SourceText> {
    skip_whitespace(text, cursor);
    if text.get(*cursor) != Some(&b'[') {
        return None;
    }

    let start = *cursor + 1;
    let mut depth = 1;
    let mut index = start;
    while index < text.len() {
        match text[index] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let value = SourceText::from_bytes(&text[start..index]);
                    *cursor = index + 1;
                    return Some(value);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn parse_optional_parenthesized(text: &[u8], cursor: &mut usize) -> Option<SourceText> {
    skip_whitespace(text, cursor);
    if text.get(*cursor) != Some(&b'(') {
        return None;
    }

    let start = *cursor + 1;
    let mut depth = 1;
    let mut index = start;
    while index < text.len() {
        match text[index] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    let value = SourceText::from_bytes(&text[start..index]);
                    *cursor = index + 1;
                    return Some(value);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn read_token(text: &[u8], cursor: &mut usize) -> Option<SourceText> {
    skip_whitespace(text, cursor);
    let start = *cursor;
    while let Some(byte) = text.get(*cursor) {
        if byte.is_ascii_whitespace() || matches!(byte, b'[' | b'(') {
            break;
        }
        *cursor += 1;
    }
    if *cursor == start {
        None
    } else {
        Some(SourceText::from_bytes(&text[start..*cursor]))
    }
}

fn skip_whitespace(text: &[u8], cursor: &mut usize) {
    while text.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
        *cursor += 1;
    }
}

fn strip_sigil(name: &[u8]) -> &[u8] {
    let name = name.strip_prefix(b"**").unwrap_or(name);
    let name = name.strip_prefix(b"*").unwrap_or(name);
    name.strip_prefix(b"&").unwrap_or(name)
}

fn summarize(body: &[u8]) -> SourceText {
    let mut paragraph_end = body.len();
    let mut index = 0;

    while index < body.len() {
        let line_end = body[index..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(body.len(), |offset| index + offset);
        let line = &body[index..line_end];
        if line.trim_ascii().is_empty() {
            paragraph_end = index;
            break;
        }
        index = line_end + 1;
    }

    let paragraph = trim_end(trim_start(&body[..paragraph_end]));
    for (position, byte) in paragraph.iter().enumerate() {
        if *byte == b'.'
            && paragraph
                .get(position + 1)
                .is_none_or(u8::is_ascii_whitespace)
        {
            return SourceText::from_bytes(&paragraph[..=position]);
        }
    }

    SourceText::from_bytes(paragraph)
}

fn dedent(raw: &[u8]) -> Vec<u8> {
    let lines = split_lines(raw);
    let indent = lines
        .iter()
        .filter(|line| !line.trim_ascii().is_empty())
        .map(|line| line.iter().take_while(|byte| **byte == b' ').count())
        .min()
        .unwrap_or(0);

    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            result.push(b'\n');
        }
        result.extend_from_slice(&line[indent.min(line.len())..]);
    }
    result
}

fn split_lines(raw: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = raw.split(|byte| *byte == b'\n').collect();
    if raw.ends_with(b"\n") && lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

fn join_lines(lines: &[&[u8]]) -> Vec<u8> {
    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            result.push(b'\n');
        }
        result.extend_from_slice(line);
    }
    result
}

fn first_line(bytes: &[u8]) -> &[u8] {
    bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default()
}

fn trim_bytes(bytes: &[u8]) -> &[u8] {
    trim_end(trim_start(bytes))
}

fn indent(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count()
}

fn trim_horizontal_start(mut bytes: &[u8]) -> &[u8] {
    while let Some((first, rest)) = bytes.split_first() {
        if matches!(first, b' ' | b'\t') {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn trim_start(mut bytes: &[u8]) -> &[u8] {
    while let Some((first, rest)) = bytes.split_first() {
        if first.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

fn trim_end(mut bytes: &[u8]) -> &[u8] {
    while let Some((last, rest)) = bytes.split_last() {
        if last.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}
