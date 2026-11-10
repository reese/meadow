//! Cross-reference link expansion for docstrings.

use meadow_model::{ObjectId, Registry, SourceText};

use crate::resolve_name;

/// Supplies output URLs and file contents to the link expander.
pub trait LinkTargetResolver {
    /// Returns the URL for a resolved object.
    fn object_url(&self, registry: &Registry, id: ObjectId) -> String;

    /// Returns the URL for an extra file.
    fn file_url(&self, path: &str) -> String;

    /// Reads an extra file, returning raw bytes.
    fn read_file(&self, path: &str) -> Option<Vec<u8>>;

    /// Returns a URL for an externally known proxy name.
    fn proxy_url(&self, _name: &str) -> Option<String> {
        None
    }
}

/// The result of expanding links in a docstring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedDocstring {
    /// Text with links rewritten as Markdown links and includes inlined.
    pub text: SourceText,
    /// Warnings for unresolved links and missing files.
    pub warnings: Vec<String>,
}

const MAX_INCLUDE_DEPTH: usize = 8;

/// Resolves an external proxy URL for an unresolved name.
///
/// Exact proxy entries always win. When no exact entry exists, a constant name
/// such as `File::SEPARATOR` is linked through its parent namespace entry
/// (`File.html#SEPARATOR`). Top-level constants such as `STDOUT` are linked
/// through an `Object` entry (`Object.html#STDOUT`).
#[must_use]
pub fn proxy_url_for(resolver: &dyn LinkTargetResolver, name: &str) -> Option<String> {
    if let Some(url) = resolver.proxy_url(name) {
        return Some(url);
    }

    let name = name.strip_prefix("::").unwrap_or(name);
    if name.is_empty() {
        return None;
    }
    if let Some(url) = resolver.proxy_url(name) {
        return Some(url);
    }

    if let Some((parent, constant)) = name.rsplit_once("::") {
        if is_constant_name(constant) {
            let parent_url = resolver.proxy_url(parent)?;
            return anchored_url(&parent_url, constant);
        }
        return None;
    }

    if is_constant_name(name) {
        let object_url = resolver.proxy_url("Object")?;
        return anchored_url(&object_url, name);
    }

    None
}

fn is_constant_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && characters.all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn anchored_url(base: &str, anchor: &str) -> Option<String> {
    if base.contains('#') {
        None
    } else {
        Some(format!("{base}#{anchor}"))
    }
}

/// Expands YARD link syntax outside code spans and fenced code blocks.
#[must_use]
pub fn expand_links(
    text: &SourceText,
    namespace: ObjectId,
    registry: &Registry,
    resolver: &dyn LinkTargetResolver,
) -> ExpandedDocstring {
    let mut warnings = Vec::new();
    let expanded = expand_bytes(
        text.as_bytes(),
        namespace,
        registry,
        resolver,
        0,
        &mut warnings,
    );
    ExpandedDocstring {
        text: SourceText::from_vec(expanded),
        warnings,
    }
}

fn expand_bytes(
    input: &[u8],
    namespace: ObjectId,
    registry: &Registry,
    resolver: &dyn LinkTargetResolver,
    depth: usize,
    warnings: &mut Vec<String>,
) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut position = 0;
    let mut line_start = true;
    let mut in_fence = false;
    let mut in_inline_code = false;

    while position < input.len() {
        if line_start && input[position..].starts_with(b"```") {
            in_fence = !in_fence;
            line_start = false;
            output.extend_from_slice(b"```");
            position += 3;
            continue;
        }

        let byte = input[position];
        if !in_fence && byte == b'`' {
            in_inline_code = !in_inline_code;
            output.push(byte);
            position += 1;
            line_start = false;
            continue;
        }

        if !in_fence
            && !in_inline_code
            && byte == b'{'
            && let Some(end) = input[position + 1..].iter().position(|byte| *byte == b'}')
        {
            let end = position + 1 + end;
            let content = trim_ascii(&input[position + 1..end]);
            expand_link(
                content,
                namespace,
                registry,
                resolver,
                depth,
                warnings,
                &mut output,
            );
            position = end + 1;
            line_start = false;
            continue;
        }

        output.push(byte);
        line_start = byte == b'\n';
        position += 1;
    }

    output
}

fn expand_link(
    content: &[u8],
    namespace: ObjectId,
    registry: &Registry,
    resolver: &dyn LinkTargetResolver,
    depth: usize,
    warnings: &mut Vec<String>,
    output: &mut Vec<u8>,
) {
    if let Some(path) = content.strip_prefix(b"include:file:") {
        include_file(trim_ascii(path), resolver, warnings, output);
        return;
    }
    if let Some(target) = content.strip_prefix(b"include:") {
        include_object(
            trim_ascii(target),
            namespace,
            registry,
            resolver,
            depth,
            warnings,
            output,
        );
        return;
    }
    if let Some(path) = content.strip_prefix(b"render:") {
        include_file(trim_ascii(path), resolver, warnings, output);
        return;
    }
    if let Some(rest) = content.strip_prefix(b"file:") {
        let (target, label) = split_target_label(rest);
        let path = String::from_utf8_lossy(target).into_owned();
        let default_label = path.split('#').next().unwrap_or(&path);
        let label = if label.is_empty() {
            default_label.as_bytes()
        } else {
            label
        };
        write_markdown_link(output, label, resolver.file_url(&path).as_bytes());
        return;
    }

    let (target, label) = split_target_label(content);
    let target_text = String::from_utf8_lossy(target).into_owned();

    if is_url(target) {
        let label = if label.is_empty() { target } else { label };
        write_markdown_link(output, label, target);
        return;
    }

    let name = SourceText::from_bytes(target);
    if let Some(id) = resolve_name(registry, &name, Some(namespace)) {
        let url = resolver.object_url(registry, id);
        let label = if label.is_empty() { target } else { label };
        write_markdown_link(output, label, url.as_bytes());
    } else if let Some(url) = proxy_url_for(resolver, &target_text) {
        let label = if label.is_empty() { target } else { label };
        write_markdown_link(output, label, url.as_bytes());
    } else {
        warnings.push(format!(
            "Cannot resolve link to {} from text: {{{}}}",
            target_text,
            String::from_utf8_lossy(content)
        ));
        let label = if label.is_empty() { target } else { label };
        output.extend_from_slice(label);
    }
}

fn include_file(
    path: &[u8],
    resolver: &dyn LinkTargetResolver,
    warnings: &mut Vec<String>,
    output: &mut Vec<u8>,
) {
    let path = String::from_utf8_lossy(path).into_owned();
    if let Some(contents) = resolver.read_file(&path) {
        output.extend_from_slice(&contents);
    } else {
        warnings.push(format!("Cannot include file {path}"));
    }
}

fn include_object(
    target: &[u8],
    namespace: ObjectId,
    registry: &Registry,
    resolver: &dyn LinkTargetResolver,
    depth: usize,
    warnings: &mut Vec<String>,
    output: &mut Vec<u8>,
) {
    if depth >= MAX_INCLUDE_DEPTH {
        warnings.push("Maximum docstring include depth exceeded".to_string());
        return;
    }

    let name = SourceText::from_bytes(target);
    let Some(id) = resolve_name(registry, &name, Some(namespace)) else {
        warnings.push(format!(
            "Cannot include {}",
            String::from_utf8_lossy(target)
        ));
        return;
    };
    let docstring = &registry.get(id).docstring.text;
    output.extend_from_slice(&expand_bytes(
        docstring.as_bytes(),
        id,
        registry,
        resolver,
        depth + 1,
        warnings,
    ));
}

fn split_target_label(content: &[u8]) -> (&[u8], &[u8]) {
    let trimmed = trim_ascii(content);
    match trimmed.iter().position(u8::is_ascii_whitespace) {
        Some(position) => (&trimmed[..position], trim_ascii(&trimmed[position + 1..])),
        None => (trimmed, &[]),
    }
}

fn is_url(target: &[u8]) -> bool {
    target.starts_with(b"http://")
        || target.starts_with(b"https://")
        || target.starts_with(b"ftp://")
}

fn write_markdown_link(output: &mut Vec<u8>, label: &[u8], url: &[u8]) {
    output.push(b'[');
    output.extend_from_slice(label);
    output.extend_from_slice(b"](");
    output.extend_from_slice(url);
    output.push(b')');
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while let Some((first, rest)) = bytes.split_first() {
        if first.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    while let Some((last, rest)) = bytes.split_last() {
        if last.is_ascii_whitespace() {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}
